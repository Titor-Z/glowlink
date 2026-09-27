//! View：GPUI 渲染。
//!
//! 只读 Model 与 Controller 的快照，**不持有任何业务状态**，
//! 也不直接发起阻塞 IO（数据由上层取好再交给这里画）。

pub mod app;
pub mod node;
pub mod titlebar;
