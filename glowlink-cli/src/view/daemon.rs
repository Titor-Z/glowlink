//! 守护进程视图：把「收到文件先问人」搬到没有终端的分离进程里。
//!
//! 守护进程没有 stdin，问不了人，所以：
//! 1. `confirm_receive` 登记待确认请求，并在 [`PendingQueue`] 上阻塞等待；
//! 2. 用户用 `glowlink accept <编号>` / `glowlink reject <编号>` 经 IPC 落决定；
//! 3. 超时按拒绝处理——没人管的时候宁可什么都不收。

use std::sync::Arc;
use std::time::Duration;

use crate::pending::{PendingOffer, PendingQueue};
use crate::state::append_log;
use glowlink_core::controller::transfer::TransferView;

pub struct DaemonView {
    queue: Arc<PendingQueue>,
    timeout: Duration,
}

impl DaemonView {
    pub fn new(queue: Arc<PendingQueue>, timeout: Duration) -> Self {
        DaemonView { queue, timeout }
    }
}

impl TransferView for DaemonView {
    fn confirm_receive(&self, peer: &str, name: &str, size: u64) -> bool {
        // 每个请求各占一个槽位、各拿一个编号，互不影响：
        // 单槽会让第二个发送方静默挂在 backlog 里。
        let offer = PendingOffer { peer: peer.to_string(), name: name.to_string(), size };
        let accepted = self.queue.ask(offer, self.timeout, |token| {
            append_log(&format!(
                "收到来自 {peer} 的文件「{name}」（{size} 字节），编号 {token}，等待确认：\
                 glowlink accept {token} / glowlink reject {token}"
            ));
        });
        append_log(if accepted {
            "已确认接收，开始传输"
        } else {
            "未确认（被拒绝或超时），已放弃本次接收"
        });
        accepted
    }

    /// 守护进程没有终端，进度不往日志里刷：一次传输刷几十行会把有用信息淹掉。
    fn progress(&self, _label: &str, _done: u64, _total: u64) {}
}
