//! 萤光 GlowLink CLI 表现层。
//!
//! 拆成库是为了让参数解析、配置加载、终端视图可以被独立测试；
//! 可执行入口只做装配与依赖注入。
//!
//! 本地 IPC 层（协议、状态文件、客户端/服务端骨架）在 `glowlink-ipc` 里，
//! 桌面版与 CLI 共用同一份实现。

pub mod args;
pub mod config;

pub mod exit;
pub mod keys;
pub mod pairing;
pub mod pending;
pub mod view;

pub use view::daemon::DaemonView;
pub use view::CliView;
