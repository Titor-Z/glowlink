//! View：空态（`正在搜索附近设备…` + spinner）。
//!
//! 设计稿 `.empty-note`：绝对定位在 `top: 268px`、水平居中、11px 弱色文字，
//! 左边一个 12px 的 spinner（`border-top-color: node-self`，0.9s 一圈）。
//!
//! **gpui 没有 `rotate`**，所以这里不是「转一个环」，而是**按相位重画那一段弧**：
//! 环底是 25% 透明度的品牌色整圈，前面一段 90° 的不透明弧跟着相位走。
//! 看起来就是在转，而且只在相位变化时才有重绘代价。

use gpui::{
    canvas, div, prelude::*, px, size, rgba, PathBuilder, Point, SharedString, Styled,
};

use crate::model::state::{note_text, EmptyNote};
use crate::theme::{self, Theme};

/// spinner 直径（设计稿 12px）。
const SPINNER_SIZE: f32 = 12.0;
/// 环的线宽（设计稿 1.5px）。
const SPINNER_WIDTH: f32 = 1.5;
/// 一圈多久（设计稿 `spin 0.9s`）。
pub const SPINNER_PERIOD_SECS: f32 = 0.9;
/// 前面那段弧占多少圈（设计稿的四分之一）。
const SPINNER_ARC: f32 = 0.25;
/// 用多少段直线近似一段弧（12px 的环，16 段看不出棱角）。
const SEGMENTS: usize = 16;

/// 空态提示：`[spinner] 一句话`（`None` = 什么都不画）。
///
/// 只有「正在搜索」时才转：服务关着、或搜索已经出结论时，**不动**——
/// 转圈的意思是「正在做事」，不该用来装饰一句静止的结论。
pub fn empty_note(
    t: &Theme,
    note: Option<(EmptyNote, bool)>,
    phase: f32,
) -> impl IntoElement {
    let (kind, spinning) = note.unwrap_or((EmptyNote::Searching, false));
    let tone = theme::NODE_SELF;
    div()
        .absolute()
        .left(px(0.0))
        .right(px(0.0))
        .top(px(theme::EMPTY_NOTE_Y))
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .when(note.is_none(), |el| el.opacity(0.0))
        .when(note.is_some() && spinning, |el| {
            el.child(
                div()
                    .flex_none()
                    .w(px(SPINNER_SIZE))
                    .h(px(SPINNER_SIZE))
                    .child(
                        canvas(
                            move |_, _, _| (),
                            move |bounds, _, window, _| {
                                let center = bounds.center();
                                let radius = SPINNER_SIZE / 2.0 - SPINNER_WIDTH / 2.0;
                                // 环底：整圈弱色（设计稿 rgba(124,140,255,.25)）
                                let mut track = PathBuilder::stroke(px(SPINNER_WIDTH));
                                trace_circle(&mut track, center, radius, 0.0, 1.0);
                                if let Ok(path) = track.build() {
                                    window.paint_path(path, theme::hsla(rgba(0x7C8CFF_40)));
                                }
                                // 前面那段：跟着相位转
                                let mut head = PathBuilder::stroke(px(SPINNER_WIDTH));
                                trace_circle(&mut head, center, radius, phase, SPINNER_ARC);
                                if let Ok(path) = head.build() {
                                    window.paint_path(path, theme::hsla(tone));
                                }
                            },
                        )
                        .w(px(SPINNER_SIZE))
                        .h(px(SPINNER_SIZE)),
                    ),
            )
        })
        .child(
            div()
                .text_size(px(theme::TEXT_BODY))
                .text_color(t.text_muted)
                .child(SharedString::from(if note.is_some() {
                    note_text(kind)
                } else {
                    ""
                })),
        )
}
/// 把一段圆弧描成折线（`PathBuilder` 的 `arc_to` 要算 SVG 的
/// `large_arc/sweep` 标志，为一段 12px 的弧不值得；16 段直线肉眼无缝）。
fn trace_circle(
    path: &mut PathBuilder,
    center: Point<gpui::Pixels>,
    radius: f32,
    start: f32,
    span: f32,
) {
    if span >= 1.0 {
        // 整圈：首尾相接（闭合），否则会有个接缝
        for step in 0..=SEGMENTS * 4 {
            let angle = std::f32::consts::TAU * (step as f32 / (SEGMENTS * 4) as f32);
            let point = point_at(center, radius, angle);
            if step == 0 {
                path.move_to(point);
            } else {
                path.line_to(point);
            }
        }
        return;
    }
    for step in 0..=SEGMENTS {
        let t = step as f32 / SEGMENTS as f32;
        let point = point_at(center, radius, std::f32::consts::TAU * (start + span * t));
        if step == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
}

fn point_at(center: Point<gpui::Pixels>, radius: f32, angle: f32) -> Point<gpui::Pixels> {
    // 从 12 点方向起算（角度 0 = 正上方），顺时针为正
    let x = center.x.as_f32() + radius * angle.sin();
    let y = center.y.as_f32() - radius * angle.cos();
    size(px(x), px(y)).into()
}
