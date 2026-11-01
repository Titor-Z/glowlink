//! View：整窗装配 —— 320×500 玻璃窗。
//!
//! 窗口本身是**标准窗口 + 隐藏标题栏**（不是无边框窗口）：
//! macOS 与 Win11 会自带圆角与系统阴影，正好是设计稿的样子；
//! 自己造无边框窗口就得连圆角、阴影、拖拽一起自己实现，而且平台差异一堆。
//! 玻璃走 `WindowBackgroundAppearance::Blurred`（macOS = NSVisualEffectView，
//! Windows = DWM 组合属性），窗内再叠 `backdrop_filter` 做卡片玻璃。

use gpui::{
    div, prelude::*, px, App, Bounds, Context, DragMoveEvent, ElementId, EntityInputHandler,
    FocusHandle, IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString, Styled,
    UTF16Selection, Window, WindowAppearance, WindowBackgroundAppearance, WindowBounds,
    WindowOptions,
};

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::controller::{
    DeviceSource, DeviceView, IpcSource, PairingSource, PairingView, PendingView, ReceiveDirSource,
    TransferRow, TransferSource, TransferState, TrustedView, TrustSource,
};
use crate::model::connect::{self, Connection};
use crate::model::drag::{self, DeviceDrag};
use crate::model::layout;
use crate::model::motion::{Ghost, Motion};
use crate::model::namefield::{char_index_from_utf16, utf16_index_from_char, NameField};
use crate::model::paircode::CodeInput;
use crate::model::privacy;
use crate::model::settings::{self as settings_model, Settings, ThemeChoice};
use crate::model::spring::{Spring, ZETA_CRITICAL, ZETA_MOMENTUM};
use crate::model::transfer::{self, SpeedMeter};
use crate::model::state::{self, Phase, UiState};
use crate::theme::{self, Theme};
use crate::model::drop::{self, DropState};
use crate::view::{
    dropzone, node, paircard, pending as pending_card, progress, settings as settings_panel,
    spinner, titlebar,
};

/// 拒收态保持多久（设计稿：抖一下 + 提示 1.8s，然后自己回到常态）。
const DENY_HOLD: Duration = Duration::from_millis(1800);
/// 文件悬在窗口上多久没动静就当「已经不悬了」（离开窗口没有回调）。
const OVER_HOLD: Duration = Duration::from_millis(600);

/// 设置页分段控件的滑块时长（设计稿 `.seg__thumb` 的 `--motion-base` = 200ms）。
const SEG_SLIDE: Duration = Duration::from_millis(200);

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
    /// 底部拖拽区的状态（拖入 / 拒收由文件的拖放事件驱动）。
    drop: DropState,
    /// 拒收态要显示到什么时候（设计稿 1.8s 后自动回到常态）。
    deny_until: Option<std::time::Instant>,
    /// 文件正悬在窗口上的最后时刻：拖放离开窗口**没有回调**（gpui 把
    /// `FileDropEvent::Exited` 当输入事件发，不会派发到我们这一层），
    /// 所以用「久无动静」来收起高亮。
    over_until: Option<std::time::Instant>,
    /// 用户点拖拽区选中的文件（后台线程写完放进来，`sync` 取走）。
    picked: Arc<Mutex<Vec<std::path::PathBuf>>>,
    /// 一批发送里失败的说明（后台线程写，`sync` 取走并提示）。
    send_errors: Arc<Mutex<Vec<String>>>,
    /// 正在显示的这一批（一次投放 = 一批，守护进程按顺序发）。
    batch: Option<transfer::Batch>,
    /// 别人要发文件给你、等你拍板的那些请求（守护进程的 `PENDING` 表）。
    /// 界面只显示**第一条**：一次只强调一件事，处理完下一条自然浮上来。
    pending: Option<PendingView>,
    /// 待确认请求总数（多于 1 时卡片要补一句「另有 N 个」）。
    pending_total: usize,
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
    /// 发起侧槽位的第一次出现时刻（token + 时刻）：用来决定「等够 0.8s 才亮码」。
    pair_seen: Option<(String, std::time::Instant)>,
    /// 对端发起、等本机拍板的请求（接收侧卡片）。
    ///
    /// 与 `pair` 一个槽位：一个是「我求人」，一个是「人求我」，不同时强调两件事。
    pair_request: Option<paircard::RequestCard>,
    /// 接收侧那 6 位码的输入（纯状态机见 `model::paircode`）。
    code_input: CodeInput,
    /// 输入框的焦点：键盘事件只发给**拿到焦点**的那个元素，
    /// 不拿焦点的话用户敲数字毫无反应（看起来像卡死）。
    code_focus: FocusHandle,
    /// 焦点是否已交给输入框（每个请求只交一次，别每帧抢）。
    code_focused: bool,
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
    /// 空态 spinner 的相位（`0..1`）。
    spin: f32,
    /// 「当前这个局面」从什么时候开始（服务状态或设备有无一变就重新计时）。
    ///
    /// 空态那几句话是**过渡报告**，不是常驻说明：搜 10 秒后出结论、结论再留几秒就收起
    /// （常驻的说明在拖拽区）。所以这里记的是局面的起点，不是进程的启动时刻。
    note_since: std::time::Instant,
    /// 上一拍看到的局面（服务状态 + 有没有设备），用来判断「换局面了」。
    last_situation: Option<(state::ServiceState, bool)>,
    /// 上一拍拖拽区显示的那句话（变了就记日志）。
    last_zone_note: Option<String>,
    /// 预览专用：把服务当「在跑」（见 `open` 的 `preview_linked`）。真实路径恒为 `false`。
    preview_linked: bool,
    /// 隐私浮层开着吗（点盾牌开 / 点别处关 / Esc 关）。
    privacy_open: bool,
    /// 浮层是**哪一刻**开的：入场进度由它算（`privacy::IN` 之后不再动）。
    privacy_since: std::time::Instant,
    privacy_focus: FocusHandle,
    /// 用户**主动断开**过的设备 id：不再劝他连回去（他刚做过选择）。
    /// 设备下线（不在发现表里了）就清掉——下次发现是**新的一轮**，该提示还得提示。
    declined: std::collections::HashSet<String>,
    /// 焦点交过一次了吗（每帧都抢焦点的话，用户点别处也会被抢回来）。
    privacy_focused: bool,
    /// **传输中**的呼吸相位：设计稿要求光圈加快到 1.5s（表示「正在跑」）。
    halo_fast: f32,
    /// 正在传数据的对端显示名（`None` = 没有传输）。线、节点、卡片三处共用它。
    flowing_peer: Option<String>,
    /// 配对名单来源（设置页；预览模式下是假数据）。
    trusts: Option<Arc<dyn TrustSource>>,
    /// 接收目录来源（设置页）。
    receive: Option<Arc<dyn ReceiveDirSource>>,
    /// 桌面设置（主题、空态开关、接收目录；持久化见 `model::settings`）。
    /// `pub(crate)`：设置面板的开关回调要读当前值再取反（`view::settings`）。
    pub(crate) settings: Settings,
    /// 配对名单快照（设置面板画的就这份）。
    trusts_list: Vec<TrustedView>,
    /// 接收目录快照。
    receive_dir: Option<String>,
    /// 用户在设置页选的新目录（后台线程写完放进来，`sync` 取走）。
    picked_dir: Arc<Mutex<Option<std::path::PathBuf>>>,
    /// 设置面板开着吗（标题栏齿轮点开 / 返回或 Esc 关）。
    settings_open: bool,
    /// 面板是**哪一刻**开的：入场进度由它算。
    settings_since: std::time::Instant,
    /// 分段控件滑块的起点档位与起始时刻（gpui 没有 CSS transition，用自家时钟）。
    seg_from: f32,
    seg_since: std::time::Instant,
    settings_focus: FocusHandle,
    /// 焦点交过一次了吗（与隐私浮层同一套做法）。
    settings_focused: bool,
    /// 正在确认删除的配对（指纹）：删除走两步，一步到位就是替用户省掉反悔的机会。
    /// `pub(crate)`：确认按钮从这取「正在删哪条」（`view::settings`）。
    pub(crate) confirm_delete: Option<String>,
    /// 正在改名的配对（`None` = 没有在改）。
    rename: Option<RenameSession>,
    /// 改名输入框的焦点：键盘事件只发给拿到焦点的元素，IME 也是。
    rename_focus: FocusHandle,
    /// 焦点交过一次了吗（与配对码输入同一套做法）。
    rename_focused: bool,
    /// 输入框在屏幕上的位置（paint 时由 canvas 记下，IME 候选窗定位要用）。
    rename_bounds: Arc<Mutex<Option<Bounds<Pixels>>>>,
    /// 已「拖到一起」的设备（连接环上的方位）。
    connections: Vec<Connection>,
    /// 断开之后就地留下的落点（按设备 ID 记）。
    ///
    /// 是**种子**而不是钉子：它不回原位，但也不能压在中心盘或别人身上，
    /// 所以交给排斥迭代推到最近的合法位置。
    seeds: Vec<layout::Seed>,
}

/// 正在改名的配对（哪一台 + 输入状态）。一次只改一台。
#[derive(Clone, Debug, PartialEq)]
struct RenameSession {
    fingerprint: String,
    field: NameField,
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
        cx: &mut Context<Self>,
        device_name: impl Into<SharedString>,
        source: Arc<dyn DeviceSource>,
        service: Option<Arc<IpcSource>>,
        pairing: Option<Arc<dyn PairingSource>>,
        transfers: Option<Arc<dyn TransferSource>>,
        trusts: Option<Arc<dyn TrustSource>>,
        receive: Option<Arc<dyn ReceiveDirSource>>,
        settings: Settings,
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
            deny_until: None,
            over_until: None,
            picked: Arc::new(Mutex::new(Vec::new())),
            send_errors: Arc::new(Mutex::new(Vec::new())),
            batch: None,
            pending: None,
            pending_total: 0,
            selected: None,
            pairing,
            transfers,
            transfer: None,
            meter: SpeedMeter::default(),
            dismissed: Vec::new(),
            dismissed_pairs: Vec::new(),
            pair_seen: None,
            pair_request: None,
            code_input: CodeInput::new(),
            code_focus: cx.focus_handle(),
            code_focused: false,
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
            spin: 0.0,
            note_since: std::time::Instant::now(),
            last_situation: None,
            last_zone_note: None,
            preview_linked: false,
            privacy_open: false,
            privacy_since: std::time::Instant::now(),
            privacy_focus: cx.focus_handle(),
            declined: std::collections::HashSet::new(),
            privacy_focused: false,
            halo_fast: 0.0,
            flowing_peer: None,
            trusts,
            receive,
            settings,
            trusts_list: Vec::new(),
            receive_dir: None,
            picked_dir: Arc::new(Mutex::new(None)),
            settings_open: false,
            settings_since: std::time::Instant::now(),
            seg_from: 0.0,
            seg_since: std::time::Instant::now(),
            settings_focus: cx.focus_handle(),
            settings_focused: false,
            confirm_delete: None,
            rename: None,
            rename_focus: cx.focus_handle(),
            rename_focused: false,
            rename_bounds: Arc::new(Mutex::new(None)),
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
        // 目录以界面上那个值为准（守护进程的实际情况；没在跑时退回本机设置），
        // 只有两处都没有才用内置默认——不然改了目录，这里仍会打开旧位置。
        let dir = settings_model::effective_receive_dir(
            self.receive_dir.clone(),
            self.settings.receive_dir.as_deref(),
        )
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| glowlink_ipc::state::state_dir().join("received"));
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

    /// 接收侧第一段：用户点了「允许」——把卡片切到输入段，**此时还不发任何东西**
    /// （协议里「允许」与「码」是同一次提交：码要从对方屏幕上抄过来）。
    pub fn allow_pairing(&mut self) {
        if let Some(request) = self.pair_request.as_mut() {
            request.allowed = true;
            request.wrong = false;
            self.code_input.clear();
            self.code_focused = false;
        }
    }

    /// 接收侧：提交输入的码（对错由守护进程/对端判定，错了会话保持、可重输）。
    pub fn submit_pair_code(&mut self) {
        let Some(request) = self.pair_request.clone() else {
            return;
        };
        if !self.code_input.is_complete() {
            return;
        }
        let Some(pairing) = self.pairing.clone() else {
            return;
        };
        let code = self.code_input.digits().to_string();
        log::info!("提交配对码 #{0}", request.token);
        if let Err(e) = pairing.approve(&request.token, &code) {
            log::info!("提交配对码失败：{e}");
            return;
        }
        // 先不收起卡片：对没对由对端说了算；下一轮快照里「请求仍在 + 剩余次数减少」
        // 就是输了（那时清空输入，用户重输）
        self.code_input.clear();
        self.code_focused = false;
    }

    /// 接收侧：拒绝（或卡片上的 ✕）。
    pub fn reject_pairing(&mut self, token: String) {
        if let Some(pairing) = self.pairing.clone() {
            log::info!("拒绝配对 #{token}");
            if let Err(e) = pairing.reject(&token) {
                log::info!("拒绝配对失败：{e}");
            }
        }
        self.pair_request = None;
        self.code_input.clear();
        self.code_focused = false;
    }

    /// 盾牌：开/关隐私浮层（同一个按钮两个方向，用户不必再找关闭键——点别处也行）。
    pub fn toggle_privacy(&mut self) {
        self.privacy_open = !self.privacy_open;
        self.privacy_since = std::time::Instant::now();
        // 关掉之后要能再交一次（否则第二次打开 Esc 就不灵了）
        self.privacy_focused = false;
        log::info!("隐私浮层：{}", if self.privacy_open { "开" } else { "关" });
    }

    /// 关掉浮层（点窗口别处 / Esc / 再点一次盾牌）。
    pub fn close_privacy(&mut self) {
        if self.privacy_open {
            self.privacy_open = false;
            log::info!("隐私浮层：关");
        }
    }

    // ----------------------------------------------------- 设置面板

    /// 齿轮：开/关设置面板（返回按钮、Esc、点别处……不，点别处不关：
    /// 面板里全是可点的控件，「点外面关」会把误触变成丢设置）。
    pub fn toggle_settings(&mut self) {
        self.settings_open = !self.settings_open;
        self.settings_since = std::time::Instant::now();
        self.settings_focused = false;
        self.confirm_delete = None;
        log::info!("设置面板：{}", if self.settings_open { "开" } else { "关" });
    }

    pub fn close_settings(&mut self) {
        if self.settings_open {
            self.settings_open = false;
            self.confirm_delete = None;
            log::info!("设置面板：关");
        }
    }

    /// 面板入场进度（0..1）：与隐私浮层同一条曲线（`privacy::pop_in`）。
    pub fn settings_progress(&self) -> f32 {
        let total = crate::model::privacy::IN.as_secs_f32();
        if total <= 0.0 {
            return 1.0;
        }
        (self.settings_since.elapsed().as_secs_f32() / total).min(1.0)
    }

    pub fn wants_settings_focus(&self) -> bool {
        self.settings_open && !self.settings_focused
    }

    /// 改设置项并落盘。**只有真的变了才写**：每 300ms 的快照轮询不该变成磁盘写入器。
    fn change_settings(&mut self, note: &str, change: impl FnOnce(&mut Settings)) {
        let before = self.settings.clone();
        change(&mut self.settings);
        if self.settings == before {
            return;
        }
        log::info!("设置：{note}");
        if let Err(e) = settings_model::save(&self.settings) {
            log::info!("设置保存失败：{e}");
        }
    }

    pub fn set_theme(&mut self, theme: ThemeChoice) {
        if self.settings.theme != theme {
            // 记住从哪一档出发：滑块要滑过去，不是跳过去
            self.seg_from = self.settings.theme.index() as f32;
            self.seg_since = std::time::Instant::now();
        }
        self.change_settings(&format!("主题 → {:?}", theme), |s| s.theme = theme);
    }

    /// 分段控件滑块的当前位置（浮点档位）：200ms 内从上一档滑到 `to`。
    ///
    /// gpui 没有 CSS transition，也没有 transform，所以位移由自家时钟算——
    /// 与面板入场同一条缓出曲线（快起慢收）。
    pub fn seg_position(&self, to: usize) -> f32 {
        let total = SEG_SLIDE.as_secs_f32();
        let p = if total <= 0.0 {
            1.0
        } else {
            (self.seg_since.elapsed().as_secs_f32() / total).min(1.0)
        };
        let eased = 1.0 - (1.0 - p).powi(3);
        self.seg_from + (to as f32 - self.seg_from) * eased
    }

    /// 滑块还在滑吗（在滑就得每帧重绘）。
    pub fn seg_animating(&self) -> bool {
        self.seg_since.elapsed() < SEG_SLIDE
    }

    pub fn set_self_name_in_empty(&mut self, on: bool) {
        self.change_settings(&format!("空态显示本机名字 → {on}"), |s| {
            s.self_name_in_empty = on
        });
    }

    pub fn set_note_persistent(&mut self, on: bool) {
        self.change_settings(&format!("空态提示常驻 → {on}"), |s| s.note_persistent = on);
    }

    /// 配对名单：进入删除确认态（第二步才真的删）。
    pub fn begin_delete_trust(&mut self, fingerprint: &str) {
        log::info!("删除配对（待确认）：{fingerprint}");
        self.confirm_delete = Some(fingerprint.to_string());
    }

    pub fn cancel_delete_trust(&mut self) {
        self.confirm_delete = None;
    }

    /// 真的删：交给守护进程（名单归它管），名单下一轮快照里少一条。
    pub fn confirm_delete_trust(&mut self, fingerprint: &str) {
        self.confirm_delete = None;
        let Some(source) = self.trusts.clone() else {
            log::info!("预览模式：不真的删除配对 {fingerprint}");
            return;
        };
        log::info!("删除配对：{fingerprint}");
        if let Err(e) = source.untrust(fingerprint) {
            log::info!("删除配对失败：{e}");
        }
    }

    /// 进入改名态：输入框预填当前名字，焦点等下一帧交（`wants_rename_focus`）。
    pub fn begin_rename(&mut self, fingerprint: &str, current: &str) {
        log::info!("改名（开始）：{fingerprint}");
        self.confirm_delete = None;
        self.rename_focused = false;
        self.rename = Some(RenameSession {
            fingerprint: fingerprint.to_string(),
            field: NameField::new(current),
        });
    }

    pub fn cancel_rename(&mut self) {
        if self.rename.take().is_some() {
            log::info!("改名：取消");
        }
        self.rename_focused = false;
    }

    /// 保存改名：空名字不发（那是删名字不是改名），没变也不发（别让守护进程白写盘）。
    pub fn save_rename(&mut self) {
        let Some(session) = self.rename.take() else { return };
        self.rename_focused = false;
        let name = session.field.text().trim().to_string();
        if name.is_empty() {
            log::info!("改名：名字是空的，当作取消");
            return;
        }
        let unchanged = self
            .trusts_list
            .iter()
            .find(|trust| trust.fingerprint == session.fingerprint)
            .map(|trust| trust.name == name)
            .unwrap_or(false);
        if unchanged {
            log::info!("改名：名字没变");
            return;
        }
        let Some(source) = self.trusts.clone() else {
            log::info!("预览模式：不真的改名");
            return;
        };
        log::info!("改名：{} → {}", session.fingerprint, name);
        if let Err(e) = source.rename(&session.fingerprint, &name) {
            log::info!("改名失败：{e}");
        }
    }

    /// 改名输入框的键盘：只吃退格 / 回车 / Esc。
    ///
    /// **打印字符不在这里接**——它们走输入处理器（`EntityInputHandler`，
    /// 中文 IME 与普通字符同一条路）。这里返回 `true` 的键要拦下冒泡
    /// （否则 Esc 会把整个设置面板一起关掉）。
    pub fn rename_key(&mut self, key: &str) -> bool {
        let Some(session) = self.rename.as_mut() else { return false };
        match key {
            "backspace" => {
                session.field.backspace();
                true
            }
            "enter" => {
                self.save_rename();
                true
            }
            "escape" => {
                self.cancel_rename();
                true
            }
            _ => false,
        }
    }

    pub fn wants_rename_focus(&self) -> bool {
        self.rename.is_some() && !self.rename_focused
    }

    /// 改名输入框当前显示的文本（`view::settings` 画输入框用）。
    pub fn rename_text(&self) -> String {
        self.rename
            .as_ref()
            .map(|session| session.field.text().to_string())
            .unwrap_or_default()
    }

    /// 输入框焦点句柄（paint 期注册输入处理器要用）。
    pub fn rename_focus_handle(&self) -> FocusHandle {
        self.rename_focus.clone()
    }

    /// 输入框位置槽（paint 期写入，IME 候选窗定位读取）。
    pub fn rename_bounds_slot(&self) -> Arc<Mutex<Option<Bounds<Pixels>>>> {
        Arc::clone(&self.rename_bounds)
    }

    /// 换接收目录：系统对话框选一个，结果经 `picked_dir` 在 `sync` 里落地
    /// （对话框是阻塞的原生模态，只能后台开——与选文件同一套做法）。
    pub fn choose_receive_dir(&mut self) {
        let picked = Arc::clone(&self.picked_dir);
        std::thread::spawn(move || {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                *picked.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir);
            }
        });
    }

    /// 入场进度（0..1）：跑完一直是 1，界面不必再为它重绘。
    pub fn privacy_progress(&self) -> f32 {
        let total = privacy::IN.as_secs_f32();
        if total <= 0.0 {
            return 1.0;
        }
        (self.privacy_since.elapsed().as_secs_f32() / total).min(1.0)
    }

    /// 该不该把键盘焦点交给浮层：开着、且还没交过。
    ///
    /// 不交的话 Esc 收不到（键盘事件只发给拿到焦点的元素），而界面上看不出缺了什么——
    /// 用户只会觉得"Esc 不管用"。**只交一次**：每帧都抢焦点，用户点别处也会被抢回来。
    pub fn wants_privacy_focus(&self) -> bool {
        self.privacy_open && !self.privacy_focused
    }

    /// 该不该把焦点交给输入框：请求在、已允许、且还没交过。
    pub fn wants_code_focus(&self) -> bool {
        self.pair_request.as_ref().map(|r| r.allowed).unwrap_or(false) && !self.code_focused
    }

    /// 键盘：只吃数字与退格；回车 = 确认，Esc = 拒绝。
    ///
    /// **不碰 IME / `EntityInputHandler`**：6 位数字用不上输入法，
    /// 碰那套就要处理 UTF-16 换算与组合态，代价远大于收益。
    pub fn code_key(&mut self, key: &str) -> bool {
        match key {
            "backspace" => self.code_input.backspace(),
            "enter" => {
                self.submit_pair_code();
                true
            }
            "escape" => {
                if let Some(request) = self.pair_request.clone() {
                    self.reject_pairing(request.token);
                }
                true
            }
            _ => {
                let mut typed = false;
                for ch in key.chars() {
                    typed |= self.code_input.push(ch);
                }
                typed
            }
        }
    }

    /// 点一台设备 = 选中它（再点一下取消）。
    ///
    /// 设计稿的 `select()`：单选、可反选；选中后拖拽区文案变成「拖到此处 → 甲机」。
    /// 选的是**设备 ID**（名字可以重，见 `drop::accept_target`）。
    pub fn toggle_selected(&mut self, id: &str) {
        if self.selected.as_deref() == Some(id) {
            self.selected = None;
            log::info!("取消选择设备 {id}");
        } else {
            self.selected = Some(id.to_string());
            log::info!("选中设备 {id}");
        }
    }

    /// 已连接设备的 ID（环上那些）——**可发名单**。
    fn connected_ids(&self) -> Vec<&str> {
        self.connections.iter().map(|c| c.id.as_str()).collect()
    }

    /// 这批文件该发给谁（`Err` 里是给用户看的那句话）。
    ///
    /// 可发名单是**已连接的**：发现到的设备要先用拖到中心连接（配对）才能收文件
    /// （用户拍板的流程规则，见 [`drop::accept_target`]）。
    fn drop_target(&self) -> std::result::Result<DeviceView, &'static str> {
        let discovered: Vec<&str> = self.devices.iter().map(|d| d.id.as_str()).collect();
        let connected = self.connected_ids();
        let id = drop::accept_target(&discovered, &connected, self.selected.as_deref())?;
        self.devices
            .iter()
            .find(|device| device.id == id)
            .cloned()
            .ok_or("选中的设备已不在了")
    }

    /// 后台服务对界面而言的三态（`Starting` 那几百毫秒不能说「已关闭」）。
    pub fn service_state(&self) -> state::ServiceState {
        // 预览没有真服务：只有显式要求「假装在跑」时才走设备那几句文案
        if self.preview_linked {
            return state::ServiceState::On;
        }
        match self.service.as_ref() {
            None => state::ServiceState::Off,
            Some(service) if service.starting() => state::ServiceState::Starting,
            Some(service) if service.running() => state::ServiceState::On,
            Some(_) => state::ServiceState::Off,
        }
    }

    /// 空态现在该说什么、还转不转（纯规则见 [`state::empty_state`]）。
    pub fn empty_note(&self) -> Option<(state::EmptyNote, bool)> {
        state::empty_state(
            self.service_state(),
            self.devices.len(),
            self.note_since.elapsed(),
            self.settings.note_persistent,
        )
    }

    /// 拖拽区文案要的现状：发现了几台、连了几台、选了哪台、唯一那台已连接的是谁。
    fn zone(&self) -> drop::Zone<'_> {
        let name_of = |id: &str| {
            self.devices
                .iter()
                .find(|device| device.id == id)
                .map(|device| device.name.as_str())
        };
        // 选中只对**已连接**的设备有意义：选了一台没连的，文案不该说「拖到此处 → 它」
        let selected = self
            .selected
            .as_deref()
            .filter(|id| self.is_connected(id))
            .and_then(name_of);
        // 发现的设备是不是**一台可劝的都没有**了（都被主动断开过）
        let all_declined = !self.devices.is_empty()
            && self
                .devices
                .iter()
                .all(|device| self.declined.contains(&device.id));
        let only = if self.connections.len() == 1 {
            name_of(&self.connections[0].id)
        } else {
            None
        };
        drop::Zone {
            device_count: self.devices.len(),
            connected: self.connections.len(),
            all_declined,
            selected,
            only,
            service: self.service_state(),
        }
    }

    /// 拒收：改文案 + 抖一下，1.8s 后自己回到常态（设计稿）。
    fn deny_drop(&mut self, reason: &'static str) {
        log::info!("投放被拒：{reason}");
        // 选中的设备没了就顺手清掉选择，否则用户会一直对着一台不存在的目标
        if reason == "选中的设备已不在了" {
            self.selected = None;
        }
        self.drop = DropState::Denied(reason);
        self.deny_until = Some(std::time::Instant::now() + DENY_HOLD);
    }

    /// 用户扔进来一批文件（拖放或选文件都走这里）。
    ///
    /// **一次投放 = 一批**：全部排进守护进程的发送队列（它按顺序一个文件一个文件地发），
    /// 界面只留一条卡片，位置用 `2/5` 表示。
    pub fn accept_files(&mut self, paths: Vec<std::path::PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let target = match self.drop_target() {
            Ok(target) => target,
            Err(reason) => {
                self.deny_drop(reason);
                return;
            }
        };
        let Some(transfers) = self.transfers.clone() else {
            self.deny_drop("现在不能发送");
            return;
        };
        let names: Vec<String> = paths
            .iter()
            .map(|path| {
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string())
            })
            .collect();
        log::info!(
            "投放到「{}」：{} 个文件（{}）",
            target.name,
            names.len(),
            names.join("、")
        );
        self.batch = Some(transfer::Batch { peer: target.name.clone(), names });
        self.drop = DropState::Idle;
        self.deny_until = None;

        // 入队是同步 IPC（几十微秒），但**必须在后台线程**：这是真实连接，
        // 放在拖放回调里就是「窗口卡一下」（gpui 的 executor 是裸 pollster，
        // 阻塞它会连其他后台任务一起拖住）。
        let errors = Arc::clone(&self.send_errors);
        let device_id = target.id.clone();
        std::thread::spawn(move || {
            for path in paths {
                let text = path.to_string_lossy().to_string();
                if let Err(e) = transfers.send(&device_id, &text) {
                    let note = format!("{}：{e}", path.display());
                    log::info!("排入发送队列失败：{note}");
                    errors.lock().unwrap_or_else(|e| e.into_inner()).push(note);
                }
            }
        });
    }

    /// 点拖拽区 = 用系统对话框选文件（设计稿的 `fileIn.click()`）。
    ///
    /// **先查前置条件**：没有可发的设备就别让用户白挑一遍文件
    /// （拖拽区上本来写着「先选择一台设备」，挑完再说「不能发」是耍人）。
    pub fn pick_files(&mut self) {
        if let Err(reason) = self.drop_target() {
            self.deny_drop(reason);
            return;
        }
        let picked = Arc::clone(&self.picked);
        // 对话框是**阻塞**的（原生模态窗口）：只能后台开，选完把结果放回共享状态，
        // 由 `sync` 取走——绝不在渲染或事件回调里等它。
        std::thread::spawn(move || {
            let files = rfd::FileDialog::new().pick_files().unwrap_or_default();
            if !files.is_empty() {
                picked.lock().unwrap_or_else(|e| e.into_inner()).extend(files);
            }
        });
    }

    /// 接受一个待确认的接收请求（码已经由人核对过）。
    ///
    /// 接受之后就交给守护进程了：它会登记成一条传输，界面的传输卡随之出现。
    pub fn accept_receive(&mut self) {
        let Some(pending) = self.pending.clone() else {
            return;
        };
        let Some(transfers) = self.transfers.clone() else {
            return;
        };
        log::info!(
            "接受接收 #{0}：来自「{1}」的 {2}（校验码 {3}）",
            pending.token,
            pending.peer,
            pending.name,
            pending.code
        );
        if let Err(e) = transfers.decide(&pending.token, true) {
            log::info!("接受失败：{e}");
            return;
        }
        // 先收起卡片：下一轮快照里这条请求就没了，传输卡会紧接着出现
        self.pending = None;
    }

    /// 拒绝一个待确认的接收请求（卡片上的「拒绝」或 ✕）。
    pub fn reject_receive(&mut self, token: String) {
        if let Some(transfers) = self.transfers.clone() {
            log::info!("拒绝接收 #{token}");
            if let Err(e) = transfers.decide(&token, false) {
                log::info!("拒绝失败：{e}");
            }
        }
        self.pending = None;
    }

    /// 卡片上的 ✕：在传 ⇒ **取消这一批**；已结束 ⇒ 收起卡片。
    pub fn close_transfer_card(&mut self) {
        let Some(row) = self.transfer.clone() else {
            return;
        };
        if row.state.is_terminal() {
            self.dismiss_transfer(&row.token);
            return;
        }
        let Some(batch) = self.batch.clone() else {
            self.cancel_transfer(&row.token);
            return;
        };
        // 整批：正在传的那个 + 还在排队的那些（守护进程按编号各自收尾）
        let rows = self
            .transfers
            .as_ref()
            .map(|source| source.transfers())
            .unwrap_or_default();
        for target in batch.cancel_targets(&rows) {
            self.cancel_transfer(&target.token);
        }
        log::info!("取消这一批（「{}」共 {} 个文件）", batch.peer, batch.total());
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
        // 这批已经结束了：把批次也放下（它的使命就是让卡片知道「2/5」）
        if self
            .batch
            .as_ref()
            .map(|batch| batch.names.iter().any(|_| true))
            .unwrap_or(false)
        {
            let rows: Vec<TransferRow> = self
                .transfers
                .as_ref()
                .map(|source| source.transfers())
                .unwrap_or_default();
            if self.batch.as_ref().map(|b| b.is_finished(&rows)).unwrap_or(false) {
                self.batch = None;
            }
        }
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
        self.spin = (self.spin + dt / spinner::SPINNER_PERIOD_SECS).fract();
        // 交替高亮：只有真在传的时候跳（传完就停在静止态）
        self.pulse = (self.pulse + dt / transfer::PULSE_PERIOD_SECS).fract();
        self.halo_fast = (self.halo_fast + dt / HALO_FAST_SECS).fract();
        self.flowing_peer = match self.transfer.as_ref() {
            Some(row) if row.state == TransferState::Running => Some(row.peer.clone()),
            _ => None,
        };
        // **只有真的在转**才需要快档：静止的结论句不该继续吃 60fps 重绘
        // （「一直提示 + 一直转」正是用户抱怨的那两件事）
        let spinning = self
            .empty_note()
            .map(|(_, spinning)| spinning)
            .unwrap_or(false);
        let busy = self.dragging.is_some()
            || !self.tint.is_settled()
            || !self.motion.is_settled()
            || spinning
            // 浮层入场那 0.2s 要每帧重绘，落定后就安静
            || (self.privacy_open && privacy::animating(self.privacy_progress()))
            || (self.settings_open && privacy::animating(self.settings_progress()))
            || self.seg_animating();
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

    /// 预览专用：把第一台设备接上（真实路径靠用户拖，这个方法只给 `GLOWLINK_PREVIEW=linked`）。
    fn connect_first(&mut self) {
        let Some(device) = self.devices.first().cloned() else {
            return;
        };
        let drag = DeviceDrag {
            id: device.id.clone(),
            name: device.name.clone(),
            kind: device.kind,
        };
        self.connect(&drag);
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
        // 记下「这是他主动断的」：拖拽区就不再说「把设备拖到中间即可发送」
        self.declined.insert(id.to_string());
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
            // 不在本地先占一张卡：槽位是守护进程登记的，卡片**由快照决定**。
            // 已信任的设备会在几十毫秒内拿到放行帧，于是卡片根本不出现（「无感」）。
            Ok(()) => log::info!("发起配对：{}（{}）", drag.name, drag.id),
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
    /// 发起侧卡片该显示成什么（`None` = 不显示）。
    ///
    /// 三条规矩：
    /// - **只有守护进程登记的槽位才算数**（界面不自己造卡）；
    /// - 用户收起过的编号不再弹回来；
    /// - 信任的设备先**不打扰**：对端免码放行几乎立刻就回，卡片根本不会出现；
    ///   真需要人工核对时（陌生设备、或对端不信任本机）等够
    ///   [`drag::CODE_DELAY_MS`] 再把码亮出来。
    fn next_pair_card(&mut self, row: Option<&PairingView>) -> Option<PairingCard> {
        let row = row?;
        let token = row.token.clone();
        // 记下这个 token 第一次出现的时刻（同一 token 不重置，否则永远等不到 0.8s）
        match &self.pair_seen {
            Some((seen, at)) if *seen == token => {
                let _ = at;
            }
            _ => self.pair_seen = Some((token.clone(), std::time::Instant::now())),
        }
        let waited_ms = self
            .pair_seen
            .as_ref()
            .map(|(_, at)| at.elapsed().as_millis() as u64)
            .unwrap_or(0);
        if self.dismissed_pairs.iter().any(|t| *t == token) {
            return None;
        }
        if !drag::show_pair_card(row.state, waited_ms) {
            return None;
        }
        let fresh = PairingCard {
            token,
            peer: row.peer.clone(),
            code: row.code.clone(),
            state: row.state,
        };
        if self.pair.as_ref() != Some(&fresh) {
            log::info!(
                "配对卡显示：{}（{}）",
                drag::pair_state_note(fresh.state),
                fresh.peer
            );
        }
        Some(fresh)
    }

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
        // 接收侧的请求要先**取出来**再判断（`PAIRPENDING` 不属于上面那三样）：
        // 守卫只看设备表/服务/传输，设备表没变就直接早退的话，
        // 新来的配对请求永远轮不到处理——界面看着像没反应（这个坑刚踩过）。
        // 别人发来的待确认（接收侧）：与配对请求一样，是**新增的外来状态**，
        // 必须一起进守卫，否则「设备表没变」时它永远轮不到处理。
        let pending_rows: Vec<PendingView> = self
            .transfers
            .as_ref()
            .map(|source| source.pending())
            .unwrap_or_default();
        let pending_next = pending_rows.first().cloned();
        let pending_changed = pending_next.as_ref().map(|row| row.token.clone())
            != self.pending.as_ref().map(|row| row.token.clone())
            || pending_rows.len() != self.pending_total;
        let incoming = self
            .pairing
            .as_ref()
            .and_then(|pairing| pairing.pending().into_iter().next());
        let incoming_changed = match (&incoming, &self.pair_request) {
            (Some(row), Some(card)) => row.token != card.token || row.remaining != card.remaining,
            (Some(_), None) | (None, Some(_)) => true,
            (None, None) => false,
        };
        // 配对名单与接收目录也是**外来状态**（守护进程的）：进守卫，
        // 否则删了一条配对、换了目录，面板却因为「设备表没变」不重绘——同一类坑。
        let trusts_next: Vec<TrustedView> = self
            .trusts
            .as_ref()
            .map(|source| source.trusts())
            .unwrap_or_default();
        let trusts_changed = trusts_next != self.trusts_list;
        let receive_next = self.receive.as_ref().and_then(|source| source.receive_dir());
        let receive_changed = receive_next != self.receive_dir;
        // 发起侧的槽位也要先取出来：卡片**由快照决定**，所以它同样不能只靠
        // 「设备表变了没」来判断（同一类坑：守卫漏了新状态）。
        let outgoing = self
            .pairing
            .as_ref()
            .and_then(|pairing| pairing.outgoing().into_iter().last());
        let outgoing_changed = match (&outgoing, &self.pair) {
            (Some(row), Some(card)) => row.token != card.token || row.state != card.state,
            (Some(_), None) | (None, Some(_)) => true,
            (None, None) => false,
        };
        // 显示决策每个 tick 都要重算：它还依赖「等了多久」（信任的设备先不亮码）。
        let next_pair = self.next_pair_card(outgoing.as_ref());
        let pair_changed = next_pair != self.pair;
        // 拖拽区文案变了就记一条：它写着「能不能发、发给谁」，是**流程规则**的对外表现，
        // 日志里能看见它，就等于看见规则有没有生效（截图之外的第二条证据）。
        // 放在早退守卫**之前**：守卫在「什么都变」时会直接返回，正是那种时刻才最需要证据。
        let zone_note = self.drop.note(self.zone());
        if self.last_zone_note.as_deref() != Some(zone_note.as_str()) {
            log::info!("拖拽区：{zone_note}");
            self.last_zone_note = Some(zone_note);
        }
        // 设置页选的新目录：**必须在早退守卫之前取出来**。
        // 否则「只选了个目录、别的什么都没变」时守卫直接返回，这一取永远轮不到
        // ——用户看到的就是「选了新目录，界面不刷新」。
        let picked_dir: Option<std::path::PathBuf> = {
            let mut guard = self.picked_dir.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        };
        if fresh == self.devices
            && service_on == self.service_on
            && !transfer_changed
            && !incoming_changed
            && !outgoing_changed
            && !pair_changed
            && !pending_changed
            && !trusts_changed
            && !receive_changed
            && picked_dir.is_none()
        {
            return false;
        }
        let mut changed = transfer_changed || pair_changed || pending_changed;
        if trusts_changed {
            self.trusts_list = trusts_next;
            changed = true;
        }
        if receive_changed {
            // 服务没在跑时守护进程报不出目录：不能拿 `None` 把刚选的目录冲掉，
            // 退回本机设置（服务在跑时仍以守护进程为事实来源）。
            self.receive_dir =
                settings_model::effective_receive_dir(receive_next, self.settings.receive_dir.as_deref());
            changed = true;
        }
        // 发给守护进程（下一连接生效）+ 记进设置（下次拉起带 `--dir`）
        if let Some(dir) = picked_dir {
            let text = dir.display().to_string();
            if let Some(receive) = self.receive.clone() {
                if let Err(e) = receive.set_receive_dir(&text) {
                    log::info!("更换接收目录失败：{e}");
                }
            }
            let old = self.settings.receive_dir.clone();
            self.receive_dir = Some(text.clone());
            self.change_settings(&format!("接收目录 → {text}"), |s| s.receive_dir = Some(text.clone()));
            if old != self.settings.receive_dir {
                changed = true;
            }
        }
        if pair_changed {
            self.pair = next_pair;
        }
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

        // 接收侧的请求跟着 `PAIRPENDING` 走：**码不在里面**（它只存在于对端屏幕上），
        // 这里只有「谁在问、还剩几次机会」。
        if self.pairing.is_some() {
            match (incoming, self.pair_request.clone()) {
                (Some(row), Some(card)) if row.token == card.token => {
                    let mut fresh = card;
                    // 剩余次数变少 = 上一次输入不对（由对端判定，界面不自己判对错）
                    if row.remaining < fresh.remaining {
                        fresh.wrong = true;
                        fresh.allowed = true;
                        log::info!(
                            "配对 #{0}：校验码不符，还剩 {1} 次",
                            row.token,
                            row.remaining
                        );
                    }
                    fresh.remaining = row.remaining;
                    let changed_here = fresh != self.pair_request.clone().unwrap();
                    if changed_here {
                        self.pair_request = Some(fresh);
                        changed = true;
                    }
                }
                (Some(row), _) => {
                    log::info!("收到配对请求 #{}：来自「{}」", row.token, row.peer);
                    self.code_input.clear();
                    self.code_focused = false;
                    // 新请求一律从「询问段」开始（码还没在对方屏幕上出现）
                    self.pair_request = Some(paircard::RequestCard {
                        token: row.token,
                        peer: row.peer,
                        remaining: row.remaining,
                        allowed: false,
                        wrong: false,
                    });
                    changed = true;
                }
                // 请求没了（对方放弃 / 超时 / 已落定）：收起卡片
                (None, Some(_)) => {
                    self.pair_request = None;
                    self.code_input.clear();
                    self.code_focused = false;
                    changed = true;
                }
                (None, None) => {}
            }
        }

        if pending_changed {
            if let Some(row) = pending_next.as_ref() {
                log::info!(
                    "待确认 #{0}：来自「{1}」的 {2}（校验码 {3}）",
                    row.token,
                    row.peer,
                    row.name,
                    row.code
                );
            }
            self.pending = pending_next;
            self.pending_total = pending_rows.len();
        }

        // 后台线程交回来的东西：先取走（只有它们会让界面有反应）
        let picked: Vec<std::path::PathBuf> = {
            let mut guard = self.picked.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        };
        if !picked.is_empty() {
            changed = true;
            self.accept_files(picked);
        }
        let errors: Vec<String> = {
            let mut guard = self.send_errors.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        };
        if !errors.is_empty() {
            changed = true;
            self.deny_drop("发送没能开始");
            for note in errors {
                log::info!("发送失败：{note}");
            }
        }
        // 拒收态到时自动收回（设计稿 1.8s）——**不靠动画自己复位**，
        // 因为要复位的是状态而不只是那一下抖动
        if let Some(at) = self.deny_until {
            if std::time::Instant::now() >= at {
                self.deny_until = None;
                if self.drop.is_denied() {
                    self.drop = DropState::Idle;
                    changed = true;
                }
            }
        }
        // 文件不再悬在窗口上：高亮自己退掉（离开窗口没有回调可听）
        if let Some(at) = self.over_until {
            if std::time::Instant::now() >= at && self.drop == DropState::Over {
                self.over_until = None;
                self.drop = DropState::Idle;
                changed = true;
            }
        }
        // 设备下线：**连接一起摘掉**——留着一台已死设备在环上，它既是假的「已连接」，
        // 又会被当成可发目标（拖拽区写着「拖到此处 → 它」，其实根本发不出去）
        let alive: Vec<String> = self.devices.iter().map(|d| d.id.clone()).collect();
        let gone: Vec<String> = self
            .connections
            .iter()
            .map(|c| c.id.clone())
            .filter(|id| !alive.contains(id))
            .collect();
        for id in gone {
            self.disconnect(&id);
            changed = true;
        }
        // 断开的名单只对**仍在附近**的设备有意义：设备下线就清掉——
        // 下次再发现它是一次新的相遇，该提示的还得提示
        self.declined.retain(|id| self.devices.iter().any(|d| &d.id == id));
        // 选中的设备下线了：清掉选择（拖拽区文案随之回到「先选择一台设备」）
        if let Some(id) = self.selected.clone() {
            if !self.devices.iter().any(|device| device.id == id) {
                log::info!("选中的设备已下线：{id}");
                self.selected = None;
                changed = true;
            }
        }
        // 局面（服务状态 + 有没有设备）一变就重新计时：那是一次新的搜索/新的等待，
        // 不是上一个局面的延续——不重置的话「刚打开服务」会立刻显示上一个局面的结论
        let situation = (self.service_state(), !self.devices.is_empty());
        if self.last_situation != Some(situation) {
            self.last_situation = Some(situation);
            self.note_since = std::time::Instant::now();
            changed = true;
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
        // 主题：用户挑的档位说了算；「跟随系统」每次渲染现问窗口
        // （系统外观一变 gpui 会触发重绘，50ms 的界面时钟也会把它带出来）。
        let light = match self.settings.theme {
            ThemeChoice::System => matches!(
                cx.window_appearance(),
                WindowAppearance::Light | WindowAppearance::VibrantLight
            ),
            ThemeChoice::Light => true,
            ThemeChoice::Dark => false,
        };
        let t = Theme::pick(light);
        // 动画回调要求 `'static`：捕获的是这个「按值的一份」，不是 `t` 的借用
        let t_theme = t;
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

        // 齿轮 = 设置面板的入口。开着时墨色提亮：与盾牌同一套「当前在哪」的提示。
        let cx_handle_settings = cx.entity().clone();
        let settings_btn = titlebar::icon_button(
            "settings",
            &t,
            "icons/settings.svg",
            if self.settings_open { t.text_primary } else { t.text_secondary },
            None,
            theme::IconHover::Neutral,
            move |_, _, cx| {
                let _ = cx_handle_settings.update(cx, |root, cx| {
                    root.toggle_settings();
                    cx.notify();
                });
            },
        );

        // 标题栏右侧：设置 · 隐私浮层 · 收进托盘 · 关闭。
        // 关闭是**自绘的窗口控制**（系统红绿灯已经藏掉，见 `crate::platform`）。
        // 盾牌 = 隐私浮层的入口（"我们的特色"就在浮层里：无云端 / 无中转 / 无账号 / 无日志）。
        // 开着时墨色提亮：这是"当前打开的是哪一颗"的唯一提示（不加新颜色）。
        let privacy = titlebar::icon_button(
            "privacy",
            &t,
            "icons/shield.svg",
            if self.privacy_open { t.text_primary } else { t.text_secondary },
            None,
            theme::IconHover::Neutral,
            move |_, _, cx| {
                let _ = cx_handle.update(cx, |root, cx| {
                    root.toggle_privacy();
                    cx.notify();
                });
            },
        );
        // 点外面关浮层：`on_mouse_down` 在最外层，子元素点一下也会冒泡上来。
        // 所以**盾牌自己要把事件拦住**（见下面的 wrapper），否则「再点一次盾牌」会先被
        // 这里关掉、再被按钮打开，看着像没反应。
        let privacy_guard = div()
            .absolute()
            .inset_0()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, _| {
                this.close_privacy();
            }));
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
                let select_id = device.id.clone();
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
                // 单击 = 选中它（设计稿的 `select()`：单选、再点取消）。
                // 与拖动共存在同一个元素上：位移一大就变成拖动（gpui 自己分流）。
                .on_click(cx.listener(move |this, _, _, cx| {
                    // 单击 = 选中（只对**已连接**的设备有效）；
                    // 没连的设备点它不选中，而是把「先连接」这句摆出来——
                    // 默默无视一次点击，用户只会以为界面卡了。
                    if this.is_connected(&select_id) {
                        this.toggle_selected(&select_id);
                    } else {
                        this.deny_drop(drop::CONNECT_FIRST);
                    }
                    cx.notify();
                }))
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

        // 传输卡要用的数字：**在渲染前算好**（渲染层不做判断，也不每帧重算速度）
        let progress_fraction = self.progress.x() as f64;
        let transfer_card = self.transfer.clone().map(|row| {
            let rows: Vec<TransferRow> = self
                .transfers
                .as_ref()
                .map(|source| source.transfers())
                .unwrap_or_default();
            let mut numbers = transfer::numbers(&row, self.transfer_speed);
            let label = self.batch.as_ref().and_then(|batch| batch.label(&rows));
            // 整批传完：元信息写「这批多大」，而不是最后那一个文件
            if let Some(batch) = self.batch.as_ref() {
                if batch.is_finished(&rows) {
                    numbers.meta = format!(
                        "{} 个文件 · 共 {}",
                        batch.total(),
                        crate::model::format::human_size(batch.total_bytes(&rows))
                    );
                }
            }
            (row, numbers, label)
        });

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
            // ── 文件拖放（与设备拖拽并存：gpui 按**拖拽值的类型**分发） ──
            //
            // 命中区是**整窗**（不只是那条 44px 的框）：拖拽区是「提示与状态」，
            // 不是一个小按钮——从 Finder 拖过来时，任何人都会直接往窗口上扔。
            .on_drag_move(cx.listener(
                |this, _event: &DragMoveEvent<gpui::ExternalPaths>, _window, cx| {
                    // 悬停高亮：等用户真把文件拖进来才亮（提前亮＝预判他要做什么）
                    this.over_until =
                        Some(std::time::Instant::now() + OVER_HOLD);
                    if !this.drop.is_denied() {
                        this.drop = DropState::Over;
                    }
                    cx.notify();
                },
            ))
            .on_drop(cx.listener(
                |this, paths: &gpui::ExternalPaths, _window, cx| {
                    let files = paths.paths().to_vec();
                    this.accept_files(files);
                    cx.notify();
                },
            ))
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
                            .child(settings_btn)
                            // 拦住鼠标按下：否则「再点一次盾牌」会被外层的关闭先吃掉
                            .child(
                                div()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .child(privacy),
                            )
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
                    // 空态：只有自机 + spinner + 「正在搜索附近设备…」（设计稿 `.empty-note`）
                    //
                    // **必须画在图区里**：设计稿的 `.empty-note { top: 268px }` 是相对
                    // `.graph` 的；画到根节点上就少了标题栏那 44px，
                    // 提示会糊到盘底下、把机器名挤到提示下面去（截图上一眼能看出来）。
                    .child(spinner::empty_note(&t, self.empty_note(), self.spin))
                    // 中心盘：和设备**同一层**（物理世界保证不重叠：它是钉死的不动体，
                    // 会把压上来的气泡挤开）。位置钉在圆心，缩放也不挪中心。
                    .child({
                        // 空态显不显示本机名字（设置页可关）：设备在图上时名字始终显示
                        let name = if self.devices.is_empty()
                            && !self.settings.self_name_in_empty
                        {
                            String::new()
                        } else {
                            self.device_name.to_string()
                        };
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
                                label: &name,
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
                                label: &name,
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
            .child(dropzone::dropzone(
                &t,
                self.drop,
                self.zone(),
                cx.listener(|this, _, _, cx| {
                    this.pick_files();
                    cx.notify();
                }),
            ))
            // 浮层卡（配对 / 传输）画在拖拽区**之上**（层级：拖拽区 < 卡 < 隐私浮层）：
            // 两张卡占**同一个槽位**（同一个 `view::card` 外壳），所以同时出现时
            // 配对卡优先——配对是等人拍板的事，传输卡让它一步回去（「一次只强调一件事」）。
            // 槽位只有一个，优先级：**等人的请求**（接收侧）> 出示码（发起侧）> 传输卡
            // 槽位优先级：**等人拍板的**（接收确认 / 配对请求）> 出示码 > 状态报告（传输卡）
            .when(self.pair_request.is_none() && self.pending.is_none(), |el| {
                el.when(self.pair.is_none(), |el| {
                el.when_some(transfer_card, |el, (row, numbers, label)| {
                    let fraction = progress_fraction;
                    el.child(progress::card(
                        &t,
                        &row,
                        &numbers,
                        fraction,
                        label.as_deref(),
                        cx,
                    ))
                })
                })
            })
            // 接收侧待确认：别人在等你拍板，优先级仅次于配对请求（都要人动手）
            .when_some(self.pending.clone(), |el, pending| {
                let total = self.pending_total;
                el.child(pending_card::card(&t, &pending, total, cx))
            })
            .when_some(self.pair_request.clone(), |el, request| {
                el.child(paircard::request_card(
                    &t,
                    &request,
                    &self.code_input,
                    &self.code_focus,
                    cx,
                ))
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
            // 设置面板：盖在浮层卡之上（它在「管理」这张窗），在隐私浮层之下。
            // Esc 要能关，就得先有焦点——与隐私浮层同一套做法（只交一次）。
            .when(self.settings_open, |el| {
                let trusts = self.trusts_list.clone();
                let devices = self.devices.clone();
                let receive_dir = self.receive_dir.clone();
                let confirm_del = self.confirm_delete.clone();
                let settings = self.settings.clone();
                let rename = self
                    .rename
                    .as_ref()
                    .map(|session| (session.fingerprint.clone(), session.field.clone()));
                let rename_ctx = settings_panel::RenameCtx {
                    entity: cx.entity().clone(),
                    focus: self.rename_focus_handle(),
                    bounds: self.rename_bounds_slot(),
                };
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .track_focus(&self.settings_focus)
                        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                            if event.keystroke.key.as_str() == "escape" {
                                this.close_settings();
                                cx.notify();
                            }
                        }))
                        .child(settings_panel::panel(
                            &t,
                            &settings,
                            &trusts,
                            &devices,
                            receive_dir.as_deref(),
                            confirm_del.as_deref(),
                            rename.as_ref().map(|(fingerprint, field)| {
                                (fingerprint.as_str(), field)
                            }),
                            &rename_ctx,
                            self.settings_progress(),
                            self.seg_position(settings.theme.index()),
                            cx,
                        )),
                )
            })
            // 隐私浮层：层级最上（拖拽区 < 浮层卡 < 设置面板 < 隐私浮层）。它是**只读的说明**，
            // 不参与任何流程，压住谁也不影响操作。
            .when(self.privacy_open, |el| {
                el.child(
                    div()
                        .absolute()
                        .inset_0()
                        .track_focus(&self.privacy_focus)
                        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                            if event.keystroke.key.as_str() == "escape" {
                                this.close_privacy();
                                cx.notify();
                            }
                        }))
                        .child(crate::view::privacy::popover(&t, self.privacy_progress())),
                )
            })
            .child(titlebar::window_border(&t))
            // 点外面关：透明、只在浮层开着时才挂上（不然它会挡住所有点击）
            .when(self.privacy_open, |el| el.child(privacy_guard))
    }
}

/// 输入处理：把改名输入框接进系统文本栈（IME 靠它，普通字符也走它）。
///
/// 契约（gpui `EntityInputHandler`）：区间一律 **UTF-16 下标**；换算收口在
/// `model::namefield`。没有改名会话时全部如实返回 `None` / 不接受——
/// 输入处理器是「当前焦点元素」的，我们只有这一个输入场景，用它当开关刚好。
impl EntityInputHandler for AppRoot {
    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.rename.as_ref()?.field.text();
        let start = char_index_from_utf16(text, range.start);
        let end = char_index_from_utf16(text, range.end);
        let selected: String = text.chars().skip(start).take(end - start).collect();
        *adjusted_range = Some(range.start..range.start + selected.encode_utf16().count());
        Some(selected)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let session = self.rename.as_ref()?;
        let caret = utf16_index_from_char(session.field.text(), session.field.caret());
        // 不做选区：光标只是一个点
        Some(UTF16Selection { range: caret..caret, reversed: false })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        let session = self.rename.as_ref()?;
        let (start, end) = session.field.marked()?;
        let text = session.field.text();
        Some(utf16_index_from_char(text, start)..utf16_index_from_char(text, end))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if let Some(session) = self.rename.as_mut() {
            session.field.unmark();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.rename.as_mut() {
            session.field.replace_utf16_range(range, new_text);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.rename.as_mut() {
            session.field.replace_and_mark_utf16(range, new_text);
        }
    }

    fn bounds_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // 输入框的位置在 paint 时由 canvas 记下（见 `view::settings` 的改名输入框）
        self.rename_bounds.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        // 点击定位光标（v1 不做）：如实告诉平台「我不知道」，别瞎猜一个位置
        None
    }

    fn text_length_utf16(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        Some(self.rename.as_ref()?.field.text().encode_utf16().count())
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.rename.is_some()
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
    trusts: Option<Arc<dyn TrustSource>>,
    receive: Option<Arc<dyn ReceiveDirSource>>,
    settings: Settings,
    // 预览专用（`GLOWLINK_PREVIEW=linked`）：开窗就连上第一台，并把服务当「在跑」——
    // 否则拖拽区永远只显示「服务已关闭」，看不到设备那几句文案。
    // **真实路径永远是 `false`**：连接只能由用户拖出来，服务状态只能看真实进程。
    preview_linked: bool,
    // 预览专用：一开窗就打开隐私浮层（`GLOWLINK_PREVIEW=privacy`）
    preview_privacy: bool,
    // 预览专用：一开窗就打开设置面板（`GLOWLINK_PREVIEW=settings`）
    preview_settings: bool,
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
        let view = cx.new(|cx| {
            let mut root = AppRoot::new(
                cx,
                device_name,
                source,
                service,
                pairing,
                transfers,
                trusts,
                receive,
                settings,
            );
            root.preview_linked = preview_linked;
            if preview_linked {
                root.connect_first();
            }
            if preview_privacy {
                root.toggle_privacy();
            }
            if preview_settings {
                root.toggle_settings();
            }
            root
        });
        // 快照轮询：**比较**在界面线程（只读内存），真正的 IPC 在后台线程。
        // 窗口关了 `update` 会失败，循环自己退出（不必额外取消机制）。
        let weak = view.downgrade();
        // 用 async 闭包（rustc 1.85+ 的 `async move |cx| …`）：普通闭包返回的
        // future 会借用 `&mut AsyncApp`，而 `spawn` 要求 future 是 `'static`。
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(SYNC_INTERVAL).await;
                let alive = weak.update(cx, |root, cx| {
                    let changed = root.sync();
                    // 输入段一出现就把键盘焦点交过去（**只交一次**）：
                    // 不交的话用户敲数字毫无反应，而界面上看不出缺了什么。
                    // 焦点要在有窗口的地方拿，所以借用窗口句柄走一趟。
                    if root.wants_privacy_focus() {
                        let handle = root.privacy_focus.clone();
                        if let Some(window) = cx.windows().first() {
                            if window
                                .update(cx, |_, window, cx| window.focus(&handle, cx))
                                .is_ok()
                            {
                                root.privacy_focused = true;
                            }
                        }
                    }
                    if root.wants_rename_focus() {
                        let handle = root.rename_focus.clone();
                        if let Some(window) = cx.windows().first() {
                            if window
                                .update(cx, |_, window, cx| window.focus(&handle, cx))
                                .is_ok()
                            {
                                root.rename_focused = true;
                            }
                        }
                    }
                    if root.wants_settings_focus() {
                        let handle = root.settings_focus.clone();
                        if let Some(window) = cx.windows().first() {
                            if window
                                .update(cx, |_, window, cx| window.focus(&handle, cx))
                                .is_ok()
                            {
                                root.settings_focused = true;
                            }
                        }
                    }
                    if root.wants_code_focus() {
                        let handle = root.code_focus.clone();
                        if let Some(window) = cx.windows().first() {
                            if window
                                .update(cx, |_, window, cx| window.focus(&handle, cx))
                                .is_ok()
                            {
                                root.code_focused = true;
                            }
                        }
                    }
                    if changed {
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
