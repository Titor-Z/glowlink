//! View：整窗装配 —— 320×500 玻璃窗。
//!
//! 窗口本身是**标准窗口 + 隐藏标题栏**（不是无边框窗口）：
//! macOS 与 Win11 会自带圆角与系统阴影，正好是设计稿的样子；
//! 自己造无边框窗口就得连圆角、阴影、拖拽一起自己实现，而且平台差异一堆。
//! 玻璃走 `WindowBackgroundAppearance::Blurred`（macOS = NSVisualEffectView，
//! Windows = DWM 组合属性），窗内再叠 `backdrop_filter` 做卡片玻璃。

use gpui::{
    div, prelude::*, px, AnimationExt, App, Context, DragMoveEvent, ElementId, IntoElement,
    MouseButton,
    ParentElement, Render, SharedString, Styled, Window, WindowBackgroundAppearance, WindowBounds,
    WindowOptions,
};

use std::sync::Arc;
use std::time::Duration;

use crate::controller::{DeviceSource, DeviceView, IpcSource, PairingSource};
use crate::model::drag::DeviceDrag;
use crate::model::layout::{self, NodeSize};
use crate::model::state::{Phase, UiState};
use crate::theme::{self, Theme};
use crate::model::drop::DropState;
use crate::view::{dropzone, node, paircard, titlebar};

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
    /// 设备在环带上的位置（与 `devices` 同序）。
    ///
    /// 只在**设备集合变化**时重算：每帧重算会让所有节点跟着抖，那就不叫漂浮了。
    placement: layout::Placement,
    /// 上次布局用的设备 ID，用来判断「集合是否真变了」。
    placed_ids: Vec<String>,
    /// 底部拖拽区的状态（拖入/拒收由 `FileDropEvent` 驱动，见下一步）。
    drop: DropState,
    /// 已选中的目标设备名（点设备节点选中；一台设备时不必选）。
    selected: Option<String>,
    /// 配对能力（预览模式下没有）。
    pairing: Option<Arc<dyn PairingSource>>,
    /// 正在被拖动的那台设备 ID（只用于压暗它自己）。
    dragging: Option<String>,
    /// 拖动中指针的**全窗坐标**（落点是否在中心盘里由它算，不另存布尔值）。
    /// 松手落在别处时由根节点的 mouse_up 清掉。
    drag_at: Option<layout::Point>,
    /// 本机发起、正在等的配对（编号 + 对端 + 码 + 状态）。
    pair: Option<PairingCard>,
    /// 松手没落到中心盘的那台：从哪个位置飞回原位。`seq` 让每次回弹都重播动画。
    returning: Option<Returning>,
    /// 回弹序号（只增不减）：动画 id 靠它区分「这一次」和「上一次」。
    return_seq: u64,
}

/// 一次「飞回原位」的记录。
#[derive(Clone, Debug, PartialEq)]
struct Returning {
    id: String,
    from: layout::Point,
    seq: u64,
}

/// 界面要显示的配对卡内容。
#[derive(Clone, Debug, PartialEq)]
struct PairingCard {
    token: String,
    peer: String,
    code: String,
    state: glowlink_ipc::row::PairingState,
}

impl AppRoot {
    pub fn new(
        device_name: impl Into<SharedString>,
        source: Arc<dyn DeviceSource>,
        service: Option<Arc<IpcSource>>,
        pairing: Option<Arc<dyn PairingSource>>,
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
        let (placement, placed_ids) = Self::layout_for(&devices);
        AppRoot {
            device_name: device_name.into(),
            state,
            source,
            service,
            service_on,
            devices,
            placement,
            placed_ids,
            drop: DropState::Idle,
            selected: None,
            pairing,
            dragging: None,
            drag_at: None,
            pair: None,
            returning: None,
            return_seq: 0,
        }
    }

    /// 正在被拖的那台设备的辉光色（没有拖动时 `None`）。
    fn dragged_tone(&self) -> Option<gpui::Rgba> {
        let id = self.dragging.as_deref()?;
        self.devices
            .iter()
            .find(|device| device.id == id)
            .map(|device| theme::device_tone(device.kind))
    }

    /// 松手：没落到中心盘上就让它从落点飞回原位。
    ///
    /// 位置**永远回原位**，不留在拖到的地方：位置是由设备 ID 派生的
    /// （`layout::place_devices`），留在半路会与别人重叠、也让「哪个位置是哪台」
    /// 的记忆失效。飞回去而不是跳回去，是为了让人看清「它回到哪儿了」。
    fn settle_drag(&mut self) {
        if let (Some(id), Some(from)) = (self.dragging.take(), self.drag_at.take()) {
            self.return_seq += 1;
            self.returning = Some(Returning { id, from, seq: self.return_seq });
        }
        self.drag_at = None;
    }

    /// 拖动的落点是否在本机盘上（**全窗坐标**）。
    ///
    /// 派生值：由 `drag_at` 现算，不另存一个会跟它不一致的布尔量。
    fn over_center(&self) -> bool {
        self.drag_at.map(layout::hits_center).unwrap_or(false)
    }

    /// 把一台设备拖到了本机盘上：发起配对。
    ///
    /// 码要等守护进程握手完才有（下一轮快照），所以先亮一张只有名字的卡，
    /// 码到了再补上——这样用户立刻看到「有反应了」。
    fn start_pairing(&mut self, drag: &DeviceDrag) {
        let Some(pairing) = self.pairing.clone() else {
            log::info!("预览模式：不做配对");
            return;
        };
        match pairing.start(&drag.id) {
            Ok(()) => {
                log::info!("发起配对：{}（{}）", drag.name, drag.id);
                self.pair = Some(PairingCard {
                    token: String::new(),
                    peer: drag.name.clone(),
                    code: String::new(),
                    state: glowlink_ipc::row::PairingState::WaitingPeer,
                });
            }
            Err(e) => log::info!("配对发起失败：{e}"),
        }
    }

    /// 给一批设备算位置（纯几何，见 `layout::place_devices`）。
    fn layout_for(devices: &[DeviceView]) -> (layout::Placement, Vec<String>) {
        let ids: Vec<&str> = devices.iter().map(|d| d.id.as_str()).collect();
        (layout::place_devices(&ids), devices.iter().map(|d| d.id.clone()).collect())
    }

    /// 拉一次快照；**真变了才返回 `true`**（调用方据此决定要不要重绘）。
    ///
    /// 设备表很小，比一次比每次都 `notify()` 好：后者会让窗口每秒白重绘 3 次，
    /// 而重绘会打断正在跑的动画（呼吸圈的相位会被拖回去）。
    pub fn sync(&mut self) -> bool {
        let fresh = self.source.devices();
        let service_on = self.service.as_ref().map(|s| s.running()).unwrap_or(false);
        if fresh == self.devices && service_on == self.service_on && self.pair.is_none() {
            return false;
        }
        let mut changed = false;
        if fresh != self.devices {
            // 日志落 stderr：排查「界面没反应」时第一眼就知道设备表到底有没有变
            log::info!("设备表更新：{} 台", fresh.len());
            self.devices = fresh;
            changed = true;
            self.state.observe_devices(self.devices.len());
            // 位置只在集合变了才算：同一批设备换序（谁先应答不一定）不该挪位置
            let ids: Vec<String> = self.devices.iter().map(|d| d.id.clone()).collect();
            if ids != self.placed_ids {
                (self.placement, self.placed_ids) = Self::layout_for(&self.devices);
            }
        }
        // 配对卡跟着 `PAIRINGS` 走：码与状态都在那些行里，界面不自己猜
        if let (Some(card), Some(pairing)) = (self.pair.clone(), self.pairing.clone()) {
            let rows = pairing.outgoing();
            let latest = rows
                .iter()
                .filter(|row| row.peer == card.peer)
                .next_back()
                .cloned();
            match latest {
                Some(row) => {
                    let fresh = PairingCard {
                        token: row.token,
                        peer: row.peer,
                        code: row.code,
                        state: row.state,
                    };
                    if fresh != card {
                        log::info!("配对状态：{}", crate::model::drag::pair_state_note(fresh.state));
                        self.pair = Some(fresh);
                        changed = true;
                    }
                }
                // 槽位已经从守护进程的表里消失：收起卡片，不留一张看不出死活的浮层
                None if !card.token.is_empty() => {
                    self.pair = None;
                    changed = true;
                }
                None => {}
            }
        }
        if service_on != self.service_on {
            log::info!("服务开关：{}", if service_on { "开" } else { "关" });
            self.service_on = service_on;
            changed = true;
        }
        changed
    }
}

impl Render for AppRoot {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::DARK;
        // 动画回调要求 `'static`：捕获的是这个「按值的一份」，不是 `t` 的借用
        let t_theme = Theme::DARK;
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
                let center = self
                    .placement
                    .centers
                    .get(index)
                    .copied()
                    .unwrap_or_else(layout::self_center);
                let rect = layout::Rect::from_center(center, NodeSize::Device);
                let phase = self.placement.phases.get(index).copied().unwrap_or(0.0);
                let being_dragged = self.dragging.as_deref() == Some(device.id.as_str());
                let payload = DeviceDrag {
                    id: device.id.clone(),
                    name: device.name.clone(),
                    kind: device.kind,
                };
                let visual = node::node(
                    &t,
                    node::NodeSpec {
                        rect,
                        who: node::NodeWho::Device(device.kind),
                        label: &device.name,
                        look: node::NodeLook::device_default(),
                        scale: 1.0,
                    },
                )
                // 拖动：起手就记住是谁，并按设备 ID 建拖影（拖影自己跟着指针走）
                .on_drag(payload, |drag: &DeviceDrag, _pos, _window, cx| {
                    cx.new(|_| node::DragGhost { name: drag.name.clone(), kind: drag.kind })
                })
                // **这个回调在元素内外都会触发**（gpui 按拖动起点投递），
                // 所以指针位置能一直跟着更新——落点判定就靠它
                .on_drag_move(cx.listener(
                    |this, event: &DragMoveEvent<DeviceDrag>, _window, cx| {
                        let point = window_point(event.bounds.origin, event.event.position);
                        let dragging = Some(event.drag(cx).id.clone());
                        let changed = this.dragging != dragging || this.drag_at != Some(point);
                        if changed {
                            this.dragging = dragging;
                            this.drag_at = Some(point);
                            cx.notify();
                        }
                    },
                ));
                // 拖动中原件**淡出**（不是压暗留着——留着看着就像「没拖动」）
                let faded = visual.with_animation(
                    ElementId::Name(format!("fade-{}-{}", device.id, being_dragged).into()),
                    gpui::Animation::new(Duration::from_millis(crate::model::drag::FADE_MS)),
                    move |el, delta| el.opacity(if being_dragged { 1.0 - delta } else { delta }),
                );
                // 位置：平常漂浮；松手没命中时从**落点**飞回原位（不是啪地跳回去）。
                // 一次性动画，id 里带序号——序号不变的话第二次回弹不会重播。
                match self.returning.as_ref().filter(|ret| ret.id == device.id) {
                    Some(ret) => {
                        let from = ret.from;
                        node::positioned(
                            faded,
                            ElementId::Name(format!("return-{}-{}", device.id, ret.seq).into()),
                            gpui::Animation::new(Duration::from_millis(
                                crate::model::drag::RETURN_MS,
                            ))
                            .with_easing(gpui::ease_out_quint()),
                            move |delta| {
                                (
                                    from.x + (rect.x - from.x) * delta,
                                    from.y + (rect.y - from.y) * delta,
                                )
                            },
                        )
                        .into_any_element()
                    }
                    None => node::positioned(
                        faded,
                        node::drift_id(rect),
                        node::drift_animation(),
                        move |delta| node::drift_offset(rect, phase, delta),
                    )
                    .into_any_element(),
                }
            })
            .collect();

        let show_devices = self.state.phase().shows_devices();
        let self_rect = layout::Rect::from_center(layout::self_center(), NodeSize::SelfNode);

        div()
            .id("root")
            // 松手落在中心盘的**外面**时，drop 目标不会触发（它 `stop_propagation`），
            // 于是事件冒泡到这里——正好用来把拖动态清掉，否则高亮会一直挂着
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.drag_at.is_some() || this.dragging.is_some() {
                        this.settle_drag();
                        cx.notify();
                    }
                }),
            )
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
                    // 中心盘：命中时**染上被拖那台的辉光色**并加强（不是只换个环）。
                    // 过渡靠 `with_animation` 一次性按状态重播（id 里带命中与否）——
                    // gpui 没有 CSS transition，这是等价做法。
                    .child({
                        let highlighted = self.over_center();
                        let dragged_tone = self.dragged_tone();
                        let name = self.device_name.clone();
                        let rect = graph_space(self_rect);
                        div().absolute().left(px(0.0)).top(px(0.0)).with_animation(
                            ElementId::Name(
                                format!(
                                    "center-tint-{}-{}",
                                    highlighted,
                                    dragged_tone.map(|_| 1).unwrap_or(0)
                                )
                                .into(),
                            ),
                            gpui::Animation::new(Duration::from_millis(
                                crate::model::drag::CENTER_TINT_MS,
                            ))
                            .with_easing(gpui::ease_out_quint()),
                            move |el, delta| {
                                let t = if highlighted { delta } else { 1.0 - delta };
                                let tone = dragged_tone.unwrap_or(theme::NODE_SELF);
                                el.child(node::node(
                                    &t_theme,
                                    node::NodeSpec {
                                        rect,
                                        who: node::NodeWho::SelfNode,
                                        label: &name.to_string(),
                                        look: node::NodeLook::self_node_blended(t, tone),
                                        scale: 1.0,
                                    },
                                ))
                            },
                        )
                    })
                    // 中心盘的落点区：84px 的盘本身（不含呼吸光圈）。
                    // 设计稿那句「落在中心 84px 内才生效」就是它。
                    .child(
                        div()
                            .id("center-hit")
                            .absolute()
                            .left(px(self_rect.x - theme::CENTER_HIT / 2.0))
                            .top(px(self_rect.y - theme::CENTER_HIT / 2.0))
                            .w(px(theme::CENTER_HIT))
                            .h(px(theme::CENTER_HIT))
                            .rounded(px(theme::CENTER_HIT / 2.0))
                            .on_drop(cx.listener(
                                |this, drag: &DeviceDrag, _window, cx| {
                                    // 命中区是圆，而落点区是方：这里再用圆判定一次
                                    let inside =
                                        this.drag_at.map(layout::hits_center).unwrap_or(false);
                                    if inside {
                                        // 命中：位置本来就没动过，淡入回原位即可
                                        this.dragging = None;
                                        this.drag_at = None;
                                        this.start_pairing(drag);
                                    } else {
                                        this.settle_drag();
                                    }
                                    cx.notify();
                                },
                            )),
            )
            .when_some(self.pair.clone(), |el, card| {
                el.child(paircard::card(&t, &card.peer, &card.code, card.state))
            })
            )
            // 拖拽区：设计稿里它是窗内固定的一段（44px + 16 边距），始终在
            .child(dropzone::dropzone(&t, self.drop, self.selected.as_deref()))
            .child(titlebar::window_border(&t))
    }
}

/// 拖动事件里的指针位置 → **全窗坐标**。
///
/// `DragMoveEvent` 给的是「相对被拖元素」的位置，所以要加上那个元素的窗口坐标。
fn window_point(origin: gpui::Point<gpui::Pixels>, position: gpui::Point<gpui::Pixels>) -> layout::Point {
    layout::Point { x: origin.x.as_f32() + position.x.as_f32(), y: origin.y.as_f32() + position.y.as_f32() }
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
    pairing: Option<Arc<dyn PairingSource>>,
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
        let view = cx.new(|_| AppRoot::new(device_name, source, service, pairing));
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
