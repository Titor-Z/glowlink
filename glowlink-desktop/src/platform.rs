//! 平台层：让系统自己画的东西让位给我们的自绘标题栏。
//!
//! 现在只有一件事：**藏掉 macOS 的系统红绿灯**（☓ − ⤢）。
//!
//! 为什么不用 `TitlebarOptions::traffic_light_position` 把它们挪出窗外：那只是把按钮移到
//! 看不见的坐标——幽灵命中区还在、全屏切换时 gpui 会把它们搬回来（`restore_traffic_light`）、
//! AppKit 重建按钮时也可能复活，属于糊。要「自己画就得系统一个不留」，就得进制调 AppKit。
//!
//! 只 macOS 编译；其他平台是空函数（Windows 侧 gpui-ce 已经把整条非客户区去掉，
//! 本来就没有系统按钮可藏）。

use gpui::Window;

/// 藏掉系统标题栏按钮（macOS：关闭 / 最小化 / 缩放三个）。
///
/// 在**窗口创建回调里**调用；幂等，AppKit 若重建按钮再调一次即可。
pub fn hide_system_window_buttons(window: &Window) {
    imp::hide(window)
}

#[cfg(target_os = "macos")]
mod imp {
    use gpui::Window;
    use objc2_app_kit::{NSWindowButton, NSView};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub fn hide(window: &Window) {
        // 注意：`Window` 自己有个同名固有方法 `window_handle()`（返回 `AnyWindowHandle`），
        // 会遮蔽 trait 方法；要拿原生句柄必须显式走 `HasWindowHandle::window_handle`。
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };

        // SAFETY: `ns_view` 是本窗正在用的那个 NSView；窗口创建回调跑在主线程，
        // 而 AppKit 的视图/窗口查询必须在主线程。指针活到本函数结束（窗口不归我们释放）。
        let view: &NSView = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
        let Some(ns_window) = view.window() else {
            return;
        };

        for kind in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ] {
            if let Some(button) = ns_window.standardWindowButton(kind) {
                button.setHidden(true);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use gpui::Window;

    /// 其他平台没有系统标题栏按钮要藏。
    pub fn hide(_window: &Window) {}
}
