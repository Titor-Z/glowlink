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

/// 松手时的方向：指针相对中心的角度（弧度，屏幕坐标系，y 向下）。
///
/// 指针正好落在中心上时给**正上方**（−90°）：方向无从可选，但总得给一个，
/// 而正上方是「最近的空位」最自然的一端。
pub fn angle_of(center: Point, at: Point) -> f32 {
    let (dx, dy) = (at.x - center.x, at.y - center.y);
    if dx.abs() < 0.001 && dy.abs() < 0.001 {
        return -std::f32::consts::FRAC_PI_2;
    }
    dy.atan2(dx)
}

/// 角度 → 连接环上的落点。
pub fn point_at(center: Point, angle: f32) -> Point {
    let r = ring_radius();
    Point { x: center.x + r * angle.cos(), y: center.y + r * angle.sin() }
}

/// 把一批角度分开到至少 `MIN_ANGLE_GAP`，保持相对顺序。
///
/// 全都拖到同一侧时不能让它们叠成一坨；也不能「均匀重排」——
/// 那会把用户记住的方位（「我把 iPad 拖到了右上」）抹掉。
/// 做法：从小到大扫一遍，每一台至少比前一台多一个间隔。
pub fn separate(angles: &[f32]) -> Vec<f32> {
    let mut sorted: Vec<f32> = angles.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let gap = min_angle_gap();
    for index in 1..sorted.len() {
        let floor = sorted[index - 1] + gap;
        if sorted[index] < floor {
            sorted[index] = floor;
        }
    }
    // 推开后可能整体转出了可视范围：整体平移回「以原均值为中心」
    if sorted.is_empty() {
        return sorted;
    }
    let before: f32 = angles.iter().sum::<f32>() / angles.len() as f32;
    let after: f32 = sorted.iter().sum::<f32>() / sorted.len() as f32;
    let shift = before - after;
    sorted.iter().map(|a| a + shift).collect()
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
