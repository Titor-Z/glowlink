//! Controller：**IPC 实现** —— 界面是守护进程的前端，自己不保管状态。
//!
//! 设备表、待确认、配对队列全部问守护进程要，界面不读 `~/.glowlink/` 下任何文件
//! （信任列表尤其不读：它归守护进程管，两处各读一份必然不一致）。
//!
//! ## 线程模型（这里是本模块唯一需要想清楚的地方）
//!
//! - **读**：一个独立轮询线程把三张表抓成 [`Snapshot`]，界面线程只读快照；
//!   版本号变了才重绘（否则每 300ms 白重绘一次）。
//! - **写**：指令进队列，由**同一个轮询线程**发出。界面绝不自己连 IPC——
//!   一次 `PAIR` 在守护进程那边要先连对端，慢起来就是好几秒，
//!   放在按钮回调里就是「点了没反应，窗口卡住」。详见 [`PairingSource::start`]。
//! - 为什么不用 gpui 的 executor：它是裸 pollster，阻塞在里面会把其他后台任务
//!   一起拖住（认知纠正里记过），所以轮询用 `std::thread`。
//!
//! 代价（诚实记录）：指令是**异步**的，`Ok(())` 只代表「已交给守护进程」，
//! 真正的成败靠下一轮快照（`error` 字段 + 表变化）反映到界面上。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use glowlink_core::model::error::{GlowError, Result};

use glowlink_ipc::client::IpcClient;
use glowlink_ipc::row::{DeviceRow, PairingRow, PendingRow};
use glowlink_ipc::state::read_state;

use crate::controller::source::{
    DeviceSource, DeviceView, PairingSource, PairingView, PendingView, ProgressView, TransferSource,
};

/// 轮询周期：界面 3 次/秒够用，再密只是让守护进程空转。
const POLL_INTERVAL: Duration = Duration::from_millis(300);

/// 三张表 + 最近一次抓取的结果。
#[derive(Clone, Debug, Default, PartialEq)]
struct Snapshot {
    devices: Vec<DeviceView>,
    pending: Vec<PendingView>,
    /// 本机发起、等对方输码的配对槽位（码要显示给人核对）。
    pairing_outgoing: Vec<PairingView>,
    /// 对端发起、等本机用户输码的配对请求。
    pairing_pending: Vec<PairingView>,
    /// 最近一次失败的原因；成功时 `None`。
    error: Option<String>,
}

/// 守护进程数据源。
pub struct IpcSource {
    shared: Arc<Mutex<Snapshot>>,
    /// 待发指令队列（由轮询线程取走）。
    outgoing: Arc<Mutex<Vec<String>>>,
    /// 快照版本号：界面靠它判断要不要重绘。
    version: Arc<AtomicU64>,
}

impl IpcSource {
    /// 连上守护进程并开始轮询；没在跑或 PING 不通时返回 `None`。
    ///
    /// **先 PING**：状态文件存在 != 守护进程活着（可能刚崩、端口被别人占了），
    /// 「能连上并 PING 通」才是判据（与 CLI 同一口径）。
    pub fn attach() -> Option<Arc<Self>> {
        let state = read_state().ok().flatten()?;
        IpcClient::connect(&state).ok()?.call_first("PING").ok()?;

        let source = Arc::new(IpcSource {
            shared: Arc::new(Mutex::new(Snapshot::default())),
            outgoing: Arc::new(Mutex::new(Vec::new())),
            version: Arc::new(AtomicU64::new(0)),
        });

        let shared = Arc::clone(&source.shared);
        let outgoing = Arc::clone(&source.outgoing);
        let version = Arc::clone(&source.version);
        thread::spawn(move || loop {
            // 先发指令再抓表：本轮的改动下一轮就能看到，界面感觉是「点完就动」
            for command in outgoing.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
                if let Err(e) = send(&command) {
                    let mut guard = shared.lock().unwrap_or_else(|e| e.into_inner());
                    guard.error = Some(e.to_string());
                    version.fetch_add(1, Ordering::SeqCst);
                }
            }
            let mut guard = shared.lock().unwrap_or_else(|e| e.into_inner());
            match fetch() {
                Ok(fresh) => {
                    if *guard != fresh {
                        *guard = fresh;
                        version.fetch_add(1, Ordering::SeqCst);
                    }
                }
                // 一次握手抖动不该让屏幕上的设备全消失：表保持不动，只更新错误
                Err(e) => {
                    let text = e.to_string();
                    if guard.error.as_deref() != Some(text.as_str()) {
                        guard.error = Some(text);
                        version.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
            drop(guard);
            thread::sleep(POLL_INTERVAL);
        });

        Some(source)
    }

    /// 快照版本号：变了就说明该重绘。
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    /// 最近一次抓取失败的原因（成功时为 `None`）。
    pub fn error(&self) -> Option<String> {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .error
            .clone()
    }

    /// 把一条指令交给轮询线程。
    fn submit(&self, command: String) {
        self.outgoing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(command);
    }
}

/// 抓一次三张表。
fn fetch() -> Result<Snapshot> {
    let mut client = IpcClient::connect(&current_state()?)?;
    let devices = client.call("DEVICES")?;
    let pending = client.call("PENDING")?;
    let pairing_outgoing = client.call("PAIRINGS")?;
    let pairing_pending = client.call("PAIRPENDING")?;
    Ok(Snapshot {
        devices: devices.iter().skip(1).filter_map(|l| DeviceRow::from_line(l)).map(device_view).collect(),
        pending: pending.iter().skip(1).filter_map(|l| PendingRow::from_line(l)).map(pending_view).collect(),
        pairing_outgoing: pairing_outgoing
            .iter()
            .skip(1)
            .filter_map(|l| PairingRow::from_line(l))
            .map(pairing_view)
            .collect(),
        pairing_pending: pairing_pending
            .iter()
            .skip(1)
            .filter_map(|l| PairingRow::from_line(l))
            .map(pairing_view)
            .collect(),
        error: None,
    })
}

/// 发一条指令（用一次性连接；IPC 是有状态的短交互，不值得养长连接）。
fn send(command: &str) -> Result<String> {
    let mut client = IpcClient::connect(&current_state()?)?;
    client.call_first(command)
}

fn current_state() -> Result<glowlink_ipc::state::State> {
    read_state()?.ok_or_else(|| GlowError::Missing("后台服务没在跑（先运行 glowlink on）".into()))
}

fn device_view(row: DeviceRow) -> DeviceView {
    DeviceView { id: row.id, name: row.name, kind: row.kind, trusted: row.trusted }
}

fn pending_view(row: PendingRow) -> PendingView {
    PendingView { token: row.token, peer: row.peer, name: row.name, size: row.size, code: row.code }
}

fn pairing_view(row: PairingRow) -> PairingView {
    PairingView { token: row.token, peer: row.peer, code: row.code, remaining: row.remaining }
}

impl DeviceSource for IpcSource {
    fn devices(&self) -> Vec<DeviceView> {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .devices
            .clone()
    }

    fn refresh(&self) -> Result<()> {
        // 轮询线程在跑；这里只把「最近一次失败」如实报出去，不另开一次连接
        match self.error() {
            Some(text) => Err(GlowError::Remote(text)),
            None => Ok(()),
        }
    }
}

impl PairingSource for IpcSource {
    fn start(&self, device_id: &str) -> Result<()> {
        // **不同步等结果**：守护进程收到 PAIR 后要先连对端，慢起来好几秒。
        // 码与状态从 `outgoing()` 的快照里拿（`PAIRINGS` 行本身就带码）。
        self.submit(format!("PAIR {device_id}"));
        Ok(())
    }

    fn outgoing(&self) -> Vec<PairingView> {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pairing_outgoing
            .clone()
    }

    fn pending(&self) -> Vec<PairingView> {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pairing_pending
            .clone()
    }

    fn approve(&self, token: &str, code: &str) -> Result<()> {
        self.submit(format!("PAIR_ACCEPT {token} {code}"));
        Ok(())
    }

    fn reject(&self, token: &str) -> Result<()> {
        self.submit(format!("PAIR_REJECT {token}"));
        Ok(())
    }
}

impl TransferSource for IpcSource {
    fn progress(&self) -> Option<ProgressView> {
        // 传输进度还没有 IPC 指令（守护进程侧也还没有把进度暴露出来），
        // 先如实返回「没有进度」，不编造一张静默停在中途的卡。
        None
    }

    fn decide(&self, token: &str, accept: bool) -> Result<()> {
        let verb = if accept { "ACCEPT" } else { "REJECT" };
        self.submit(format!("{verb} {token}"));
        Ok(())
    }

    fn pending(&self) -> Vec<PendingView> {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .clone()
    }
}
