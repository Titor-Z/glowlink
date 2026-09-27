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

/// 设备节点的**槽位**：设计稿只画了三台（手机/平板/台式机），
/// 多出来的设备按顺序占用第一个空槽；超过三个就叠在最后一个槽上并计数。
/// 这样「发现 5 台设备」不会把画面撑破——窗口是锁死 320×500 的。
pub const SLOTS: [(f32, f32); 3] = [
    theme::PHONE_CENTER,
    theme::TABLET_CENTER,
    theme::DESKTOP_CENTER,
];

/// 第 `index` 台设备的圆心。
///
/// 越界不回绕：宁可让第 4 台和最后一台重叠（配合界面上的「+N」提示），
/// 也不要让它跑到窗口外面去——看不见的设备比重叠更难理解。
pub fn device_center(index: usize) -> Point {
    let (x, y) = match SLOTS.get(index) {
        Some(slot) => *slot,
        None => SLOTS[SLOTS.len() - 1],
    };
    Point { x, y }
}

/// 两点距离的平方（比较远近时不必开方）。
fn dist2(a: Point, b: Point) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

/// 某个点是否落在中心盘的命中区里（拖动配对用）。
///
/// 命中区就是**中心盘本身**（84px），不含呼吸光圈：光圈是视觉层，
/// 把它算进命中会让「看着没放上去」也生效，手感变糊。
pub fn hits_center(point: Point) -> bool {
    let radius = theme::CENTER_HIT / 2.0;
    dist2(point, self_center()) <= radius * radius
}

/// 拖动时该吸附到哪台设备（就近吸附，超过半径就松手回弹）。
///
/// `radius` 由调用方给（界面用盘直径），返回被吸附的下标；
/// 都不在范围内返回 `None` ⇒ 回弹到原位。
pub fn snap_to_device(point: Point, devices: usize, radius: f32) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for index in 0..devices {
        let d = dist2(point, device_center(index));
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
