//! View：底部拖拽区（44px 的虚线框）。
//!
//! 设计稿三态：`.dropzone` / `.is-over`（拖入）/ `.is-denied`（发不出去）。
//! 状态由 `model::drop::DropState` 给，这里只负责画。

use std::time::Duration;

use gpui::{
    div, prelude::*, px, AbsoluteLength, Animation, AnimationExt, EdgesRefinement, ElementId,
    FontWeight, IntoElement, SharedString, Styled,
};

use crate::model::drop::DropState;
use crate::theme::{self, Theme};

/// 拒收时的抖动时长（设计稿 `shake 0.36s`）。
const SHAKE_MS: u64 = 360;
/// 抖动的横向幅度（设计稿 25% 处 −4px、75% 处 +4px）。
const SHAKE_AMPLITUDE: f32 = 4.0;
/// 宽度写死：抖动靠改左边距实现，不写死宽度的话盒子会跟着伸缩。
const DROPZONE_W: f32 = theme::WINDOW_W - 2.0 * theme::DROPZONE_INSET;

/// 一个 `0 → −A → 0 → +A → 0` 的位移曲线（正好复刻设计稿那四个关键帧）。
///
/// 用 `sin(2π·delta)` 而不是四个分支：连续、且首尾都归零（动画结束就停在原地，
/// 不必再「复位」一次，也就不会闪那一帧）。
fn shake_offset(delta: f32) -> f32 {
    -(SHAKE_AMPLITUDE * (delta * std::f32::consts::TAU).sin())
}

/// 拖拽区。
///
/// `on_click` 由调用方给（点它 = 用系统对话框选文件；没有可发的设备时会被调用方拒掉）。
pub fn dropzone(
    t: &Theme,
    state: DropState,
    target: Option<&str>,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let (border, ink, bg) = match state {
        // 设计稿：拖入时边框转品牌蓝、底色上一层淡蓝
        DropState::Over => (theme::NODE_SELF, t.text_secondary, Some(t.drop_hover_bg)),
        // 拒收：边框转**台式机粉**（四色常驻色之一，不新增颜色）
        DropState::Denied(_) => (theme::device_tone(glowlink_core::DeviceKind::Desktop), t.text_secondary, None),
        DropState::Idle => (t.drop_border, t.text_muted, None),
    };

    // 先把文字挂上：`Div` 才有 `child`（带 id 之后变成 `Stateful`，那上面的父子关系
    // 要走 `ParentElement`，而 hover / cursor 又只认「可交互元素」——两边各让一步，
    // 顺序就成了：先 child，再 id 与交互样式。
    let base = div()
        .child(SharedString::from(state.note(target)))
        // 要 id 才算「可交互元素」，hover / cursor 这类交互样式都挂在它上面
        .id("dropzone")
        .flex_none()
        .w(px(DROPZONE_W))
        .ml(px(theme::DROPZONE_INSET))
        .mb(px(theme::DROPZONE_INSET))
        .h(px(theme::DROPZONE_H))
        .rounded(px(theme::RADIUS_DROP))
        // 设计稿是 1.5px，而 gpui 只生成 border_1..8 的整数档（没有 border_1_5），
        // 所以这里直接改样式树里的边框宽度——`style()` 是 `Styled` 给的裸通道。
        .map(|mut el| {
            let width = AbsoluteLength::from(px(theme::DROPZONE_BORDER));
            el.style().border_widths = EdgesRefinement {
                top: Some(width),
                right: Some(width),
                bottom: Some(width),
                left: Some(width),
            };
            el
        })
        .border_dashed()
        .border_color(border)
        .when_some(bg, |el, bg| el.bg(bg))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_size(px(theme::TEXT_BODY))
        .font_weight(FontWeight::NORMAL)
        .text_color(ink)
        // hover 只提亮文字（设计稿 `.dropzone:hover { color: text-hint }`）：
        // 边框不动，免得跟「拖入」状态抢信号
        .hover(move |style| style.text_color(t.text_hint))
        .on_click(on_click);

    if !state.is_denied() {
        return base.into_any_element();
    }
    // 拒收才抖一下：**只抖一次**（`repeat()` 会一直抖，那是惩罚不是提示）
    base.with_animation(
        ElementId::Name("dropzone-shake".into()),
        Animation::new(Duration::from_millis(SHAKE_MS)),
        move |el, delta| el.ml(px(theme::DROPZONE_INSET + shake_offset(delta))),
    )
    .into_any_element()
}
