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
use crate::model::connect::{self, Connection};
use crate::model::drag::DeviceDrag;
use crate::model::layout::{self, NodeSize};
use crate::model::state::{Phase, UiState};
use crate::theme::{self, Theme};
use crate::model::drop::DropState;
use crate::view::{dropzone, node, paircard, titlebar, wire};

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
    /// 位置变化的一次性过渡（回弹、被撞开、连接重排都走它）。
    ///
    /// `seq` 让每一次都换一个动画 id → 每次都重播；播完（或超时）就丢掉，
    /// 交回给漂浮动画。
    moves: Vec<MoveAnim>,
    move_seq: u64,
    /// 上一次渲染时各设备的目标点（用来判断「位置变了没有」）。
    shown: Vec<(String, layout::Point)>,
    /// 上一次为拖动而重排布局时的指针位置（节流用）。
    last_relayout: Option<layout::Point>,
    /// 拖动时被**推开**的节点（按设备 ID 记当前落点）。
    contacts: Vec<(String, layout::Point)>,
    /// 染色层是否曾经出现过（没出现过就压根不渲染——否则首帧会闪一下）。
    tint_shown: bool,
    /// 每次「进入命中」递增：过渡动画的 id 靠它每次都换新，才会重播。
    tint_seq: u64,
    /// 上一次命中时的目标色：淡出时用它，免得颜色在淡出途中跳回品牌蓝。
    tint_tone: gpui::Rgba,
    /// 已「拖到一起」的设备（连接环上的方位）。
    connections: Vec<Connection>,
    /// 断开之后就地留下的落点（按设备 ID 记）。
    ///
    /// 是**种子**而不是钉子：它不回原位，但也不能压在中心盘或别人身上，
    /// 所以交给排斥迭代推到最近的合法位置。
    seeds: Vec<layout::Seed>,
}

/// 一次位置过渡：从 `from` 滑到 `to`。
#[derive(Clone, Debug)]
struct MoveAnim {
    id: String,
    from: layout::Point,
    to: layout::Point,
    seq: u64,
    started: std::time::Instant,
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
        let mut root = AppRoot {
            device_name: device_name.into(),
            state,
            source,
            service,
            service_on,
            devices: devices.clone(),
            placement: layout::Placement::default(),
            placed_ids: Vec::new(),
            drop: DropState::Idle,
            selected: None,
            pairing,
            dragging: None,
            drag_at: None,
            pair: None,
            moves: Vec::new(),
            move_seq: 0,
            shown: Vec::new(),
            last_relayout: None,
            contacts: Vec::new(),
            tint_shown: false,
            tint_seq: 0,
            tint_tone: theme::NODE_SELF,
            connections: Vec::new(),
            seeds: Vec::new(),
        };
        let (placement, placed_ids) = root.layout_for(&devices);
        root.placement = placement;
        root.placed_ids = placed_ids;
        root
    }

    /// 某台设备的辉光色（用于把中心盘染成它的色）。
    fn dragged_tone_of(&self, id: &str) -> Option<gpui::Rgba> {
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
            // 目标是它本来的落点（它从没真的移动过，所以这里就是「飞回去」）
            if let Some(to) = self.target_of(&id) {
                self.push_move(&id, from, to);
            }
        }
        self.drag_at = None;
    }

    /// 重算漂浮布局（拖动时把拖影当成移动的障碍物），并记下由此产生的位置过渡。
    ///
    /// 拖动时**每一小段路就重算一次**：别的气泡被顶开、让路，松手后再漂回去。
    /// 这一步必须在拖动回调里做——等 300ms 的轮询去算，手感就是「粘的」。
    fn refresh_floating(&mut self) {
        let (placement, placed_ids) = self.layout_for(&self.devices);
        self.placement = placement;
        self.placed_ids = placed_ids;
        self.apply_contacts();
        self.track_moves();
    }

    /// 拖动时：把**真正被压到**的节点推开（局部、连续，不重新求解全局布局）。
    ///
    /// 为什么不做全局重排：那是「求平衡点」，会让远处的节点也跟着挪，
    /// 表现就是「都没遇到就碰、而且碰还都碰」。接触响应只该发生在接触的那几枚上。
    fn apply_contacts(&mut self) {
        self.contacts.clear();
        let Some(ghost) = self.drag_at else {
            return;
        };
        // 拖影比原件大一点（抓起时放大），接触判定要按它的实际尺寸来
        let ghost_radius = layout::DEVICE_RADIUS * crate::model::drag::GHOST_SCALE;
        let resting: Vec<layout::Point> = self.placement.centers.clone();
        let pushed = layout::resolve_contacts(
            &resting,
            layout::DEVICE_RADIUS,
            &[(ghost, ghost_radius)],
        );
        for (index, device) in self.devices.iter().enumerate() {
            let (Some(rest), Some(now)) = (resting.get(index), pushed.get(index)) else {
                continue;
            };
            if layout::distance(*rest, *now) > 0.5 {
                self.contacts.push((device.id.clone(), *now));
            }
        }
    }

    /// 这台设备此刻被推开到了哪儿（`None` = 它没被碰到）。
    fn contact_point(&self, id: &str) -> Option<layout::Point> {
        self.contacts.iter().find(|(cid, _)| cid == id).map(|(_, p)| *p)
    }

    /// 拖动时这次重算要不要做（指针走了够远才算，免得每个像素都重排）。
    fn should_relayout(&self, point: layout::Point) -> bool {
        match self.last_relayout {
            Some(last) => layout::distance(last, point) >= 3.0,
            None => true,
        }
    }

    /// 比对「上一次渲染的目标点」，变了就记一笔过渡。
    ///
    /// 这是所有「自动位移」的统一入口：碰撞把它挤开、断开后归位、连接重排……
    /// 全都走同一条过渡动画，所以手感一致（不会一处弹开一处瞬移）。
    fn track_moves(&mut self) {
        // 先把目标点全算出来（`self` 只读），再统一记过渡（要改 `self`）
        let targets: Vec<(String, layout::Point)> = self
            .devices
            .iter()
            .filter_map(|device| Some((device.id.clone(), self.target_of(&device.id)?)))
            .collect();
        for (id, point) in &targets {
            if let Some((_, old)) = self.shown.iter().find(|(shown_id, _)| shown_id == id) {
                self.push_move(id, *old, *point);
            }
        }
        self.shown = targets;
    }

    /// 这台设备此刻该待的点（已连接 → 环上；否则散布算法的结果）。
    fn target_of(&self, id: &str) -> Option<layout::Point> {
        if let Some(point) = self.ring_point(id) {
            return Some(point);
        }
        let index = self.devices.iter().position(|d| d.id == id)?;
        Some(
            self.placement
                .centers
                .get(index)
                .copied()
                .unwrap_or_else(layout::self_center),
        )
    }

    /// 记一笔位置过渡（同一台设备的旧记录作废，免得抢坐标）。
    fn push_move(&mut self, id: &str, from: layout::Point, to: layout::Point) {
        if layout::distance(from, to) < 0.5 {
            return;
        }
        self.move_seq += 1;
        self.moves.retain(|m| m.id != id);
        self.moves.push(MoveAnim {
            id: id.to_string(),
            from,
            to,
            seq: self.move_seq,
            started: std::time::Instant::now(),
        });
    }

    /// 拖动的落点是否在本机盘上（**全窗坐标**）。
    ///
    /// 派生值：由 `drag_at` 现算，不另存一个会跟它不一致的布尔量。
    fn over_center(&self) -> bool {
        self.drag_at.map(layout::snaps_to_center).unwrap_or(false)
    }

    /// 拖到一起：登记一条连接（按松手方向摆到连接环上）。
    ///
    /// 已经连上的再拖一次 = 换方向（把用户记住的方位更新掉），不重复登记。
    fn connect(&mut self, drag: &DeviceDrag, at: layout::Point) {
        let angle = connect::angle_of(layout::self_center(), at);
        let mut angles: Vec<f32> = self
            .connections
            .iter()
            .filter(|c| c.id != drag.id)
            .map(|c| c.angle)
            .collect();
        angles.push(angle);
        // 分开到最小夹角：全都拖到同一侧时不能叠成一坨，但也不做「均匀重排」
        //（那会把用户记住的方向抹掉）
        let spread = connect::separate(&angles);
        let mut next: Vec<Connection> = Vec::new();
        for (index, existing) in self.connections.iter().enumerate() {
            if existing.id == drag.id {
                continue;
            }
            let slot = spread
                .iter()
                .position(|a| (*a - existing.angle).abs() < 0.001)
                .unwrap_or(index);
            next.push(Connection { id: existing.id.clone(), angle: spread[slot] });
        }
        let own = spread[spread.len().saturating_sub(1)];
        next.push(Connection { id: drag.id.clone(), angle: own });
        self.connections = next;
        // 它不再漂浮，钉在环上；之前「就地留下」的种子作废
        self.seeds.retain(|seed| seed.id != drag.id);
        log::info!("已连接：{}（{}）", drag.name, drag.id);
    }

    /// 断开连接：**位置留在原地**（不回到最初拖来的地方）。
    fn disconnect(&mut self, id: &str) {
        let Some(index) = self.connections.iter().position(|c| c.id == id) else {
            return;
        };
        let connection = self.connections.remove(index);
        let point = connect::point_at(layout::self_center(), connection.angle);
        // 位置就地留下（当种子，不再回原位），但它恢复全尺寸并重新参与碰撞——
        // 「留在原地」与「不合法就得让开」是两件事，后者交给排斥迭代
        self.seeds.retain(|seed| seed.id != id);
        self.seeds.push(layout::Seed { id: id.to_string(), point });
        log::info!("已断开：{id}（就地留下 {:.0}, {:.0}，恢复全尺寸）", point.x, point.y);
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
    ///
    /// 已连接的设备**不参与漂浮**（它钉在连接环上）；断开后保留在原地的那些也钉住。
    /// 两者都作为「钉子」交给放置算法，别人绕着它们排。
    fn layout_for(&self, devices: &[DeviceView]) -> (layout::Placement, Vec<String>) {
        let floating: Vec<&str> = devices
            .iter()
            .filter(|d| !self.is_connected(&d.id))
            .map(|d| d.id.as_str())
            .collect();
        let pinned = connect::pinned_points(&self.connections);
        (
            layout::place_devices(&floating, &pinned, &self.seeds),
            devices.iter().map(|d| d.id.clone()).collect(),
        )
    }

    /// 已连接的设备不参与漂浮：它钉在连接环上。
    fn is_connected(&self, id: &str) -> bool {
        self.connections.iter().any(|c| c.id == id)
    }

    /// 已连接设备在连接环上的落点（`None` = 它还在漂浮）。
    fn ring_point(&self, id: &str) -> Option<layout::Point> {
        let connection = self.connections.iter().find(|c| c.id == id)?;
        Some(connect::point_at(layout::self_center(), connection.angle))
    }

    /// 拉一次快照；**真变了才返回 `true`**（调用方据此决定要不要重绘）。
    ///
    /// 设备表很小，比一次比每次都 `notify()` 好：后者会让窗口每秒白重绘 3 次，
    /// 而重绘会打断正在跑的动画（呼吸圈的相位会被拖回去）。
    pub fn sync(&mut self) -> bool {
        // 过渡播完就丢掉，交回给漂浮动画（不丢的话节点会一直僵在那儿不漂）
        let before = self.moves.len();
        self.moves
            .retain(|m| m.started.elapsed() < Duration::from_millis(crate::model::drag::MOVE_MS + 80));
        let released = self.moves.len() != before;

        let fresh = self.source.devices();
        let service_on = self.service.as_ref().map(|s| s.running()).unwrap_or(false);
        if fresh == self.devices && service_on == self.service_on && self.pair.is_none() {
            return released;
        }
        let mut changed = released;
        if fresh != self.devices {
            // 日志落 stderr：排查「界面没反应」时第一眼就知道设备表到底有没有变
            log::info!("设备表更新：{} 台", fresh.len());
            self.devices = fresh;
            changed = true;
            self.state.observe_devices(self.devices.len());
            // 位置只在集合变了才算：同一批设备换序（谁先应答不一定）不该挪位置
            let ids: Vec<String> = self.devices.iter().map(|d| d.id.clone()).collect();
            if ids != self.placed_ids {
                self.refresh_floating();
            }
        }
        // 位置过渡：目标点变了就记一笔（被撞开、重排、断开归位都靠它——「轻轻弹开」的手感）
        self.track_moves();

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
                let connected = self.is_connected(&device.id);
                // 已连接的：位置钉在连接环上；其余（含断开后就地留下的）：散布算法给
                let contact = self.contact_point(&device.id);
                let center = self.ring_point(&device.id).unwrap_or_else(|| {
                    contact.unwrap_or_else(|| {
                        self.placement
                            .centers
                            .get(index)
                            .copied()
                            .unwrap_or_else(layout::self_center)
                    })
                });
                let size = if connected {
                    theme::NODE_DEVICE_SIZE * connect::CONNECTED_SCALE
                } else {
                    theme::NODE_DEVICE_SIZE
                };
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
                        // id 由设备 ID 派生（不是坐标）：位置变了动画也不重头开始
                        id: ElementId::Name(format!("node-{}", device.id).into()),
                        who: node::NodeWho::Device(device.kind),
                        label: &device.name,
                        // 已连接：保留高亮（环拉满 + 辉光加强 + 常驻呼吸）
                        look: if connected {
                            node::NodeLook::device_connected()
                        } else {
                            node::NodeLook::device_default()
                        },
                        scale: if connected { connect::CONNECTED_SCALE } else { 1.0 },
                    },
                )
                // 拖动：起手就记住是谁，并按设备 ID 建拖影（拖影自己跟着指针走）
                .on_drag(payload, |drag: &DeviceDrag, _pos, _window, cx| {
                    cx.new(|_| node::DragGhost { name: drag.name.clone(), kind: drag.kind })
                })
                // **这个回调在元素内外都会触发**（gpui 按拖动起点投递），
                // 所以指针位置能一直跟着更新——落点判定就靠它
                .on_drag_move(cx.listener(
                    |this, event: &DragMoveEvent<DeviceDrag>, window, cx| {
                        // **位置一律问窗口**（窗口坐标），不去拼「元素原点 + 相对位置」：
                        // 那个相对量到底相对谁，只有框架知道；拼错了就是「怎么拖都不亮」。
                        let point = window_point(window.mouse_position());
                        // 先看**上一帧的落点**在不在命中区：只有「刚进来」才换动画 id
                        let was_over = this.over_center();
                        let dragging = Some(event.drag(cx).id.clone());
                        let now_over = layout::snaps_to_center(point);
                        if now_over && !was_over {
                            this.tint_shown = true;
                            this.tint_seq += 1;
                            if let Some(tone) = this.dragged_tone_of(&event.drag(cx).id) {
                                this.tint_tone = tone;
                            }
                            // 进/出吸附区各记一行：出问题时这两行能直接说明「判定到底发生没有」
                            log::info!(
                                "进入吸附区（距中心 {:.0}px）",
                                layout::distance_to_center(point)
                            );
                        } else if !now_over && was_over {
                            log::info!("离开吸附区");
                        }
                        // 再次按住 = 打断过渡：否则那份「滑过去」的一次性动画
                        // 还在每帧摆位置，跟拖动抢同一个坐标
                        this.moves.retain(|m| m.id != event.drag(cx).id);
                        let changed = this.dragging != dragging || this.drag_at != Some(point);
                        if changed {
                            this.dragging = dragging;
                            this.drag_at = Some(point);
                            // 走够一小段就重排一次：被拖到的那枚把旁边的顶开
                            if this.should_relayout(point) {
                                this.last_relayout = Some(point);
                                this.refresh_floating();
                            }
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
                // 已连接：缩小的盘 + 断开按钮，位置钉在环上
                if connected {
                    let disconnect_id = device.id.clone();
                    let stack = div().relative().w(px(size)).h(px(size)).child(faded).child(
                        node::disconnect_button(
                            &t,
                            ElementId::Name(format!("unlink-{}", device.id).into()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.disconnect(&disconnect_id);
                            cx.notify();
                        })),
                    );
                    // 连上的那一刻也走「滑过去」：啪地跳进去会显得很硬
                    return match self.moves.iter().find(|m| m.id == device.id) {
                        Some(anim) => {
                            let (from, seq) = (anim.from, anim.seq);
                            node::positioned(
                                stack,
                                ElementId::Name(format!("move-{}-{}", device.id, seq).into()),
                                gpui::Animation::new(Duration::from_millis(
                                    crate::model::drag::MOVE_MS,
                                ))
                                .with_easing(gpui::ease_out_quint()),
                                move |delta| {
                                    (
                                        from.x + (center.x - from.x) * delta,
                                        from.y + (center.y - from.y) * delta,
                                    )
                                },
                            )
                            .into_any_element()
                        }
                        None => node::positioned(
                            stack,
                            ElementId::Name(format!("pinned-{}", device.id).into()),
                            gpui::Animation::new(Duration::from_millis(1)),
                            move |_| (center.x, center.y),
                        )
                        .into_any_element(),
                    };
                }
                // 位置：平常漂浮；松手没命中时从**落点**飞回原位（不是啪地跳回去）。
                // 一次性动画，id 里带序号——序号不变的话第二次回弹不会重播。
                match self.moves.iter().find(|m| m.id == device.id) {
                    Some(anim) => {
                        let (from, to, seq) = (anim.from, anim.to, anim.seq);
                        node::positioned(
                            faded,
                            ElementId::Name(format!("move-{}-{}", device.id, seq).into()),
                            gpui::Animation::new(Duration::from_millis(
                                crate::model::drag::MOVE_MS,
                            ))
                            .with_easing(gpui::ease_out_quint()),
                            move |delta| {
                                (
                                    from.x + (to.x - from.x) * delta,
                                    from.y + (to.y - from.y) * delta,
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
        // 连上之后：本机盘也跟着缩小（「把设备拉过来一起变小」）并保持高亮
        let connected_any = !self.connections.is_empty();
        let self_scale = if connected_any { connect::CONNECTED_SCALE } else { 1.0 };
        let self_rect = layout::Rect::from_center(layout::self_center(), NodeSize::SelfNode);

        // 连线（画在节点之下）：每台已连接设备一条，颜色跟着那台设备
        let segments: Vec<wire::Segment> = self
            .connections
            .iter()
            .filter_map(|connection| {
                let device = self.devices.iter().find(|d| d.id == connection.id)?;
                Some(wire::Segment {
                    end: connect::point_at(layout::self_center(), connection.angle),
                    tone: theme::device_tone(device.kind),
                })
            })
            .collect();

        div()
            .id("root")
            // 松手落在中心盘的**外面**时，drop 目标不会触发（它 `stop_propagation`），
            // 于是事件冒泡到这里——正好用来把拖动态清掉，否则高亮会一直挂着
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.drag_at.is_some() || this.dragging.is_some() {
                        this.settle_drag();
                        this.last_relayout = None;
                        this.refresh_floating();
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
                    // 连线在最底层（设计稿层级：连线 < 节点）
                    .when(!segments.is_empty(), |el| el.child(wire::wires(segments)))
                    .when(show_devices, |el| el.children(devices))
                    // 中心盘：**本机盘 + 一层「染色版」叠加**，染色层的不透明度做过渡。
                    //
                    // 为什么是叠两层而不是逐帧改色：`with_animation` 的回调只能改
                    // 它自己那个元素的样式，改不了子元素的颜色——而 `Fn` 回调又没法
                    // 把新的子元素塞进去。叠一层、调它的 opacity，就是 gpui 里的 crossfade。
                    .child({
                        let highlighted = self.over_center();
                        let name = self.device_name.clone();
                        let rect = graph_space(self_rect);
                        let container = div()
                            .absolute()
                            .left(px(rect.x))
                            .top(px(rect.y))
                            .child(node::node(
                                &t_theme,
                                node::NodeSpec {
                                    id: ElementId::Name("node-self".into()),
                                    who: node::NodeWho::SelfNode,
                                    label: &name.to_string(),
                                    look: if connected_any {
                                        node::NodeLook::self_node_blended(1.0, theme::NODE_SELF)
                                    } else {
                                        node::NodeLook::self_node()
                                    },
                                    scale: self_scale,
                                },
                            ));
                        let tinted = node::node(
                            &t_theme,
                            node::NodeSpec {
                                id: ElementId::Name("node-self-tinted".into()),
                                who: node::NodeWho::SelfNode,
                                label: &name.to_string(),
                                look: node::NodeLook::self_node_blended(1.0, self.tint_tone),
                                scale: self_scale,
                            },
                        );
                        // 动画 id **只用「命中与否 + 每次命中的序号」**：
                        // 掺进别的会变的东西（比如色是否为 None）就会在无关时刻重播一次——
                        // 而重播的起点是 `1.0 - 0 = 1.0`，于是「一拖动中心就亮」。
                        // 没出现过就**不渲染这一层**：否则首帧会按「关」的起点（满亮）
                        // 闪一下，等于什么都没拖就先亮一次
                        if !self.tint_shown {
                            container
                        } else {
                            container.child(tinted.with_animation(
                                ElementId::Name(
                                    format!("center-tint-{}-{}", highlighted, self.tint_seq)
                                        .into(),
                                ),
                                gpui::Animation::new(Duration::from_millis(
                                    crate::model::drag::CENTER_TINT_MS,
                                ))
                                .with_easing(gpui::ease_out_quint()),
                                // 命中：染色层淡入；离开：淡出回本机蓝
                                move |el, delta| {
                                    el.opacity(if highlighted { delta } else { 1.0 - delta })
                                },
                            ))
                        }
                    })
                    // 落点区：**得装下 r=50 的吸附圆**（84px 的方框装不下它的四角），
                    // 否则边缘上「明明该吸附」的位置会因为落点区没盖住而收不到松手事件。
                    // 真正的判定在下面用圆算一次（方框只负责「别漏事件」）。
                    .child(
                        div()
                            .id("center-hit")
                            .absolute()
                            .left(px(self_rect.x - crate::model::drag::SNAP_RADIUS))
                            .top(px(self_rect.y - crate::model::drag::SNAP_RADIUS))
                            .w(px(crate::model::drag::SNAP_RADIUS * 2.0))
                            .h(px(crate::model::drag::SNAP_RADIUS * 2.0))
                            .rounded(px(crate::model::drag::SNAP_RADIUS))
                            .on_drop(cx.listener(
                                |this, drag: &DeviceDrag, window, cx| {
                                    // 判定用**松手那一刻**的指针位置（任务书要求），
                                    // 落点区是方、判定是圆，所以这里必须再算一次
                                    let point = window_point(window.mouse_position());
                                    let inside = layout::snaps_to_center(point);
                                    log::info!(
                                        "松手判定：({:.0}, {:.0}) 距中心 {:.0}px → {}",
                                        point.x,
                                        point.y,
                                        layout::distance_to_center(point),
                                        if inside { "吸附" } else { "回弹" }
                                    );
                                    this.last_relayout = None;
                                    if inside {
                                        // 命中：登记连接（摆到连接环上）
                                        this.dragging = None;
                                        this.drag_at = None;
                                        this.connect(drag, point);
                                        this.refresh_floating();
                                        this.start_pairing(drag);
                                    } else {
                                        this.settle_drag();
                                        this.refresh_floating();
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

/// 指针位置（`Pixels`）→ 模型用的 `f32` 点。
///
/// 位置来源只有一个：`Window::mouse_position()`（**窗口坐标**）。
fn window_point(position: gpui::Point<gpui::Pixels>) -> layout::Point {
    layout::Point { x: position.x.as_f32(), y: position.y.as_f32() }
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
