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

/// 设备类型 → 槽位：预览与真机共用同一套「第几台用什么图标」的约定。
///
/// 真机数据来自 `controller::DeviceSource`，这里只是预览/离线渲染的兜底，
/// 保证「第 index 台设备画成什么样」这件事只有一处定义。
pub fn preview_kind(index: usize) -> glowlink_core::DeviceKind {
    use glowlink_core::DeviceKind;
    match index {
        0 => DeviceKind::Phone,
        1 => DeviceKind::Tablet,
        _ => DeviceKind::Desktop,
    }
}
