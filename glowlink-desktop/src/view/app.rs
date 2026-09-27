//! View：整窗装配 —— 320×500 玻璃窗。
//!
//! 窗口本身是**标准窗口 + 隐藏标题栏**（不是无边框窗口）：
//! macOS 与 Win11 会自带圆角与系统阴影，正好是设计稿的样子；
//! 自己造无边框窗口就得连圆角、阴影、拖拽一起自己实现，而且平台差异一堆。
//! 玻璃走 `WindowBackgroundAppearance::Blurred`（macOS = NSVisualEffectView，
//! Windows = DWM 组合属性），窗内再叠 `backdrop_filter` 做卡片玻璃。

use gpui::{
    div, prelude::*, px, App, Context, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, WindowBackgroundAppearance, WindowBounds, WindowOptions,
};

use crate::model::layout::{self, NodeSize};
use crate::model::state::{Phase, UiState};
use crate::theme::{self, Theme};
use crate::view::{node, titlebar};

/// 应用根视图。**只读快照**：不持有业务状态，数据由 `controller` 提供。
pub struct AppRoot {
    device_name: SharedString,
    state: UiState,
    /// 设备名（按槽位顺序），由上层从 controller 取好再塞进来。
    devices: Vec<SharedString>,
}

impl AppRoot {
    pub fn new(device_name: impl Into<SharedString>) -> Self {
        AppRoot {
            device_name: device_name.into(),
            state: UiState::new(),
            devices: Vec::new(),
        }
    }

    /// 用一组设备刷新界面（上层取好快照再调，避免在渲染路径上 IO）。
    pub fn set_devices(&mut self, names: Vec<String>, cx: &mut Context<Self>) {
        self.devices = names.into_iter().map(SharedString::from).collect();
        self.state.set_devices(self.devices.len());
        cx.notify();
    }

    /// 搜索结束（进发现态）。
    pub fn finish_search(&mut self, cx: &mut Context<Self>) {
        self.state.finish_search();
        cx.notify();
    }
}

impl Render for AppRoot {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::DARK;
        let cx_handle = cx.entity().clone();

        // 标题栏右侧三个：隐私浮层 · 收进托盘 · 关闭。
        // 关闭是**自绘的窗口控制**（系统红绿灯已经藏掉，见 `crate::platform`）。
        let privacy = titlebar::icon_button(
            "privacy",
            &t,
            "icons/lock.svg",
            None,
            theme::IconHover::Neutral,
            move |_, _, cx| {
                let _ = cx_handle.update(cx, |_, cx| cx.notify());
            },
        );
        let tray = titlebar::icon_button(
            "tray",
            &t,
            "icons/tray.svg",
            None,
            theme::IconHover::Neutral,
            |_, _, _| {},
        );
        let close = titlebar::icon_button(
            "close",
            &t,
            "icons/close.svg",
            Some(gpui::WindowControlArea::Close),
            theme::IconHover::Close,
            // 关窗 + 退出进程：只 `remove_window` 的话 macOS 会留一个没有窗口的活进程
            // （Dock 里还在、`ps` 里还在），用户看到的就是「点了×没退出」。
            |_, _, cx| cx.quit(),
        );

        let devices: Vec<_> = (0..self.state.device_count())
            .map(|index| {
                let rect = layout::Rect::from_center(layout::device_center(index), NodeSize::Device);
                let label = self.devices[index].clone();
                node::node(
                    &t,
                    node::NodeSpec {
                        rect,
                        who: node::NodeWho::Device(crate::preview_kind(index)),
                        label: &label,
                        look: node::NodeLook::device_default(),
                        interactive: true,
                    },
                )
            })
            .collect();

        let show_devices = self.state.phase().shows_devices();
        let self_rect = layout::Rect::from_center(layout::self_center(), NodeSize::SelfNode);

        div()
            .flex()
            .flex_col()
            .w(px(theme::WINDOW_W))
            .h(px(theme::WINDOW_H))
            .rounded(px(theme::WINDOW_RADIUS))
            .overflow_hidden()
            .bg(t.window_bg)
            .child(
                div()
                    .flex_none()
                    .h(px(theme::TITLEBAR_H))
                    .px(px(16.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(titlebar::titlebar(&t))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(theme::GAP_ICON_BUTTON))
                            .child(privacy)
                            .child(tray)
                            .child(close),
                    ),
            )
            // 节点图：绝对定位区，子元素按**全窗坐标**摆放后再减去标题栏偏移
            .child(
                div()
                    .relative()
                    .flex_none()
                    .h(px(theme::GRAPH_H))
                    .w_full()
                    .when(show_devices, |el| el.children(devices))
                    .child(node::node(
                        &t,
                        node::NodeSpec {
                            rect: graph_space(self_rect),
                            who: node::NodeWho::SelfNode,
                            label: &self.device_name.to_string(),
                            look: node::NodeLook::self_node(),
                            interactive: false,
                        },
                    )),
            )
            .child(titlebar::window_border(&t))
    }
}

/// 全窗坐标 → 节点图内坐标（减掉标题栏高度）。
///
/// **只在这里做一次换算**：设计稿的圆席坐标是全窗的，而节点区是标题栏之下的
/// 一个盒子，两者的差就是 44px 标题栏。把换算收在一处，别处不再出现「+44」。
fn graph_space(rect: layout::Rect) -> layout::Rect {
    layout::Rect { y: rect.y - theme::TITLEBAR_H, ..rect }
}

/// 打开主窗口。
pub fn open(device_name: String, cx: &mut App) {
    let bounds = corner_bounds(cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_background: WindowBackgroundAppearance::Blurred,
        is_resizable: false,
        titlebar: Some(gpui::TitlebarOptions {
            // 窗口标题跟随产品名（系统标题栏虽然藏了，名字仍会出现在窗口列表 / Mission Control）
            title: Some(crate::theme::BRAND_NAME.into()),
            // 隐藏系统标题栏，让窗内的自绘标题栏成为唯一的一条
            appears_transparent: true,
            // 系统红绿灯由 `crate::platform::hide_system_window_buttons` **真正藏掉**
            // （不是靠 `traffic_light_position` 挪出窗外），所以这里不再设 position
            traffic_light_position: None,
        }),
        ..Default::default()
    };
    cx.open_window(options, move |window, cx| {
        crate::platform::hide_system_window_buttons(window);
        cx.new(|_| AppRoot::new(device_name))
    })
    .expect("无法打开窗口");
}

/// 默认落位：主屏**可见区**（不含 Dock / 任务栏）的右下角，留 24px 边距。
///
/// 必须用 `visible_bounds()` 而不是 `bounds()`：macOS 的 Dock 与 Windows 的任务栏
/// 都会把屏幕底边吃掉一块，「贴右下角」若按整屏算出来就会被它们盖住。
fn corner_bounds(cx: &App) -> gpui::Bounds<gpui::Pixels> {
    let size = gpui::size(px(theme::WINDOW_W), px(theme::WINDOW_H));
    let margin = px(theme::WINDOW_MARGIN);
    let Some(display) = cx.primary_display() else {
        return gpui::Bounds { origin: gpui::point(px(0.0), px(0.0)), size };
    };
    let area = display.visible_bounds();
    gpui::Bounds {
        origin: gpui::point(
            area.origin.x + area.size.width - size.width - margin,
            area.origin.y + area.size.height - size.height - margin,
        ),
        size,
    }
}

/// `Phase` 与界面可见性的一致性检查（供测试引）。
pub fn phase_shows_devices(phase: Phase) -> bool {
    phase.shows_devices()
}
