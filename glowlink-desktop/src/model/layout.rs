//! Model：节点图的几何与命中判定。
//!
//! **零 IO、零 GPUI**：全是纯函数，所以能用普通单测钉住——
//! 而渲染层是测不动的（GPUI 要真实窗口），座标一旦写错只能靠肉眼发现。
//! 设计稿把坐标写成两套基准（CSS 里 `top: 42px` 是「节点图内」，
//! 加 44px 标题栏才是全窗），这里只认**全窗**一套，杜绝二次换算。

use crate::theme;

/// 一个设备节点的圆心（全窗坐标）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// 节点直径。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeSize {
    /// 本机中心盘 84px。
    SelfNode,
    /// 设备盘 52px。
    Device,
}

impl NodeSize {
    pub fn diameter(self) -> f32 {
        match self {
            NodeSize::SelfNode => theme::NODE_SELF_SIZE,
            NodeSize::Device => theme::NODE_DEVICE_SIZE,
        }
    }
}

/// 本机中心节点。
pub fn self_center() -> Point {
    Point { x: theme::SELF_CENTER.0, y: theme::SELF_CENTER.1 }
}

/// 设备节点的散布规则：**位置不固定**，每台设备在自己的一条环带上占一个位置。
///
/// 要的是「神经元」那种感觉：一圈漂着的亮点，而不是按模板摆好的三格。
/// 但随机必须是有纪律的，三条约束不能丢：
/// 1. **稳定**：位置由设备 ID 派生，不随帧、不随列表顺序变（每帧乱跳就不是呼吸，是瘫搐）；
/// 2. **不重叠**：两圆心距离至少 `MIN_DIST`，靠排斥迭代推开；
/// 3. **不出图区**：盘与标签都留在节点图内。

/// 本机盘半径（84/2）。
pub const SELF_RADIUS: f32 = 42.0;
/// 设备盘半径（52/2）。
pub const DEVICE_RADIUS: f32 = 26.0;
/// 两枚节点之间留的缝（圆心距 = 双方半径之和 + 这个缝）。
pub const GAP: f32 = 8.0;
/// 与中心盘的净空：本机盘 + 设备盘 + 更宽的缝（24）——中心盘是画面主角，别贴太紧。
pub const CENTER_CLEAR: f32 = SELF_RADIUS + DEVICE_RADIUS + 24.0;
/// 两枚**常规**设备的圆心最小距离（= 52 + 8 = 60）。
pub const MIN_DIST: f32 = 2.0 * DEVICE_RADIUS + GAP;

/// 一个「心里想待的位置」：从这儿开始找位置（但仍会被排斥推开）。
///
/// 用途：断开连接后就地留下的那台——它不回原位，可是也不能压在中心盘或别人身上，
/// 所以给它一个起点，让排斥迭代把它推到最近的合法位置。
#[derive(Clone, Debug, PartialEq)]
pub struct Seed {
    pub id: String,
    pub point: Point,
}

/// 已被「钉住」的节点：位置固定，别人绕着它排。
///
/// 用途：拖到中心连上的那台，位置由吸附规则决定并保留下来（断开也不回原位）。
/// 它带着自己的**半径**——已连接的盘被缩小了，排斥计算不能拿常规半径去量它。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pinned {
    pub point: Point,
    pub radius: f32,
}
/// 种子点最远的生成半径（最终还要受窗口边界约束）。
const SPAWN_MAX: f32 = 200.0;
/// 设备名的最大半宽（10px 字号下约 8 个汉字 / 16 个字母）：名字盒比盘宽得多，
/// 按盘来留边距的话长名字会横穿出窗口。
const LABEL_HALF: f32 = 44.0;
/// 盘下方留给名字的高度：6px 间隙 + 12px 字高。
const LABEL_BELOW: f32 = 18.0;
/// 漂浮的纵向幅度（与 `view::node` 的 `FLOAT_AMP_Y` 同源思路，取整一点）。
const DRIFT_MARGIN: f32 = 4.0;

/// 设备盘圆心允许的范围（**全窗坐标**）。
///
/// 上下左右都得留够，否则节点会「胖」到隔壁去：
/// - 上：标题栏底边（44）之下，盘不能顶进标题栏
/// - 下：节点图底边（44 + 396 = 440）之上，**盘与名字都不能伸进拖拽区**
///   （曾经的 bug：中心 y 可以到 396，盘底 + 名字 + 漂浮直接压到拖拽框上）
/// - 左右：留出名字的半宽，长名字不会横穿出窗口
fn bound_x() -> (f32, f32) {
    let half = theme::NODE_DEVICE_SIZE / 2.0;
    let margin = LABEL_HALF.max(half) + DRIFT_MARGIN;
    (margin, theme::WINDOW_W - margin)
}

fn bound_y() -> (f32, f32) {
    let half = theme::NODE_DEVICE_SIZE / 2.0;
    let top = theme::TITLEBAR_H + half + DRIFT_MARGIN;
    let graph_bottom = theme::TITLEBAR_H + theme::GRAPH_H;
    let bottom = graph_bottom - half - LABEL_BELOW - DRIFT_MARGIN;
    (top, bottom)
}
/// 排斥迭代次数：几百次对十几枚节点仍是瞬时（只在设备集合变化时算一次），
/// 但必须**定死次数**才可复现（不做「收敛了才停」，那会让不同设备数跑出不同精度）。
const RELAX_STEPS: usize = 400;

/// 一次布局的结果（与输入顺序一一对应）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    pub centers: Vec<Point>,
    /// 漂浮相位（0..1）：与位置同源派生，同样与顺序无关。
    pub phases: Vec<f32>,
}

/// 给一组设备 ID 算位置。
///
/// **与输入顺序无关**（内部先按 ID 排序再算，最后映回原顺序）：
/// 发现层返回设备表的顺序可能变（谁先应答），顺序一变位置就整体挪一下，看起来像全体抽搐。
pub fn place_devices(ids: &[&str], pinned: &[Pinned], seeds: &[Seed]) -> Placement {
    let mut sorted: Vec<&str> = ids.to_vec();
    sorted.sort_unstable();

    let mut points: Vec<Point> = sorted
        .iter()
        .map(|id| {
            seeds
                .iter()
                .find(|seed| seed.id == *id)
                .map(|seed| seed.point)
                .unwrap_or_else(|| seeded_point(id))
        })
        .collect();
    relax(&mut points, pinned);

    // 映回调用者的顺序
    let mut centers = vec![Point { x: 0.0, y: 0.0 }; ids.len()];
    let mut phases = vec![0.0; ids.len()];
    for (index, id) in ids.iter().enumerate() {
        let slot = sorted.binary_search(id).expect("id 来自同一份列表");
        centers[index] = points[slot];
        phases[index] = seeded_phase(id);
    }
    Placement { centers, phases }
}

/// 只要「不重叠」的位置（没有钉子与种子时用）。
pub fn place_devices_free(ids: &[&str]) -> Placement {
    place_devices(ids, &[], &[])
}

/// ID → 一个看起来随机的落点（在中心盘净空之外、窗口之内）。
fn seeded_point(id: &str) -> Point {
    let seed = fnv1a(id);
    let angle = (mix(seed) as f64 / u64::MAX as f64) as f32 * std::f32::consts::TAU;
    // 半径从净空一直散到远处：**不是一个固定半径的环**，而是一圈里疏密不匀的散点，
    // 这才是「神经元」而不是「行星轨道」（半径一样会让画面看得出模板）。
    let ratio = (mix(seed ^ 0x51ed_2701_9b4d_2c0f) as f64 / u64::MAX as f64) as f32;
    let radius = CENTER_CLEAR + ratio * (SPAWN_MAX - CENTER_CLEAR);
    let center = self_center();
    Point { x: center.x + radius * angle.cos(), y: center.y + radius * angle.sin() }
}

/// ID → 漂浮相位（与位置同源：同一台设备的「轨迹」保持一致，重启也不变）。
fn seeded_phase(id: &str) -> f32 {
    (mix(fnv1a(id) ^ 0x9e37_79b9_7f4a_7c15) as u32 as f32) / u32::MAX as f32
}

/// 排斥迭代：两两推开 + 卡回净空与边界。
///
/// 纯几何、无随机数：同一输入永远得到同一输出（测试才能钉住）。
fn relax(points: &mut [Point], pinned: &[Pinned]) {
    let center = self_center();
    // 钉子（已连接、位置固定）在最前面，活节点从它后面开始，避免互相踩
    for _ in 0..RELAX_STEPS {
        for i in 0..points.len() {
            // 与钉子：只推活节点，钉子一动不动
            for fixed in pinned {
                let (a, b) = (points[i], fixed.point);
                let need = DEVICE_RADIUS + fixed.radius + GAP;
                let push = separation(a, b, need);
                if let Some((nx, ny, amount)) = push {
                    points[i] = Point { x: a.x - nx * amount, y: a.y - ny * amount };
                }
            }
            for j in (i + 1)..points.len() {
                let (a, b) = (points[i], points[j]);
                if let Some((nx, ny, amount)) = separation(a, b, MIN_DIST) {
                    // 各让一半：只有一边动的话，活节点会被钉子挤成一条线
                    points[i] = Point { x: a.x - nx * amount / 2.0, y: a.y - ny * amount / 2.0 };
                    points[j] = Point { x: b.x + nx * amount / 2.0, y: b.y + ny * amount / 2.0 };
                }
            }
        }
        for point in points.iter_mut() {
            let (dx, dy) = (point.x - center.x, point.y - center.y);
            let d = (dx * dx + dy * dy).sqrt();
            // 太靠近中心盘会压住本机
            if d < CENTER_CLEAR {
                let (nx, ny) = if d < 0.001 { (1.0, 0.0) } else { (dx / d, dy / d) };
                *point = Point { x: center.x + nx * CENTER_CLEAR, y: center.y + ny * CENTER_CLEAR };
            }
            // 最外一圈靠边界夹住（窗口只有 320×500，没有“再往外”的余地）
            let (x0, x1) = bound_x();
            let (y0, y1) = bound_y();
            point.x = point.x.clamp(x0, x1);
            point.y = point.y.clamp(y0, y1);
        }
    }
}

/// 两点若比 `need` 更近，返回「推开方向 + 还差多少」。
fn separation(a: Point, b: Point, need: f32) -> Option<(f32, f32, f32)> {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let d = (dx * dx + dy * dy).sqrt();
    if d >= need {
        return None;
    }
    // 完全重合时给一个确定的方向，否则除零就飞了
    let (nx, ny) = if d < 0.001 { (1.0, 0.0) } else { (dx / d, dy / d) };
    Some((nx, ny, need - d))
}

/// splitmix64 的收尾混合。
///
/// 直接用 FNV 的高位当坐标会撞车（实测两台设备的相位一模一样，因为短字符串
/// 只有末位不同时高位相关性很强）。先把位搅匀再用，三个用途之间再用不同常数错开。
fn mix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// FNV-1a：不引依赖的稳定散列（`DefaultHasher` 不保证跳版本后不变）。
fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 两点距离的平方（比较远近时不必开方）。
fn dist2(a: Point, b: Point) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

/// 某个点是否落在吸附判定区里（把设备拖到本机上就算）。
///
/// 半径取 `SNAP_RADIUS`（50px，任务书 v1.0），比中心盘半径 42 略宽松：
/// 盘缘再往外一点也算吸附，手不必抖到极致。
///
/// **判定只在松手那一刻做**：拖动过程中只做视觉高亮。
/// 拖动中实时判定会出现「吸附近似触发」的抖动——坐标与动画帧不同步，看着像卡了一下。
pub fn snaps_to_center(point: Point) -> bool {
    let radius = crate::model::drag::SNAP_RADIUS;
    dist2(point, self_center()) <= radius * radius
}

/// 只把**真正重叠**的节点推开（局部、不重新求解全局布局）。
///
/// 拖动时用：被拖的那枚是不可推动的障碍物，只有它**真的压到**谁才推谁，
/// 推开量恰好等于重叠量（不留额外的缝——「碰到」就是半径相切，不能提前预警）。
///
/// 为什么不把拖影塞进 [`relax`]：那是**全局重新求解平衡点**，
/// 会让远处的节点也一起挪（用户真机看到的现象：「都没遇到就碰，而且碰还都碰」）。
/// 这里只做局部的接触响应，输入连续、输出也连续。
///
/// 迭代几轮是为了把「被推的又挤到别人」传下去（链条），仍是从当前坐标接着算。
pub fn resolve_contacts(
    rest: &[Point],
    radius: f32,
    obstacles: &[(Point, f32)],
) -> Vec<Point> {
    const PASSES: usize = 3;
    let mut points = rest.to_vec();
    for _ in 0..PASSES {
        for point in points.iter_mut() {
            for (obstacle, obstacle_radius) in obstacles {
                let need = radius + obstacle_radius;
                if let Some((nx, ny, amount)) = separation(*point, *obstacle, need) {
                    // 障碍物（指针拖着的那个）推不动：位移全给节点
                    *point = Point { x: point.x - nx * amount, y: point.y - ny * amount };
                }
            }
        }
        // 被推的节点之间也要说实话：各让一半
        for i in 0..points.len() {
            for j in (i + 1)..points.len() {
                let need = radius * 2.0;
                if let Some((nx, ny, amount)) = separation(points[i], points[j], need) {
                    points[i] = Point { x: points[i].x - nx * amount / 2.0, y: points[i].y - ny * amount / 2.0 };
                    points[j] = Point { x: points[j].x + nx * amount / 2.0, y: points[j].y + ny * amount / 2.0 };
                }
            }
        }
        // 推出去的不能出图区（边界靠 [橡皮筋] 留给下一步，这里先保证不越界）
        let (x0, x1) = bound_x();
        let (y0, y1) = bound_y();
        for point in points.iter_mut() {
            point.x = point.x.clamp(x0, x1);
            point.y = point.y.clamp(y0, y1);
        }
    }
    points
}

/// 两点距离（px）。
pub fn distance(a: Point, b: Point) -> f32 {
    dist2(a, b).sqrt()
}

/// 到中心盘圆心的距离（px）。日志与调试用——数字比「吸/不吸」更能说明问题。
pub fn distance_to_center(point: Point) -> f32 {
    dist2(point, self_center()).sqrt()
}

/// 拖动时该吸附到哪台设备（就近吸附，超过半径就松手回弹）。
///
/// `centers` 是当前布局（由 [`place_devices`] 算得），`radius` 由调用方给
/// （界面用盘直径），返回被吸附的下标；都不在范围内返回 `None` ⇒ 回弹到原位。
pub fn snap_to_device(point: Point, centers: &[Point], radius: f32) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (index, center) in centers.iter().enumerate() {
        let d = dist2(point, *center);
        if d <= radius * radius && best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((index, d));
        }
    }
    best.map(|(index, _)| index)
}

/// 节点在窗口里的矩形（左上角 + 边长），供 GPUI 摆绝对定位。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

impl Rect {
    /// 由圆心与直径得到矩形（GPUI 用左上角定位，自己是中心定位）。
    pub fn from_center(center: Point, size: NodeSize) -> Self {
        let diameter = size.diameter();
        Rect {
            x: center.x - diameter / 2.0,
            y: center.y - diameter / 2.0,
            size: diameter,
        }
    }

    /// 设备标签的位置：盘下方 6px（设计稿 `labelOffset`）。
    pub fn label_y(&self) -> f32 {
        self.y + self.size + 6.0
    }
}
