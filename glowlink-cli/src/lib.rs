//! 萤光 GlowLink CLI 表现层。
//!
//! 拆成库是为了让参数解析、配置加载、终端视图可以被独立测试；
//! 可执行入口只做装配与依赖注入。

pub mod args;
pub mod config;
pub mod daemon;
pub mod ipc;
pub mod keys;
pub mod pending;
pub mod state;
pub mod view;

pub use view::daemon::DaemonView;
pub use view::CliView;
