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

use std::sync::Arc;
use std::time::Duration;

use crate::controller::{DeviceSource, DeviceView, IpcSource};
use crate::model::layout::{self, NodeSize};
use crate::model::state::{Phase, UiState};
use crate::theme::{self, Theme};
use crate::view::{node, titlebar};

/// 界面刷新周期。真正抓数据在 `IpcSource` 的后台线程里（每 300ms），
/// 这里只比内存快照——渲染路径里不做任何 IO。
const SYNC_INTERVAL: Duration = Duration::from_millis(300);

/// 应用根视图。**只读快照**：不持有业务状态，数据由 `controller` 提供。
pub struct AppRoot {
    device_name: SharedString,
    state: UiState,
    /// 设备表来源（已在跑的守护进程，或预览实现）。
    source: Arc<dyn DeviceSource>,
    /// 服务开关的来源；预览模式下是 `None`（那时不碰真实服务）。
    service: Option<Arc<IpcSource>>,
    /// 服务是否在跑（标题栏开关的显示状态）。
    service_on: bool,
    /// 当前显示的设备（快照）。
    devices: Vec<DeviceView>,
}

impl AppRoot {
    pub fn new(
        device_name: impl Into<SharedString>,
        source: Arc<dyn DeviceSource>,
        service: Option<Arc<IpcSource>>,
    ) -> Self {
        let devices = source.devices();
        let service_on = service.as_ref().map(|s| s.running()).unwrap_or(false);
        // 初值也记一笔：排查「界面是空的」时，先分清楚是「真没设备」还是「压根没连上」
        log::info!(
            "设备表初值：{} 台（服务{}）",
            devices.len(),
            if service_on { "在跑" } else { "没在跑" }
        );
        let mut state = UiState::new();
        state.observe_devices(devices.len());
        AppRoot { device_name: device_name.into(), state, source, service, service_on, devices }
    }

    /// 拉一次快照；**真变了才返回 `true`**（调用方据此决定要不要重绘）。
    ///
    /// 设备表很小，比一次比每次都 `notify()` 好：后者会让窗口每秒白重绘 3 次，
    /// 而重绘会打断正在跑的动画（呼吸圈的相位会被拖回去）。
    pub fn sync(&mut self) -> bool {
        let fresh = self.source.devices();
        let service_on = self.service.as_ref().map(|s| s.running()).unwrap_or(false);
        if fresh == self.devices && service_on == self.service_on {
            return false;
        }
        if fresh != self.devices {
            // 日志落 stderr：排查「界面没反应」时第一眼就知道设备表到底有没有变
            log::info!("设备表更新：{} 台", fresh.len());
            self.devices = fresh;
            self.state.observe_devices(self.devices.len());
        }
        if service_on != self.service_on {
            log::info!("服务开关：{}", if service_on { "开" } else { "关" });
            self.service_on = service_on;
        }
        true
    }
}

impl Render for AppRoot {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::DARK;
        let cx_handle = cx.entity().clone();

        // 服务开关（预览模式下不显示）：开 = 品牌蓝 + 滑块在右，关 = 次白墨色 + 滑块在左。
        // 两个信号（位置 + 颜色）同时给：只靠颜色在灰底上读不出来，只靠位置又不够快。
        let service_on = self.service_on;
        let supervisor = self.service.clone();
        let toggle = self.service.as_ref().map(|_| {
            titlebar::icon_button(
                "service",
                &t,
                if service_on { "icons/toggle-on.svg" } else { "icons/toggle-off.svg" },
                if service_on { theme::NODE_SELF } else { t.text_secondary },
                None,
                theme::IconHover::Neutral,
                move |_, _, _| {
                    // 状态在**点击时**读：闭包捕获的是“开/关”的旧值的话就会点反
                    let Some(source) = supervisor.clone() else { return };
                    if source.running() {
                        source.stop_service();
                    } else {
                        source.start_service();
                    }
                },
            )
        });

        // 标题栏右侧三个：隐私浮层 · 收进托盘 · 关闭。
        // 关闭是**自绘的窗口控制**（系统红绿灯已经藏掉，见 `crate::platform`）。
        let privacy = titlebar::icon_button(
            "privacy",
            &t,
            "icons/lock.svg",
            t.text_secondary,
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
            t.text_secondary,
            None,
            theme::IconHover::Neutral,
            |_, _, _| {},
        );
        let close = titlebar::icon_button(
            "close",
            &t,
            "icons/close.svg",
            t.text_secondary,
            Some(gpui::WindowControlArea::Close),
            theme::IconHover::Close,
            // 关窗 + 退出进程：只 `remove_window` 的话 macOS 会留一个没有窗口的活进程
            // （Dock 里还在、`ps` 里还在），用户看到的就是「点了×没退出」。
            |_, _, cx| cx.quit(),
        );

        let devices: Vec<_> = self
            .devices
            .iter()
            .enumerate()
            .map(|(index, device)| {
                let rect = layout::Rect::from_center(layout::device_center(index), NodeSize::Device);
                node::node(
                    &t,
                    node::NodeSpec {
                        rect,
                        who: node::NodeWho::Device(device.kind),
                        label: &device.name,
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
                            .children(toggle)
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
pub fn open(
    device_name: String,
    source: Arc<dyn DeviceSource>,
    service: Option<Arc<IpcSource>>,
    cx: &mut App,
) {
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
        let view = cx.new(|_| AppRoot::new(device_name, source, service));
        // 快照轮询：**比较**在界面线程（只读内存），真正的 IPC 在后台线程。
        // 窗口关了 `update` 会失败，循环自己退出（不必额外取消机制）。
        let weak = view.downgrade();
        // 用 async 闭包（rustc 1.85+ 的 `async move |cx| …`）：普通闭包返回的
        // future 会借用 `&mut AsyncApp`，而 `spawn` 要求 future 是 `'static`。
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(SYNC_INTERVAL).await;
                let alive = weak.update(cx, |root, cx| {
                    if root.sync() {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        view
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
