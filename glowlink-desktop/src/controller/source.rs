//! Controller：界面要用的三种能力接口。
//!
//! 全部返回 `Result` 或「快照」而不是活对象：界面只读快照，
//! 阻塞 IO 一律在实现内部的后台线程里（GPUI 的渲染路径绝不能阻塞）。

use glowlink_ipc::row::PairingState;
use glowlink_core::DeviceKind;
use glowlink_core::model::error::Result;

/// 一台设备在界面上的样子（**界面只需要这些**，不引 core 的 Device）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// 是否已配对（决定界面要不要请人核对校验码）。
    pub trusted: bool,
}

/// 待确认的接收请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingView {
    pub token: String,
    pub peer: String,
    pub name: String,
    pub size: u64,
    /// 6 位校验码：**无条件显示**（本机信任对方 ≠ 对方信任本机）。
    pub code: String,
}

/// 待决定的配对请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingView {
    pub token: String,
    pub peer: String,
    /// 本机校验码（供人核对对方屏幕上显示的那个）。
    pub code: String,
    /// 还剩几次输入机会。
    pub remaining: u8,
    /// 槽位状态（等对方 / 等本机用户 / 已配对 / 被拒 / 失败）。
    pub state: PairingState,
}

/// 传输进度的**速度**（界面自己算：协议里不带速度，因为它可以从字节与耗时推出来）。
///
/// 行本身直接用协议类型 [`glowlink_ipc::row::TransferRow`]：它就是「跨进程契约」，
/// 再包一层同字段的 View 只会多一处要同步的拷贝。
pub use glowlink_ipc::row::{TransferDirection, TransferRow, TransferState};

/// 设备表来源。
pub trait DeviceSource: Send + Sync {
    /// 当前已知设备（快照）。
    fn devices(&self) -> Vec<DeviceView>;
    /// 主动刷新一次；失败返回错误给界面显示，**不要 panic**。
    fn refresh(&self) -> Result<()>;
}

/// 配对来源。
pub trait PairingSource: Send + Sync {
    /// 本机发起配对。
    ///
    /// **不返回校验码**：守护进程要先连对端才拿得到码，慢起来好几秒，
    /// 界面不能在按钮回调里等它。码从 [`Self::outgoing`] 的快照里拿——
    /// `PAIRINGS` 的每一行本就带码，界面下一轮（≤300ms）就能显示出来。
    fn start(&self, device_id: &str) -> Result<()>;
    /// 本机发起、等对方输码的槽位（含码，供人核对）。
    fn outgoing(&self) -> Vec<PairingView>;
    /// 对端发起、等本机用户决定的请求。
    fn pending(&self) -> Vec<PairingView>;
    /// 允许并提交用户输入的码。
    fn approve(&self, token: &str, code: &str) -> Result<()>;
    /// 拒绝。
    fn reject(&self, token: &str) -> Result<()>;
}

/// 传输来源。
pub trait TransferSource: Send + Sync {
    /// 当前登记表里的传输（守护进程保留结束项一小段时间，界面据此显示完成态）。
    ///
    /// 速度**不在**这里：它由界面用相邻快照的字节差除以真实间隔算出来
    /// （`model::transfer::SpeedMeter`）——协议里塞速度就得让守护进程也维护一个时钟。
    fn transfers(&self) -> Vec<TransferRow>;
    /// 取消一次传输（桌面版的 ✕）。**只请求**：真正的收尾由守护进程的传输线程做。
    fn cancel(&self, token: &str) -> Result<()>;
    /// 接受 / 拒绝一个待确认的接收请求。
    fn decide(&self, token: &str, accept: bool) -> Result<()>;
    /// 当前待确认请求。
    fn pending(&self) -> Vec<PendingView>;
}
