//! Controller：**IPC 实现** —— 界面是守护进程的前端，自己不保管状态。
//!
//! 设备表、待确认、配对队列全部问守护进程要，界面不读 `~/.glowlink/` 下任何文件
//! （信任列表尤其不读：它归守护进程管，两处各读一份必然不一致）。
//!
//! ## 线程模型（这里是本模块唯一需要想清楚的地方）
//!
//! - **读**：一个独立轮询线程把三张表 + 服务开关状态抓成 [`Snapshot`]，
//!   界面线程只读快照；版本号变了才重绘（否则每 300ms 白重绘一次）。
//! - **写**：指令与「启停服务」请求都进队列，由**同一个轮询线程**执行。
//!   界面绝不自己连 IPC、也绝不自己 spawn 进程 —— 拉起守护进程要等它就绪（几百毫秒到几秒），
//!   放在按钮回调里就是「点了没反应，窗口卡住」。详见 [`IpcSource::start_service`]。
//! - 为什么不用 gpui 的 executor：它是裸 pollster，阻塞在里面会把其他后台任务一起拖住
//!   （认知纠正里记过），所以轮询用 `std::thread`。
//!
//! 代价（诚实记录）：写操作是**异步**的，`Ok(())` 只代表「已交给守护进程」，
//! 真正的成败靠下一轮快照（`error` + `running` + 表变化）反映到界面上。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use glowlink_core::model::error::{GlowError, Result};

use glowlink_ipc::client::IpcClient;
use glowlink_ipc::daemon;
use glowlink_ipc::row::{DeviceRow, PairingRow, PendingRow};
use glowlink_ipc::state::{read_state, state_dir};

use crate::controller::source::{
    DeviceSource, DeviceView, PairingSource, PairingView, PendingView, ProgressView, TransferSource,
};

/// 轮询周期：界面 3 次/秒够用，再密只是让守护进程空转。
const POLL_INTERVAL: Duration = Duration::from_millis(300);

/// 拉起守护进程的等待上限（与 CLI `on` 同一量级：冷启动要建 IPC 监听 + 绑发现端口）。
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

/// 停止守护进程的等待上限。
const STOP_TIMEOUT: Duration = Duration::from_secs(3);

/// 对服务生命周期的一次请求（在轮询线程里执行）。
enum ServiceRequest {
    Start,
    Stop,
}

/// 三张表 + 服务状态 + 最近一次抓取的结果。
#[derive(Clone, Debug, Default, PartialEq)]
struct Snapshot {
    /// 守护进程在跑吗（「能 PING 通」才算，不是「状态文件存在」）。
    running: bool,
    devices: Vec<DeviceView>,
    pending: Vec<PendingView>,
    /// 本机发起、等对方输码的配对槽位（码要显示给人核对）。
    pairing_outgoing: Vec<PairingView>,
    /// 对端发起、等本机用户输码的配对请求。
    pairing_pending: Vec<PairingView>,
    /// 最近一次失败的原因；成功时 `None`。
    error: Option<String>,
}

/// 守护进程数据源 + 服务开关。
///
/// **总是存在**（不再像早期那样「连不上就返回 `None`」）：开关要知道「现在到底跑没跑」，
/// 这个答案得每轮刷新，不能只在启动时问一次。
pub struct IpcSource {
    shared: Arc<Mutex<Snapshot>>,
    /// 待发指令队列（由轮询线程取走）。
    outgoing: Arc<Mutex<Vec<String>>>,
    /// 待执行的启停请求（同样由轮询线程执行）。
    service: Arc<Mutex<Vec<ServiceRequest>>>,
    /// 快照版本号：界面靠它判断要不要重绘。
    version: Arc<AtomicU64>,
}

impl IpcSource {
    /// 启动轮询线程（**不要求守护进程已在跑**）。
    pub fn spawn() -> Arc<Self> {
        let source = Arc::new(IpcSource {
            shared: Arc::new(Mutex::new(Snapshot::default())),
            outgoing: Arc::new(Mutex::new(Vec::new())),
            service: Arc::new(Mutex::new(Vec::new())),
            version: Arc::new(AtomicU64::new(0)),
        });

        let shared = Arc::clone(&source.shared);
        let outgoing = Arc::clone(&source.outgoing);
        let service = Arc::clone(&source.service);
        let version = Arc::clone(&source.version);
        // 独立线程而不是 gpui 的 executor：executor 是裸 pollster，
        // 阻塞在里面会把其他后台任务一起拖住（认知纠正里记过）。
        thread::spawn(move || loop {
            // 先处理启停与指令，再抓表：本轮的改动下一轮就能看到，界面感觉是「点完就动」
            for request in service.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
                if let Err(e) = run_service_request(request) {
                    fail(&shared, &version, e.to_string());
                }
            }
            for command in outgoing.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
                if let Err(e) = send(&command) {
                    fail(&shared, &version, e.to_string());
                }
            }
            let fresh = fetch();
            let mut guard = shared.lock().unwrap_or_else(|e| e.into_inner());
            match fresh {
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

        source
    }

    /// 快照版本号：变了就说明该重绘。
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    /// 守护进程现在在跑吗。
    pub fn running(&self) -> bool {
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).running
    }

    /// 最近一次抓取失败的原因（成功时为 `None`）。
    pub fn error(&self) -> Option<String> {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .error
            .clone()
    }

    /// 开服务。**异步**：真正拉起到就绪要几百毫秒到几秒，不能放在按钮回调里。
    pub fn start_service(&self) {
        self.service
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(ServiceRequest::Start);
    }

    /// 关服务（异步，理由同上）。
    pub fn stop_service(&self) {
        self.service
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(ServiceRequest::Stop);
    }

    /// 把一条指令交给轮询线程。
    fn submit(&self, command: String) {
        self.outgoing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(command);
    }
}

/// 记一笔失败：不动表，只把原因挂出来。
fn fail(shared: &Arc<Mutex<Snapshot>>, version: &Arc<AtomicU64>, text: String) {
    let mut guard = shared.lock().unwrap_or_else(|e| e.into_inner());
    if guard.error.as_deref() != Some(text.as_str()) {
        guard.error = Some(text);
        version.fetch_add(1, Ordering::SeqCst);
    }
}

/// 执行一条启停请求（守护进程本体的机制在 `glowlink-ipc::daemon`）。
fn run_service_request(request: ServiceRequest) -> Result<()> {
    match request {
        ServiceRequest::Start => {
            let state = daemon::ensure_running(&launch_args(), STARTUP_TIMEOUT)?;
            log::info!("服务已启动（PID {}，IPC {}）", state.pid, state.ipc_port);
        }
        ServiceRequest::Stop => match daemon::stop(STOP_TIMEOUT)? {
            Some(state) => log::info!("服务已停止（PID {}）", state.pid),
            None => log::info!("服务本来就没在跑"),
        },
    }
    Ok(())
}

/// 桌面版拉起守护进程时带的参数。
///
/// - `--yes`：对**已信任**设备免确认自动接收。桌面版现在还没有待确认入口，
///   不带这个开关的话，收到文件会卡在「等人确认」直到超时被拒。
///   （等 GUI 做出待确认列表再改回默认。）
/// - `--dir`：接收目录给**绝对路径** `~/.glowlink/received`。相对路径会跟着
///   「谁把它拉起来的、当时在哪个目录」跑，文件落在哪儿全凭运气。
fn launch_args() -> Vec<String> {
    vec![
        daemon::DAEMON_COMMAND.to_string(),
        "--yes".to_string(),
        "--dir".to_string(),
        state_dir().join("received").display().to_string(),
    ]
}

/// 抓一次快照。
///
/// 服务没在跑**不是错误**：那是「关着」这个正常状态，返回 `running: false` + 空表。
/// 只有「状态文件在、但连不上/答不上话」才算真失败（交给调用方保留上一份表）。
fn fetch() -> Result<Snapshot> {
    let Some(state) = read_state()? else {
        return Ok(Snapshot::default());
    };
    if daemon::running_state().is_none() {
        // 状态文件是残留（进程已死）：当作「关着」，不报错
        return Ok(Snapshot::default());
    }
    let mut client = IpcClient::connect(&state)?;
    let devices = client.call("DEVICES")?;
    let pending = client.call("PENDING")?;
    let pairing_outgoing = client.call("PAIRINGS")?;
    let pairing_pending = client.call("PAIRPENDING")?;
    Ok(Snapshot {
        running: true,
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
    let state = current_state()?;
    let mut client = IpcClient::connect(&state)?;
    client.call_first(command)
}

fn current_state() -> Result<glowlink_ipc::state::State> {
    read_state()?.ok_or_else(|| GlowError::Missing("后台服务没在跑（先 `glowlink on`）".into()))
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
