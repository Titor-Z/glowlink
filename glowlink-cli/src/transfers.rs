//! 传输登记表：守护进程侧「谁在传什么」的唯一真相。
//!
//! 界面（CLI / 桌面版）通过 IPC `TRANSFERS` 读它。为什么不让界面自己算进度：
//! 传输跑在**守护进程**里，界面窗口什么时候关都不该让进度丢；
//! 而且两处各算一份必然对不上（哪一份是「真的传了多少」没有答案）。
//!
//! 已结束的条目保留一小段时间再隐去：界面至少要看到一次「已完成」，
//! 否则进度条会在 99% 处直接消失。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glowlink_core::controller::transfer::TransferView;
use glowlink_ipc::row::{TransferDirection, TransferRow, TransferState};

/// 结束后的条目在表里再停留多久（够界面看到终态并做淡出）。
const KEEP_FINISHED: Duration = Duration::from_secs(30);

/// 一次传输的内部记录。
#[derive(Clone, Debug)]
struct Entry {
    token: String,
    direction: TransferDirection,
    name: String,
    peer: String,
    done: u64,
    total: u64,
    state: TransferState,
    note: String,
    finished_at: Option<Instant>,
}

/// 全部传输（守护进程里只有一份）。
#[derive(Debug, Default)]
pub struct Transfers {
    counter: AtomicU64,
    slots: Mutex<Vec<Arc<Mutex<Entry>>>>,
}

impl Transfers {
    pub fn new() -> Self {
        Transfers { counter: AtomicU64::new(0), slots: Mutex::new(Vec::new()) }
    }

    /// 登记一次传输，返回可上报进度的句柄。
    ///
    /// **先登记再开始传**：界面（或用户）要能立刻看到「正在传」，
    /// 而不是等第一块数据到位才冒出来。
    pub fn start(
        &self,
        direction: TransferDirection,
        name: impl Into<String>,
        peer: impl Into<String>,
        total: u64,
    ) -> Job {
        let token = format!("{:03}", self.counter.fetch_add(1, Ordering::SeqCst) + 1);
        let entry = Arc::new(Mutex::new(Entry {
            token,
            direction,
            name: name.into(),
            peer: peer.into(),
            done: 0,
            total,
            state: TransferState::Running,
            note: String::new(),
            finished_at: None,
        }));
        self.slots.lock().unwrap_or_else(|e| e.into_inner()).push(Arc::clone(&entry));
        Job { entry }
    }

    /// 当前该显示的条目（含刚结束的，已隐去的丢掉）。
    pub fn rows(&self) -> Vec<TransferRow> {
        let now = Instant::now();
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.retain(|slot| match slot.lock().unwrap_or_else(|e| e.into_inner()).finished_at {
            Some(at) => now.duration_since(at) < KEEP_FINISHED,
            None => true,
        });
        slots
            .iter()
            .map(|slot| {
                let entry = slot.lock().unwrap_or_else(|e| e.into_inner());
                TransferRow {
                    token: entry.token.clone(),
                    direction: entry.direction,
                    name: entry.name.clone(),
                    peer: entry.peer.clone(),
                    done: entry.done,
                    total: entry.total,
                    state: entry.state,
                    note: entry.note.clone(),
                }
            })
            .collect()
    }
}

/// 一次传输的进度句柄（可以被传输线程与视图共用）。
#[derive(Clone, Debug)]
pub struct Job {
    entry: Arc<Mutex<Entry>>,
}

impl Job {
    /// 更新进度。
    ///
    /// **结束后不再覆盖终态**：传输线程可能在收尾后还收到一次回调，
    /// 那时把「已完成」改回「进行中」会让界面上的卡片倒着走。
    pub fn update(&self, done: u64, total: u64) {
        let mut entry = self.entry.lock().unwrap_or_else(|e| e.into_inner());
        if entry.finished_at.is_some() {
            return;
        }
        entry.done = done;
        if total > 0 {
            entry.total = total;
        }
    }

    pub fn finish(&self) {
        let mut entry = self.entry.lock().unwrap_or_else(|e| e.into_inner());
        // 已经落了结论（被拒 / 失败）就不再改口：终态只有第一个算数
        if entry.finished_at.is_some() {
            return;
        }
        entry.done = entry.total;
        entry.state = TransferState::Done;
        entry.note.clear();
        entry.finished_at = Some(Instant::now());
    }

    /// 对方拒绝接收（不是错误，是一条明确的结果）。
    pub fn reject(&self, note: impl Into<String>) {
        self.end(TransferState::Rejected, note);
    }

    pub fn fail(&self, note: impl Into<String>) {
        self.end(TransferState::Failed, note);
    }

    fn end(&self, state: TransferState, note: impl Into<String>) {
        let mut entry = self.entry.lock().unwrap_or_else(|e| e.into_inner());
        if entry.finished_at.is_some() {
            return;
        }
        entry.state = state;
        entry.note = note.into();
        entry.finished_at = Some(Instant::now());
    }

    /// 编号（日志与界面都会用；界面靠它把卡片与后续指令对上）。
    pub fn token(&self) -> String {
        self.entry.lock().unwrap_or_else(|e| e.into_inner()).token.clone()
    }
}

/// 发送侧直接把它当 `TransferView` 用：进度一路记进登记表。
///
/// `confirm_receive` 不会被问（只有接收侧才问人）：真被问到就拒绝——
/// 发送路径上凭空冒出「要不要接收」说明接错了视图。
impl TransferView for Job {
    fn confirm_receive(&self, _peer: &str, _name: &str, _size: u64) -> bool {
        false
    }

    fn progress(&self, _label: &str, done: u64, total: u64) {
        self.update(done, total);
    }
}
