//! Controller：界面要用的三种能力接口。
//!
//! 全部返回 `Result` 或「快照」而不是活对象：界面只读快照，
//! 阻塞 IO 一律在实现内部的后台线程里（GPUI 的渲染路径绝不能阻塞）。

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
}

/// 传输进度快照。
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressView {
    pub name: String,
    pub total: u64,
    pub done: u64,
    /// 1 秒滑动窗口速度（不是全程平均：中途掉速时全程平均会骗人）。
    pub speed: f64,
    pub finished: bool,
}

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

/// 传输进度来源。
pub trait TransferSource: Send + Sync {
    /// 最近一次传输的进度；没有传输时返回 `None`。
    fn progress(&self) -> Option<ProgressView>;
    /// 接受 / 拒绝一个待确认的接收请求。
    fn decide(&self, token: &str, accept: bool) -> Result<()>;
    /// 当前待确认请求。
    fn pending(&self) -> Vec<PendingView>;
}
