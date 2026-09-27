//! 桌面版：320×500 玻璃小窗。
//!
//! 分层（与 CLI 同一套 MVC 口径）：
//! - `model`：纯数据与算法（节点几何、状态机、数字格式化），**零 IO、零 GPUI**
//! - `controller`：界面要用的能力，全是 **trait**（接口），实现有预览版与 IPC 版
//! - `view`：GPUI 渲染，只读快照，不在渲染路径上做 IO
//! - `theme`：`DESIGN.md` token 在代码里的唯一落地处
//!
//! 本 crate 是**唯一**允许依赖 UI 框架的地方（core / ipc / cli 保持零 UI 依赖）。

pub mod assets;
pub mod controller;
pub mod model;
pub mod platform;
pub mod theme;
pub mod view;
