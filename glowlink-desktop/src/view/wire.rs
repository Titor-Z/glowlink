//! View：本机与已连接设备之间的连线。
//!
//! 设计稿的样式：`stroke-dasharray: 5 9`、两端**从盘的外缘裁掉**
//! （不裁的话虚线会横穿中心盘的图标）。gpui 这边用 `PathBuilder::stroke()`
//! 加 `dash_array()` 画——注意 gpui 没有 `rotate`，所以不能用「旋转的小横条」
//! 去拼虚线，必须走路径。
//!
//! 语义分档（重要）：**静止的淡虚线 = 已经认识**，**流动的亮线 = 正在传数据**。
//! 我们没有常驻连接，所以「认识」这条线不能流动——那会宣称有数据在跑。

use gpui::{canvas, point, px, IntoElement, PathBuilder, Rgba, Styled};

use crate::model::connect;
use crate::model::layout::{self, Point};
use crate::theme;

/// 线宽（设计稿 1.5）。
const WIRE_WIDTH: f32 = 1.5;
/// 虚线节奏（设计稿 `5 9`）。
const DASH: (f32, f32) = (5.0, 9.0);
/// 已连接（静止）的线有多淡。淡到「看得见，但不像有数据在跑」。
const CONNECTED_ALPHA: f32 = 0.34;
/// 流动亮线的节奏：短段 + 空隙（比静止虚线的 5/9 更密、更亮，读数就是「在跑」）。
const FLOW_ON: f32 = 7.0;
const FLOW_OFF: f32 = 11.0;

/// 一条连线：设备侧的端点 + 它的辉光色（线的颜色跟着那台设备走）+ 是否在传数据。
pub struct Segment {
    pub end: Point,
    pub tone: Rgba,
    /// 这条线上正在传数据吗（流动亮线 / 静止淡虚线）。
    pub flowing: bool,
    /// 流动方向：`true` = 往本机流（接收），`false` = 往设备流（发送）。
    pub inbound: bool,
}

/// 所有连接线（画在节点**之下**：设计稿的层级就是「连线 < 节点」）。
///
/// `flow` 是流动相位（0..1，由界面时钟推进）。只有 `flowing` 的线才排短段流动；
/// 其余仍是静止淡虚线——**静止 = 已经认识，流动 = 正在传**（我们没有常驻连接，
/// 让静止的线流动就是宣称有数据在跑）。
///
/// 坐标就是**图区坐标**（画布铺满节点图、原点在图区左上角），
/// 与 `layout::self_center()` / `Motion` 同一套，不再做任何换算。
pub fn wires(segments: Vec<Segment>, flow: f32) -> impl IntoElement {
    canvas(
        |_bounds, _window, _cx| {},
        move |_bounds, (), window, _cx| {
            let center = layout::self_center();
            let self_radius = theme::NODE_SELF_SIZE / 2.0 * connect::CONNECTED_SCALE;
            let node_radius = connect::connected_radius();
            for segment in &segments {
                let Some((from, to)) = connect::wire_ends(center, segment.end, self_radius, node_radius)
                else {
                    continue;
                };
                if segment.flowing {
                    // 流动的亮线：自己排短段（gpui 的路径**没有 dashoffset**，
                    // 虚线只能从起点开始排，所以流动必须自己算位置）。
                    // 方向：发送朝外、接收朝内（换个「起点」就换了流向）。
                    let (a, b) = if segment.inbound { (to, from) } else { (from, to) };
                    for (dash_from, dash_to) in connect::flow_dashes(a, b, flow, FLOW_ON, FLOW_OFF)
                    {
                        let mut builder = PathBuilder::stroke(px(WIRE_WIDTH));
                        builder.move_to(point(px(dash_from.x), px(dash_from.y)));
                        builder.line_to(point(px(dash_to.x), px(dash_to.y)));
                        if let Ok(path) = builder.build() {
                            window.paint_path(path, theme::hsla(segment.tone));
                        }
                    }
                    continue;
                }
                let mut builder =
                    PathBuilder::stroke(px(WIRE_WIDTH)).dash_array(&[px(DASH.0), px(DASH.1)]);
                builder.move_to(point(px(from.x), px(from.y)));
                builder.line_to(point(px(to.x), px(to.y)));
                if let Ok(path) = builder.build() {
                    let color = theme::hsla(with_alpha(segment.tone, CONNECTED_ALPHA));
                    window.paint_path(path, color);
                }
            }
        },
    )
    .absolute()
    .left(px(0.0))
    .top(px(0.0))
    .w_full()
    .h_full()
}

fn with_alpha(color: Rgba, alpha: f32) -> Rgba {
    Rgba::new(color.red, color.green, color.blue, color.alpha * alpha)
}
