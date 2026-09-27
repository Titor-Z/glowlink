//! Model：把设备「拖到一起」之后的连接态。
//!
//! 这里只算**位置与比例**，不碰渲染也不碰网络。
//!
//! 术语上要说清一件事：我们没有常驻连接（配对 = 换信任，传文件另起一条 TCP）。
//! 所以这一层表达的是「这两台**已经认识**」，而不是「有一条活着的连接」——
//! 线的样式据此分档：认识 = 静止的淡虚线，正在传 = 流动的亮线。

use crate::model::layout::{self, Point};
use crate::theme;

/// 已连接的盘缩小多少。
///
/// 用户说「缩小 1/3」→ 取 ×2/3（本机 84→56、设备 52→35）。
/// 若是「缩小到 1/3」（84→28）改这一个数即可——但那样断开图标几乎和盘一样大。
pub const CONNECTED_SCALE: f32 = 2.0 / 3.0;
/// 连接环上的盘半径（设备盘缩小后）。
pub fn connected_radius() -> f32 {
    theme::NODE_DEVICE_SIZE / 2.0 * CONNECTED_SCALE
}
/// 连接环半径：两个缩小盘的半径之和 + 12px 缝。
/// 「拼合」就该是紧挨着——缝大了像吊在外面，缝小了会挤在一起。
pub fn ring_radius() -> f32 {
    theme::NODE_SELF_SIZE / 2.0 * CONNECTED_SCALE + connected_radius() + 12.0
}
/// 两台已连接设备之间的最小夹角（弧度）。
///
/// **必须从几何算**，不能拍一个数：环半径 57px 上 40° 的弦长只有 40px，
/// 而两个缩小盘要 2×17.3 + 8 = 42.7px 才不重叠 —— 拍 40° 的结果就是
/// 「都从下方拖进来时两个气泡叠在一起」（用户真机看到过）。
/// 再乘一点余量，免得贴在「刚好不重叠」的边上。
pub fn min_angle_gap() -> f32 {
    let need = 2.0 * connected_radius() + layout::GAP;
    (need / ring_radius()) * 1.15
}

/// 第一个槽位的方向：**正上方**（12 点，屏幕坐标系 y 向下）。
pub const FIRST_SLOT: f32 = -std::f32::consts::FRAC_PI_2;

/// `count` 台设备在环上的槽位角度：从正上方开始**顺时针**均匀分布。
///
/// 为什么不用「松手方向」：那是动作记忆，位置随每次拖动的方向变，用户
/// 记不住「我的 iPad 到底在哪边」；而且两台从同一侧拖进来时还会打架。
/// 现在**由中心点接管排布**：太阳在中间，卫星按顺序占位——位置完全确定，
/// 断开后压实重排（都走弹簧滑过去，看得见它去哪儿了）。
pub fn slot_angles(count: usize) -> Vec<f32> {
    let total = count.max(1) as f32;
    (0..count)
        .map(|index| FIRST_SLOT + std::f32::consts::TAU * index as f32 / total)
        .collect()
}

/// 环上最多能放几台（按几何算出的最小间隔）。
///
/// 超过这个数就只能硬塞，相邻两枚会轻微重叠——由物理约束推开（会略微离开环），
/// 不会叠成一坨。
pub fn max_slots() -> usize {
    (std::f32::consts::TAU / min_angle_gap()).floor() as usize
}

/// 角度 → 连接环上的落点。
pub fn point_at(center: Point, angle: f32) -> Point {
    let r = ring_radius();
    Point { x: center.x + r * angle.cos(), y: center.y + r * angle.sin() }
}

/// 一次连接：哪台设备、在环上的哪个角度。
#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub id: String,
    pub angle: f32,
}

/// 连接环上的节点也要参与排斥（别人绕着它排），所以包成 `layout::Pinned`。
pub fn pinned_points(connections: &[Connection]) -> Vec<layout::Pinned> {
    let center = layout::self_center();
    connections
        .iter()
        .map(|c| layout::Pinned { point: point_at(center, c.angle), radius: connected_radius() })
        .collect()
}

/// 一条连线的两个端点：两端各裁掉「盘的当前半径 + 一点余量」。
///
/// 收进 model 是为了**可测**（以前在 view 里，没人测过）。两个半径都取**当前值**
/// （本机盘与设备盘连接后都会缩小到 2/3）。
pub fn wire_ends(center: Point, end: Point, self_radius: f32, node_radius: f32) -> Option<(Point, Point)> {
    let (dx, dy) = (end.x - center.x, end.y - center.y);
    let length = (dx * dx + dy * dy).sqrt();
    let (start, stop) = (self_radius + 4.0, length - node_radius - 5.0);
    if stop <= start {
        return None;
    }
    let (nx, ny) = (dx / length, dy / length);
    Some((
        Point { x: center.x + nx * start, y: center.y + ny * start },
        Point { x: center.x + nx * stop, y: center.y + ny * stop },
    ))
}

/// 连线上「流动的亮线」的虚线段：`phase` 是 0..1 的相位（由界面时钟推进）。
///
/// **相位递增 = 朝 `to` 方向流**。收发方向因此可以用「换两端」表达：
/// 发送时 `from` 取本机（朝设备流出去），接收时把两端调过来（朝本机流进来）。
///
/// 为什么不直接用 `stroke-dasharray` + dashoffset：gpui 的 `PathBuilder` **没有 dashoffset**，
/// 虚线只能从路径起点开始排。所以这里自己算每段短线的位置——顺带也能单测。
///
/// 返回 `(起点, 终点)` 对，沿 from → to 方向排布。
pub fn flow_dashes(
    from: Point,
    to: Point,
    phase: f32,
    on: f32,
    off: f32,
) -> Vec<(Point, Point)> {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let length = (dx * dx + dy * dy).sqrt();
    let period = on + off;
    if length < 1.0 || period <= 0.0 {
        return Vec::new();
    }
    let (nx, ny) = (dx / length, dy / length);
    let point_at = |distance: f32| Point {
        x: from.x + nx * distance,
        y: from.y + ny * distance,
    };
    // 相位：相位递增整条线朝 to 走（所以这里取负号）；取模保证首尾无缝衔接
    let mut start = -((phase.rem_euclid(1.0)) * period) - on;
    let mut dashes = Vec::new();
    while start < length {
        let dash_start = start.max(0.0);
        let dash_end = (start + on).min(length);
        if dash_end > dash_start {
            dashes.push((point_at(dash_start), point_at(dash_end)));
        }
        start += period;
    }
    dashes
}
