//! View：整窗装配 —— 320×500 玻璃窗。
//!
//! 窗口本身是**标准窗口 + 隐藏标题栏**（不是无边框窗口）：
//! macOS 与 Win11 会自带圆角与系统阴影，正好是设计稿的样子；
//! 自己造无边框窗口就得连圆角、阴影、拖拽一起自己实现，而且平台差异一堆。
//! 玻璃走 `WindowBackgroundAppearance::Blurred`（macOS = NSVisualEffectView，
//! Windows = DWM 组合属性），窗内再叠 `backdrop_filter` 做卡片玻璃。

use gpui::{
    div, prelude::*, px, App, Context, DragMoveEvent, ElementId, IntoElement, MouseButton,
    ParentElement, Render, SharedString, Styled, Window, WindowBackgroundAppearance, WindowBounds,
    WindowOptions,
};

use std::sync::Arc;
use std::time::Duration;

use crate::controller::{
    DeviceSource, DeviceView, IpcSource, PairingSource, TransferRow, TransferSource, TransferState,
};
use crate::model::connect::{self, Connection};
use crate::model::drag::{self, DeviceDrag};
use crate::model::layout;
use crate::model::motion::{Ghost, Motion};
use crate::model::spring::{Spring, ZETA_CRITICAL, ZETA_MOMENTUM};
use crate::model::transfer::{self, SpeedMeter};
use crate::model::state::{Phase, UiState};
use crate::theme::{self, Theme};
use crate::model::drop::DropState;
use crate::view::{dropzone, node, paircard, progress, titlebar};

/// 界面刷新周期。真正抓数据在 `IpcSource` 的后台线程里（每 300ms），
/// 这里只比内存快照——渲染路径里不做任何 IO。
const SYNC_INTERVAL: Duration = Duration::from_millis(300);

/// 物理步进的最快快周期（有那么一点点运动时用）。
const TICK_FAST: Duration = Duration::from_millis(16);
/// 只剩漂浮时用的慢周期：漂移只有 ±3px / 7s（≈0.85px/s），
/// 50ms 一帧等于每帧动 0.04px——看不出差别，但重绘量掉到三分之一。
///
/// 为什么要分档：实测「永远 16ms」在空闲态就吃掉 60% 的 CPU
/// （玻璃+阴影整窗重绘每帧都跑）。壁纸级的小动作用不着 60fps。
const TICK_SLOW: Duration = Duration::from_millis(50);

/// 传输中呼吸光圈一圈多久（设计稿：加快到 1.5s）。
const HALO_FAST_SECS: f32 = 1.5;

/// 进度条填充的响应时间（秒）。设计稿给的是 0.12s 过渡：
/// 守护进程的进度是 300ms 一跳的，不平滑就会一抽一抽。
const PROGRESS_RESPONSE: f32 = 0.12;

/// 中心盘染色**亮起**的响应时间（秒）：够快才像「跟手」，判定一变就亮。
const TINT_RISE: f32 = 0.08;
/// 熄灭慢一些：边界上抖一下不会变成闪烁。
const TINT_FALL: f32 = 0.25;

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
    /// 漂浮设备的位置（**按设备 ID 记**，不按下标）。
    ///
    /// 按下标会在「拖起来的那台被移出集合」时整列错位——别人的坑会挪到隔壁身上。
    /// 只在**设备集合变化**时重算：每帧重算会让所有节点跟着抖，那就不叫漂浮了。
    floating: Vec<(String, layout::Point, f32)>,
    /// 上次布局用的设备 ID，用来判断「集合是否真变了」。
    placed_ids: Vec<String>,
    /// 底部拖拽区的状态（拖入/拒收由 `FileDropEvent` 驱动，见下一步）。
    drop: DropState,
    /// 已选中的目标设备名（点设备节点选中；一台设备时不必选）。
    selected: Option<String>,
    /// 配对能力（预览模式下没有）。
    pairing: Option<Arc<dyn PairingSource>>,
    /// 传输能力（预览模式下是假数据）。
    transfers: Option<Arc<dyn TransferSource>>,
    /// 当前要显示的那条传输（`None` = 卡片不出现）。
    transfer: Option<TransferRow>,
    /// 速度计：**界面自己算**（协议里不带速度，见 `model::transfer`）。
    meter: SpeedMeter,
    /// 被用户关掉的传输卡（按编号记：守护进程还会保留那条记录一小段时间）。
    dismissed: Vec<String>,
    /// 被用户关掉的配对卡（同理）。
    dismissed_pairs: Vec<String>,
    /// 进度条填充的**平滑值**（设计稿要求 0.12s 过渡：IPC 的进度是跳着来的，
    /// 直接画会像抽动）。这里用同一个弹簧原语做，不用另一套机制。
    progress: Spring,
    /// 交替高亮的相位（0..1）：本机亮一下 → 对端亮一下，由界面时钟推进。
    pulse: f32,
    /// 最近一次算出的速度（字节/秒）：卡片与元信息行都用它。
    transfer_speed: f64,
    /// 正在被拖动的那台设备 ID（只用于压暗它自己）。
    dragging: Option<String>,
    /// 拖动中那枚气泡**当前的中心**（图区坐标）。
    ///
    /// 存的是气泡而不是指针：落点判定看的是「这枚东西有没有落到盘上」，
    /// 而且拖动要带**抓取偏移**（按下时手指在盘的哪个位置，之后一直保持）。
    /// 松手落在别处时由根节点的 mouse_up 清掉。
    drag_at: Option<layout::Point>,
    /// 抓取偏移 = 按下那一刻「气泡中心 − 指针」。拖动时 `气泡 = 指针 + 偏移`。
    grab_offset: layout::Point,
    /// 本机发起、正在等的配对（编号 + 对端 + 码 + 状态）。
    pair: Option<PairingCard>,
    /// 全场运动状态：意图 → 弹簧 → 接触约束 → 只读位置（见 `model::motion`）。
    ///
    /// 位置**只住在这里**。渲染层不再有「该用哪个位置」的 if，
    /// 也不再有「动画 id 里塞了什么状态」这类只在运行时才暴露的坑。
    motion: Motion,
    /// 上一次物理步进的时刻（算 dt；切走再切回来时 dt 会被弹簧内部夹住）。
    last_tick: std::time::Instant,
    /// 中心盘染色层的不透明度：0 = 没拖到中心，1 = 拖到中心。
    /// 也是一个弹簧（同一个原语），不再用「动画 id 带命中序号」那套。
    tint: Spring,
    /// 「现在算不算落在中心盘上」。**高亮与吸附共用这一个判断**
    /// （每帧由 `over_center()` 更新）：亮起是承诺，松手就必须吸上。
    snapping: bool,
    /// 正被拖动的那台的辉光色（染色用）。
    tint_tone: gpui::Rgba,
    /// 呼吸圈的相位（`0..1`）。**由我们的时钟推进**，不用 gpui 的 repeat 动画
    /// （那会让框架持续请求整窗重绘）。
    halo: f32,
    /// 品牌辉光的相位（`0..1`）：和呼吸一样由界面时钟推进。
    brand_phase: f32,
    /// **传输中**的呼吸相位：设计稿要求光圈加快到 1.5s（表示「正在跑」）。
    halo_fast: f32,
    /// 正在传数据的对端显示名（`None` = 没有传输）。线、节点、卡片三处共用它。
    flowing_peer: Option<String>,
    /// 已「拖到一起」的设备（连接环上的方位）。
    connections: Vec<Connection>,
    /// 断开之后就地留下的落点（按设备 ID 记）。
    ///
    /// 是**种子**而不是钉子：它不回原位，但也不能压在中心盘或别人身上，
    /// 所以交给排斥迭代推到最近的合法位置。
    seeds: Vec<layout::Seed>,
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
        transfers: Option<Arc<dyn TransferSource>>,
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
            floating: Vec::new(),
            placed_ids: Vec::new(),
            drop: DropState::Idle,
            selected: None,
            pairing,
            transfers,
            transfer: None,
            meter: SpeedMeter::default(),
            dismissed: Vec::new(),
            dismissed_pairs: Vec::new(),
            progress: Spring::new(0.0, ZETA_CRITICAL, PROGRESS_RESPONSE),
            pulse: 0.0,
            transfer_speed: 0.0,
            dragging: None,
            drag_at: None,
            grab_offset: layout::Point { x: 0.0, y: 0.0 },
            pair: None,
            motion: Motion::default(),
            last_tick: std::time::Instant::now(),
            tint: Spring::new(0.0, ZETA_CRITICAL, TINT_RISE),
            snapping: false,
            tint_tone: theme::NODE_SELF,
            halo: 0.0,
            brand_phase: 0.0,
            halo_fast: 0.0,
            flowing_peer: None,
            connections: Vec::new(),
            seeds: Vec::new(),
        };
        root.refresh_floating();
        root
    }

    /// 某台设备的辉光色（用于把中心盘染成它的色）。
    fn dragged_tone_of(&self, id: &str) -> Option<gpui::Rgba> {
        self.devices
            .iter()
            .find(|device| device.id == id)
            .map(|device| theme::device_tone(device.kind))
    }

    /// 松手：清掉拖动态，让弹簧把它从**当前位置**送回目标点。
    ///
    /// 速度不用手动交接：拖起来的那枚是运动学的，它的速度本来就是指针的速度
    /// （见 `Motion::step` 的 ②），松手后弹簧接着那个速度走——这就是「拖→动画
    /// 之间没有缝」。只把阻尼比调低一点，让带着动量松手有一点点回弹。
    fn settle_drag(&mut self) {
        if let Some(id) = self.dragging.take() {
            self.motion.set_zeta(&id, ZETA_MOMENTUM);
        }
        self.drag_at = None;
        self.refresh_floating();
    }

    /// 重算**意图**（谁想去哪）+ 每枚**当前**的半径，写进运动系统。
    ///
    /// 这里只产出意图。几何（接触、碰撞）一律由 [`Motion`] 按**当前位置与当前半径**
    /// 算——静置点是「想去的地方」，不是「它在的地方」；拿它当几何输入就会出现
    /// 「还没碰到就弹开」。
    ///
    /// 拖动中**不需要**每帧调它：静置布局跟指针位置无关（接触由运动系统负责），
    /// 所以拖动时它只在「拖起来/放下」这两个瞬间各跑一次——布局不会跟着指针抖。
    fn refresh_floating(&mut self) {
        self.floating = self.layout_for(&self.devices);
        // 本机盘也进物理世界（**钉死的不动体**）：它会推别人，别人推不动它。
        // 它一缩小/长大，周围的气泡就被挤开——不然恢复全尺寸时会盖住它们。
        let self_scale = if self.connections.is_empty() { 1.0 } else { connect::CONNECTED_SCALE };
        let mut wanted: Vec<(String, layout::Point, f32)> =
            vec![(layout::SELF_ID.to_string(), layout::self_center(), 0.0)];
        wanted.extend(self.devices.iter().map(|device| {
                // 拖起来的那枚不在漂浮表里（它不占坑）：沿用它**当前**的位置，
                // 这样 `sync` 不会因为「它不在墙上」而把它从运动系统里删掉
                let point = self
                    .target_of(&device.id)
                    .or_else(|| self.motion.position(&device.id))
                    .unwrap_or_else(layout::self_center);
                (device.id.clone(), point, self.phase_of(&device.id))
            }));
        self.motion.sync(&wanted);
        for (id, point, _) in &wanted {
            self.motion.set_target(id, *point);
            self.motion.set_drifting(id, false);
        }
        for device in &self.devices {
            let id = device.id.as_str();
            self.motion.set_radius(id, self.radius_of(id));
            let drifting = !self.is_connected(id) && self.dragging.as_ref() != Some(&device.id);
            self.motion.set_drifting(id, drifting);
        }
        self.motion.set_radius(layout::SELF_ID, layout::SELF_RADIUS * self_scale);
        self.motion.set_pinned(layout::SELF_ID, true);
        self.placed_ids = self.devices.iter().map(|d| d.id.clone()).collect();
    }

    /// 这台设备「想去哪」：已连接 → 连接环上；在漂浮表里 → 散布算法的静置点。
    ///
    /// 注意：它只是**意图**，不是几何。拖动中的那枚不在漂浮表里（它不占坑），
    /// 所以这里返回 `None`，由调用方沿用它当前的位置。
    fn target_of(&self, id: &str) -> Option<layout::Point> {
        if let Some(point) = self.ring_point(id) {
            return Some(point);
        }
        self.floating
            .iter()
            .find(|(fid, _, _)| fid == id)
            .map(|(_, point, _)| *point)
    }

    /// 漂浮相位（不在漂浮表里时退回 0：相位差只是让漂浮错开，不影响正确性）。
    fn phase_of(&self, id: &str) -> f32 {
        self.floating
            .iter()
            .find(|(fid, _, _)| fid == id)
            .map(|(_, _, phase)| *phase)
            .unwrap_or(0.0)
    }

    /// 一台设备**当前**的盘半径：拖着的是拖影那个大小，已连接的是缩小后的。
    ///
    /// 半径也属于「几何」，必须跟着当前状态走：连接后盘小了一圈，
    /// 还用名义半径去判定，就会「明明没挨着却弹开」。
    fn radius_of(&self, id: &str) -> f32 {
        let scale = if self.dragging.as_deref() == Some(id) {
            crate::model::drag::GHOST_SCALE
        } else if self.is_connected(id) {
            connect::CONNECTED_SCALE
        } else {
            1.0
        };
        layout::DEVICE_RADIUS * scale
    }

    /// 打开接收目录（只有收下来的文件才给这个动作）。
    ///
    /// 用系统自带的打开方式，不引 `rfd` 之类的依赖：这是内部工具，
    /// 「在访达 / 资源管理器里看到文件」够用了。
    pub fn open_received_folder(&mut self) {
        let dir = glowlink_ipc::state::state_dir().join("received");
        log::info!("打开接收目录：{}", dir.display());
        let dir = dir.display().to_string();
        #[cfg(target_os = "macos")]
        let command = "open";
        #[cfg(target_os = "windows")]
        let command = "explorer";
        #[cfg(all(unix, not(target_os = "macos")))]
        let command = "xdg-open";
        if let Err(e) = std::process::Command::new(command).arg(&dir).spawn() {
            log::info!("打不开目录（{command}）：{e}");
        }
    }

    /// 取消一次正在进行的传输（卡片右上角的 ✕）。
    ///
    /// 只**发出请求**：真正的收尾在守护进程的传输线程里做（它才知道传到哪儿了），
    /// 结果经下一轮 `TRANSFERS` 回到界面——界面不自己改状态，否则会和守护进程对不上。
    pub fn cancel_transfer(&mut self, token: &str) {
        let Some(source) = self.transfers.clone() else {
            log::info!("预览模式：不真的取消传输 #{token}");
            return;
        };
        log::info!("请求取消传输 #{token}");
        if let Err(e) = source.cancel(token) {
            log::info!("取消失败：{e}");
        }
    }

    /// 放弃一次配对（配对卡上的 ✕，或断开那台设备的气泡）。
    ///
    /// 两件事：① 让守护进程**放弃**这次配对（它还要掐掉那条连接，
    /// 否则后台线程要一直等到超时）；② 收起卡片。
    /// 已经结束的（成功/被拒/失败）只需收起，不再打扰守护进程。
    pub fn abandon_pairing(&mut self, token: &str) {
        let finished = self
            .pair
            .as_ref()
            .map(|card| card.state != glowlink_ipc::row::PairingState::WaitingPeer)
            .unwrap_or(true);
        if finished {
            self.dismiss_pair(token);
            return;
        }
        if let Some(pairing) = self.pairing.clone() {
            log::info!("放弃配对 #{token}");
            if let Err(e) = pairing.cancel(token) {
                log::info!("放弃配对失败：{e}");
            }
        }
        // 槽位会在下一轮快照里变成「已放弃」；这里先收起，界面立刻有反应
        self.dismiss_pair(token);
    }

    /// 收起配对卡（只影响本机视图，不改守护进程的状态）。
    pub fn dismiss_pair(&mut self, token: &str) {
        if !self.dismissed_pairs.iter().any(|t| t == token) {
            self.dismissed_pairs.push(token.to_string());
        }
        if self.pair.as_ref().map(|card| card.token.as_str()) == Some(token) {
            self.pair = None;
        }
    }

    /// 用户主动关掉传输卡（设计稿：完成后不自动消失，由用户自己关）。
    pub fn dismiss_transfer(&mut self, token: &str) {
        if !self.dismissed.iter().any(|t| t == token) {
            self.dismissed.push(token.to_string());
        }
        if self.transfer.as_ref().map(|row| row.token.as_str()) == Some(token) {
            self.transfer = None;
        }
    }

    /// 正在拖（拖动期间才需要每帧问窗口「指针还在不在里面」）。
    pub fn is_dragging(&self) -> bool {
        self.dragging.is_some()
    }

    /// 一帧物理（**唯一的时钟**）。`dt` 用真实间隔，弹簧内部会夹住异常大的值。
    ///
    /// 返回下一帧该隔多久：正在拖/弹簧在跑 → 快；只剩漂浮 → 慢（省 CPU）。
    /// `pointer_inside`：指针是否还在窗口里（由调用方向窗口问，见 `open`）。
    ///
    /// 拖到窗口外时松手事件**收不到**，拖动态会永远挂在那儿（表现就是「气泡贴在
    /// 边框上不动、要再点一下才恢复」）——所以指针一出窗口就取消这次拖动，
    /// 让它自己弹回去（skill §10「允许拖离再回来取消」）。
    pub fn tick(&mut self, pointer_inside: bool) -> Duration {
        if self.dragging.is_some() && !pointer_inside {
            log::info!("指针离开窗口：取消这次拖动（气泡自己弹回去）");
            self.settle_drag();
        }
        let now = std::time::Instant::now();
        let dt = (now - self.last_tick).as_secs_f32();
        self.last_tick = now;
        let ghost = self.dragging.as_ref().map(|id| Ghost {
            id: id.clone(),
            point: self.drag_at.unwrap_or_else(layout::self_center),
            radius: layout::DEVICE_RADIUS * crate::model::drag::GHOST_SCALE,
        });
        self.motion.step(dt, ghost.as_ref());
        // 传输：速度用**滑动窗口**算（相邻字节差 / 真实间隔），进度条再平滑一次。
        // 采样与平滑都放在时钟里，事件回调不做任何计算。
        match self.transfer.as_ref() {
            Some(row) if row.state == TransferState::Running => {
                let speed = self.meter.sample(row.done, now);
                self.transfer_speed = speed;
            }
            Some(_) => self.transfer_speed = 0.0,
            None => {}
        }
        let target = match (self.transfer.as_ref(), self.transfer_speed) {
            (Some(row), speed) => transfer::numbers(row, speed).fraction,
            (None, _) => 0.0,
        };
        self.progress.retarget(target as f32);
        self.progress.step(dt);

        self.snapping = self.over_center();
        // 亮起要**跟手**（0.08s：判定一变灯就亮，不留延时），
        // 熄灭慢慢褪（0.25s，免得边界上抖成闪烁）
        self.tint.set_response(if self.snapping { TINT_RISE } else { TINT_FALL });
        self.tint.retarget(if self.snapping { 1.0 } else { 0.0 });
        self.tint.step(dt);
        self.halo = (self.halo + dt / node::HALO_PERIOD_SECS).fract();
        self.brand_phase =
            (self.brand_phase + dt / titlebar::BRAND_PERIOD_SECS).fract();
        // 交替高亮：只有真在传的时候跳（传完就停在静止态）
        self.pulse = (self.pulse + dt / transfer::PULSE_PERIOD_SECS).fract();
        self.halo_fast = (self.halo_fast + dt / HALO_FAST_SECS).fract();
        self.flowing_peer = match self.transfer.as_ref() {
            Some(row) if row.state == TransferState::Running => Some(row.peer.clone()),
            _ => None,
        };
        let busy = self.dragging.is_some() || !self.tint.is_settled() || !self.motion.is_settled();
        if busy {
            TICK_FAST
        } else {
            TICK_SLOW
        }
    }

    /// 这次拖动「算不算落到中心盘上」——**高亮与吸附共用它**。
    ///
    /// 只看被拖起来的那枚气泡（圆心 + 当前半径），不看指针：
    /// 抓在盘边上时指针与气泡能差 20~30px，按指针算会出现「一边要多走、
    /// 另一边没到就吸上」。高亮是承诺，承诺与兑现必须是同一个判断。
    fn over_center(&self) -> bool {
        self.drag_at
            .map(|center| {
                drag::snap_hit(
                    center,
                    layout::self_center(),
                    layout::SELF_RADIUS,
                    layout::DEVICE_RADIUS * drag::GHOST_SCALE,
                )
            })
            .unwrap_or(false)
    }

    /// 拖到一起：登记一条连接，槽位**由中心点分配**（不看松手方向）。
    ///
    /// 已经连上的再拖一次 = 不重复登记（位置照旧由槽位说了算）。
    fn connect(&mut self, drag: &DeviceDrag) {
        if self.is_connected(&drag.id) {
            log::info!("已连接：{}（{}）", drag.name, drag.id);
            return;
        }
        self.connections.push(Connection { id: drag.id.clone(), angle: 0.0 });
        self.reassign_slots();
        // 它不再漂浮，钉在环上；之前「就地留下」的种子作废
        self.seeds.retain(|seed| seed.id != drag.id);
        log::info!(
            "已连接：{}（{}）→ {} 号位",
            drag.name,
            drag.id,
            self.connections.len()
        );
    }

    /// 按顺序重新分配环上的槽位：第 1 台在正上方，其余**顺时针**依次均匀分布。
    ///
    /// 断开一台后剩下的会压实重排（都走弹簧滑过去，看得见它去哪儿了）。
    fn reassign_slots(&mut self) {
        let angles = connect::slot_angles(self.connections.len());
        for (connection, angle) in self.connections.iter_mut().zip(angles) {
            connection.angle = angle;
        }
    }

    /// 断开连接：**位置留在原地**（不回到最初拖来的地方）。
    fn disconnect(&mut self, id: &str) {
        let Some(index) = self.connections.iter().position(|c| c.id == id) else {
            return;
        };
        let connection = self.connections.remove(index);
        self.reassign_slots();
        let point = connect::point_at(layout::self_center(), connection.angle);
        // 位置就地留下（当种子，不再回原位），但它恢复全尺寸并重新参与碰撞——
        // 「留在原地」与「不合法就得让开」是两件事，后者交给排斥迭代
        self.seeds.retain(|seed| seed.id != id);
        self.seeds.push(layout::Seed { id: id.to_string(), point });
        // 断开这台设备时，如果配对卡正为它开着，一并收起（用户的心智动作是同一个：
        // 「这台我不连了」）——否则卡片会孤零零留在屏幕上，还得再点一次 ✕
        let peer = self.devices.iter().find(|d| d.id == id).map(|d| d.name.clone());
        if let (Some(peer), Some(card)) = (peer, self.pair.clone()) {
            if card.peer == peer {
                self.abandon_pairing(&card.token);
            }
        }
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
    fn layout_for(&self, devices: &[DeviceView]) -> Vec<(String, layout::Point, f32)> {
        let dragging = self.dragging.as_deref();
        let floating: Vec<&str> = devices
            .iter()
            // 拖起来的那台**不占坑**：它已经被拿在手里了，老位置必须空出来
            //（否则别人既要躲拖影又要躲老位置——用户真机看到的正是这个）
            .filter(|d| !self.is_connected(&d.id) && dragging != Some(d.id.as_str()))
            .map(|d| d.id.as_str())
            .collect();
        let pinned = connect::pinned_points(&self.connections);
        let placement = layout::place_devices(&floating, &pinned, &self.seeds);
        floating
            .iter()
            .zip(placement.centers.iter())
            .zip(placement.phases.iter())
            .map(|((id, point), phase)| (id.to_string(), *point, *phase))
            .collect()
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
        let fresh = self.source.devices();
        let service_on = self.service.as_ref().map(|s| s.running()).unwrap_or(false);
        // 传输表：挑一条要显示的（正在跑的优先）。换了一条就把速度计清零——
        // 两条传输的字节数不能相减。
        let mut transfer_changed = false;
        if let Some(source) = self.transfers.clone() {
            let rows = source.transfers();
            let next = transfer::pick(&rows).map(|index| rows[index].clone());
            if next.as_ref().map(|row| row.token.clone())
                != self.transfer.as_ref().map(|row| row.token.clone())
            {
                self.meter.reset();
            }
            if next != self.transfer {
                // 用户关掉的那张卡不再弹回来（守护进程还会保留这条记录一小会儿）
                let visible = match next.as_ref() {
                    Some(row) => !self.dismissed.contains(&row.token),
                    None => true,
                };
                if visible {
                    transfer_changed = true;
                    if next.is_some() {
                        log::info!(
                            "传输卡：{}",
                            next.as_ref().map(|r| format!("{} {:?}", r.name, r.state)).unwrap_or_default()
                        );
                    }
                    self.transfer = next;
                }
            }
        }
        if fresh == self.devices
            && service_on == self.service_on
            && self.pair.is_none()
            && !transfer_changed
        {
            return false;
        }
        let mut changed = transfer_changed;
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
        // 目标点可能变了（设备上下线、连接、断开）：重算意图，运动系统自己会走过去
        self.refresh_floating();

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
                    // 用户已收起的卡片不再弹回来（守护进程还会保留这条记录一小会儿）
                    let hidden = self.dismissed_pairs.iter().any(|t| *t == fresh.token);
                    if hidden {
                        if self.pair.is_some() {
                            self.pair = None;
                            changed = true;
                        }
                    } else if fresh != card {
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

        // 交替高亮：本机与对端各拿一半强度（对端由 `flowing_peer` 匹配）
        let pulse = transfer::pulse(self.pulse);
        let devices: Vec<_> = self
            .devices
            .iter()
            .map(|device| {
                let connected = self.is_connected(&device.id);
                let being_dragged = self.dragging.as_deref() == Some(device.id.as_str());
                // 「在传」是比「已连接」更强的状态：两端交替亮起、呼吸加快
                let transferring = self.flowing_peer.as_deref() == Some(device.name.as_str());
                let peer_pulse = if transferring { pulse.1 } else { 0.0 };
                // 位置只有一个来源：运动系统的**当前**位置（渲染层不做任何判断）
                let center =
                    self.motion.position(&device.id).unwrap_or_else(layout::self_center);
                // 尺寸同理：拖着的是拖影那个大小，已连接的是缩小后的
                let scale = if being_dragged {
                    crate::model::drag::GHOST_SCALE
                } else if connected {
                    connect::CONNECTED_SCALE
                } else {
                    1.0
                };
                let size = layout::DEVICE_RADIUS * 2.0 * scale;
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
                        look: if transferring {
                            node::NodeLook::device_pulse(peer_pulse)
                        } else if connected {
                            node::NodeLook::device_connected()
                        } else {
                            node::NodeLook::device_default()
                        },
                        scale,
                        halo: if transferring { self.halo_fast } else { self.halo },
                    },
                )
                // 拖动：被拖起来的**就是这枚节点自己**（不是另做一个拖影）。
                // 它在运动系统里是「运动学物体」：位置 1:1 跟指针，并作为不可推动的
                // 障碍物去推别人（拖动的东西必须和指针粘在一起）。
                .on_drag(payload, |_drag: &DeviceDrag, _pos, _window, cx| {
                    cx.new(|_| node::DragHandle)
                })
                // **这个回调在元素内外都会触发**（gpui 按拖动起点投递），
                // 所以指针位置能一直跟着更新——落点判定就靠它
                // **这个回调在元素内外都会触发**（gpui 按拖动起点投递），
                // 所以指针位置能一直跟着更新——落点判定与「跟手」都靠它
                .on_drag_move(cx.listener(
                    |this, event: &DragMoveEvent<DeviceDrag>, window, cx| {
                        // **位置一律问窗口**（窗口坐标），不去拼「元素原点 + 相对位置」：
                        // 那个相对量到底相对谁，只有框架知道；拼错了就是「怎么拖都不亮」。
                        let pointer = graph_point(window.mouse_position());
                        let id = event.drag(cx).id.clone();
                        // 起手：记住「气泡中心 − 指针」，之后一直保持这个抓法
                        if this.dragging.as_deref() != Some(id.as_str()) {
                            let here = this.motion.position(&id).unwrap_or(pointer);
                            this.grab_offset = layout::Point {
                                x: here.x - pointer.x,
                                y: here.y - pointer.y,
                            };
                        }
                        let point = layout::Point {
                            x: pointer.x + this.grab_offset.x,
                            y: pointer.y + this.grab_offset.y,
                        };
                        let was_over = this.snapping;
                        let now_over = drag::snap_hit(
                            point,
                            layout::self_center(),
                            layout::SELF_RADIUS,
                            layout::DEVICE_RADIUS * drag::GHOST_SCALE,
                        );
                        if now_over != was_over {
                            // 进/出吸附区各记一行：出问题时这两行能直接说明判定发生没有
                            log::info!(
                                "{}吸附区（距中心 {:.0}px）",
                                if now_over { "进入" } else { "离开" },
                                layout::distance_to_center(point)
                            );
                        }
                        // 拖起来的这一瞬间，它原来的坑要空出来（别人可以挪过去）
                        if this.dragging.as_deref() != Some(id.as_str()) {
                            this.dragging = Some(id.clone());
                            this.refresh_floating();
                            if let Some(tone) = this.dragged_tone_of(&id) {
                                this.tint_tone = tone;
                            }
                        }
                        // 指针位置只记在这里，物理每 16ms 自己跑（不在事件回调里算物理）
                        this.drag_at = Some(point);
                        cx.notify();
                    },
                ));
                // 已连接：缩小的盘 + 一条「断开」按钮，位置钉在连接环上
                let visual = if connected {
                    let disconnect_id = device.id.clone();
                    div()
                        .relative()
                        .w(px(size))
                        .h(px(size))
                        .child(visual)
                        .child(
                            node::disconnect_button(
                                &t,
                                ElementId::Name(format!("unlink-{}", device.id).into()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.disconnect(&disconnect_id);
                                this.refresh_floating();
                                cx.notify();
                            })),
                        )
                        .into_any_element()
                } else {
                    visual.into_any_element()
                };
                // 摆到运动系统给的当前位置上：**不套任何动画**（位置每帧都在动）
                node::placed(visual, center, size).into_any_element()
            })
            .collect();

        let show_devices = self.state.phase().shows_devices();
        // 连上之后：本机盘也跟着缩小（「把设备拉过来一起变小」）并保持高亮
        let connected_any = !self.connections.is_empty();
        let self_scale = if connected_any { connect::CONNECTED_SCALE } else { 1.0 };

        div()
            .id("root")
            // 松手落在中心盘上就吸附、落在别处就回弹。
            //
            // 这层挂在**整窗根节点**上：不再自己划一块「落点区」的方框——
            // 那块方框按**指针**算、判定却按**气泡**算，两者不一致时就会出现
            // 「灯亮了、一松手却没吸上」（用户真机看到过）。现在判定只看距离。
            .on_drop(cx.listener(
                |this, drag: &DeviceDrag, window, cx| {
                    // 判定用**松手那一刻**的气泡中心（看的是「东西落到盘上」，
                    // 不是「手指在盘上」）
                    let pointer = graph_point(window.mouse_position());
                    let point = layout::Point {
                        x: pointer.x + this.grab_offset.x,
                        y: pointer.y + this.grab_offset.y,
                    };
                    // 判定与高亮**同一个函数**（亮的那个就是会吸上的那个）
                    let inside = drag::snap_hit(
                        point,
                        layout::self_center(),
                        layout::SELF_RADIUS,
                        layout::DEVICE_RADIUS * drag::GHOST_SCALE,
                    );
                    log::info!(
                        "松手判定：({:.0}, {:.0}) 距中心 {:.0}px → {}",
                        point.x,
                        point.y,
                        layout::distance_to_center(point),
                        if inside { "吸附" } else { "回弹" }
                    );
                    if inside {
                        // 命中：登记连接（摆到连接环上），弹簧会滑过去
                        this.dragging = None;
                        this.drag_at = None;
                        this.connect(drag);
                        this.refresh_floating();
                        this.start_pairing(drag);
                    } else {
                        // 没命中：不碰速度，让它带着指针的速度弹回原位
                        this.settle_drag();
                    }
                    cx.notify();
                    },
                ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.dragging.is_some() {
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
                    .child(titlebar::titlebar(&t, self.brand_phase))
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
            // 节点图：绝对定位区，子元素按**图区坐标**摆放（与 layout / motion 同一套）
            .child(
                div()
                    .relative()
                    .flex_none()
                    .h(px(theme::GRAPH_H))
                    .w_full()
                    // 中心盘：和设备**同一层**（物理世界保证不重叠：它是钉死的不动体，
                    // 会把压上来的气泡挤开）。位置钉在圆心，缩放也不挪中心。
                    .child({
                        let name = self.device_name.clone();
                        let center = self
                            .motion
                            .position(layout::SELF_ID)
                            .unwrap_or_else(layout::self_center);
                        let self_transferring = self.flowing_peer.is_some();
                        let look = if self_transferring {
                            node::NodeLook::self_pulse(pulse.0)
                        } else if connected_any {
                            node::NodeLook::self_node_blended(1.0, theme::NODE_SELF)
                        } else {
                            node::NodeLook::self_node()
                        };
                        let self_halo = if self_transferring { self.halo_fast } else { self.halo };
                        let visual = node::node(
                            &t_theme,
                            node::NodeSpec {
                                id: ElementId::Name("node-self".into()),
                                who: node::NodeWho::SelfNode,
                                label: &name.to_string(),
                                look,
                                scale: self_scale,
                                halo: self_halo,
                            },
                        );
                        // 染色层：叠一层同款、只改它的不透明度（gpui 里唯一能做到
                        // 「逐帧改色」且不动子树的做法）。不透明度来自弹簧。
                        let tinted = node::node(
                            &t_theme,
                            node::NodeSpec {
                                id: ElementId::Name("node-self-tinted".into()),
                                who: node::NodeWho::SelfNode,
                                label: &name.to_string(),
                                look: node::NodeLook::self_node_blended(1.0, self.tint_tone),
                                scale: self_scale,
                                halo: self_halo,
                            },
                        );
                        let tint = self.tint.x();
                        node::placed(
                            div()
                                .relative()
                                .child(visual)
                                .child(div().absolute().inset_0().child(tinted.opacity(tint))),
                            center,
                            theme::NODE_SELF_SIZE * self_scale,
                        )
                    })
                    // 设备气泡（画在中心盘之后 = 之上；重叠由物理保证不会发生）
                    .when(show_devices, |el| el.children(devices)),
            )
            // 拖拽区：设计稿里它是窗内固定的一段（44px + 16 边距），始终在
            .child(dropzone::dropzone(&t, self.drop, self.selected.as_deref()))
            // 浮层卡（配对 / 传输）画在拖拽区**之上**（层级：拖拽区 < 卡 < 隐私浮层）：
            // 两张卡占**同一个槽位**（同一个 `view::card` 外壳），所以同时出现时
            // 配对卡优先——配对是等人拍板的事，传输卡让它一步回去（「一次只强调一件事」）。
            .when(self.pair.is_none(), |el| {
                el.when_some(self.transfer.clone(), |el, row| {
                    let numbers = transfer::numbers(&row, self.transfer_speed);
                    let fraction = self.progress.x() as f64;
                    el.child(progress::card(&t, &row, &numbers, fraction, cx))
                })
            })
            .when_some(self.pair.clone(), |el, card| {
                el.child(paircard::card(
                    &t,
                    &card.token,
                    &card.peer,
                    &card.code,
                    card.state,
                    cx,
                ))
            })
            .child(titlebar::window_border(&t))
    }
}

/// 指针位置（`Pixels`，**窗口坐标**）→ 图区坐标。
///
/// **全项目唯一的窗口→图区换算**。曾经布局产出全窗坐标、渲染容器用图区坐标，
/// 两套混着用：设备整体低了 44px（靠收边界糊过去），拖动时气泡又会跳 44px。
fn graph_point(position: gpui::Point<gpui::Pixels>) -> layout::Point {
    layout::Point {
        x: position.x.as_f32(),
        y: position.y.as_f32() - theme::TITLEBAR_H,
    }
}

/// 打开主窗口。
pub fn open(
    device_name: String,
    source: Arc<dyn DeviceSource>,
    service: Option<Arc<IpcSource>>,
    pairing: Option<Arc<dyn PairingSource>>,
    transfers: Option<Arc<dyn TransferSource>>,
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
        let view = cx.new(|_| AppRoot::new(device_name, source, service, pairing, transfers));
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
        // 物理时钟：**独立于快照**。运动是唯一需要每帧更新的东西，
        // 用 300ms 的快照去驱动它，「弹开」「归位」都会是一顿一顿的。
        let motion_view = view.clone();
        cx.spawn(async move |cx| {
            let weak = motion_view.downgrade();
            let mut interval = TICK_FAST;
            loop {
                cx.background_executor().timer(interval).await;
                let alive = weak.update(cx, |root, cx| {
                    // 只在拖的时候才问窗口：空闲路径不碰任何窗口 API
                    let pointer_inside = if root.is_dragging() {
                        cx.windows()
                            .first()
                            .and_then(|handle| {
                                handle
                                    .update(cx, |_, window, _| {
                                        let point = window.mouse_position();
                                        let size = window.bounds().size;
                                        let (x, y) = (point.x.as_f32(), point.y.as_f32());
                                        let (w, h) =
                                            (size.width.as_f32(), size.height.as_f32());
                                        // 留 2px 容差：贴着边不算出去
                                        x >= -2.0 && y >= -2.0 && x <= w + 2.0 && y <= h + 2.0
                                    })
                                    .ok()
                            })
                            .unwrap_or(true)
                    } else {
                        true
                    };
                    let next = root.tick(pointer_inside);
                    cx.notify();
                    next
                });
                match alive {
                    Ok(next) => interval = next,
                    Err(_) => break,
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
