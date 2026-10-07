//! Controller：界面依赖的**接口**。
//!
//! 界面不直接连守护进程，而是对着这几个 trait 画：
//! - 没有守护进程时给一套「预览实现」，界面照样能起来（UI 阶段要能离线截图比对）；
//! - 有守护进程时给 `glowlink-ipc` 的客户端实现。
//!
//! 这与 TUI 的结论一致（**界面是守护进程的前端，自己不保管状态**），
//! 区别只在于把这个结论写成了接口而不是一句注释。

pub mod ipc;
pub mod preview;
pub mod source;

pub use ipc::IpcSource;
pub use preview::{PreviewPairing, PreviewSource, PreviewTransfer, PreviewTrusts};
pub use source::{
    DeviceSource, DeviceView, PairingSource, PairingView, PendingView, ReceiveDirSource,
    TransferDirection, TransferRow, TransferSource, TransferState, TrustedView, TrustSource,
};
