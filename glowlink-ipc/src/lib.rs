//! 本地 IPC 层：回环 TCP 上的行协议。
//!
//! 拆成本 crate 的理由：**协议两端要能各自独立使用**。
//! CLI（守护进程 + 命令行）与桌面版（`glowlink-desktop`）都只说这一套协议，
//! 而它们之间没有任何别的共同代码——协议与状态文件放在这里，两边引用同一份实现，
//! 不会出现「桌面版解析 IPC 的第二份代码」。
//!
//! 模块划分：
//! - [`state`]：`~/.glowlink/state.json`（CLI 找到守护进程的唯一入口）+ 守护进程日志
//! - [`row`]：`DEVICES` / `PENDING` 的**数据行**（跨进程契约）
//! - [`codec`]：错误种类名与文本编解码、`OK` 载荷、`RESOLVE` 应答解析
//! - [`client`]：客户端（连上、发指令、读完整响应）
//! - [`server`]：服务端骨架（绑定、按行处理、连接循环）
//!
//! 守护进程**自己的实现**（`IpcHost` 的落地：发现循环、待确认队列、传输）
//! 不在本 crate 里，仍然属于 CLI——这里只有协议与两端骨架。

pub mod client;
pub mod codec;
pub mod daemon;
pub mod row;
pub mod server;
pub mod state;

pub use codec::error_reply;
pub use row::{DeviceRow, PairingRow, PairingState, PendingRow};
pub use state::State;
