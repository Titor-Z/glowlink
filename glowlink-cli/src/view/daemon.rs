//! 守护进程视图：把「收到文件先问人」搬到没有终端的分离进程里。
//!
//! 守护进程没有 stdin，问不了人，所以：
//! 1. `confirm_receive` 登记待确认请求，并在 [`PendingSlot`] 上阻塞等待；
//! 2. 用户用 `glowlink accept` / `glowlink reject` 经 IPC 落决定；
//! 3. 超时按拒绝处理——没人管的时候宁可什么都不收。

use std::sync::Arc;
use std::time::Duration;

use glowlink_core::controller::transfer::TransferView;
use crate::daemon::{append_log, PendingOffer, PendingSlot};

pub struct DaemonView {
    slot: Arc<PendingSlot>,
    timeout: Duration,
}

impl DaemonView {
    pub fn new(slot: Arc<PendingSlot>, timeout: Duration) -> Self {
        DaemonView { slot, timeout }
    }
}

impl TransferView for DaemonView {
    fn confirm_receive(&self, peer: &str, name: &str, size: u64) -> bool {
        append_log(&format!(
            "收到来自 {peer} 的文件「{name}」（{size} 字节），等待确认：glowlink accept / reject"
        ));
        let accepted = self.slot.ask(
            PendingOffer { peer: peer.to_string(), name: name.to_string(), size },
            self.timeout,
        );
        append_log(if accepted {
            "用户确认接收，开始传输"
        } else {
            "未确认（被拒绝或超时），已放弃本次接收"
        });
        accepted
    }

    /// 守护进程没有终端，进度不往日志里刷：一次传输刷几十行会把有用信息淹掉。
    fn progress(&self, _label: &str, _done: u64, _total: u64) {}
}
