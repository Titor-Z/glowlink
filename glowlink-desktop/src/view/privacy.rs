//! 隐私浮层（设计稿 `.privacy`）：标题栏盾牌点开的一张玻璃卡。
//!
//! 三条承诺 + 一条脚注，**全是事实陈述**（文案在 `model::privacy`，端口从 core 常量来）。
//! 玻璃三件套与浮层卡共用同一批常量（`view::card`），免得同一种材质两处各写一套。

use gpui::prelude::*;
use gpui::{div, px, SharedString};

use crate::model::privacy;
use crate::theme::{self, Theme};
use crate::view::card;

/// 浮层本体。
///
/// `progress` 是入场进度（0..1，由 `AppRoot` 的时钟算好）：
/// 卡片只负责按它摆位，**不自己养动画**——与全项目的做法一致
/// （gpui 的 `Animation::repeat()` 会持续请求整窗重绘，我们不用它）。
pub fn popover(t: &Theme, progress: f32) -> impl IntoElement {
    let (opacity, dy) = privacy::pop_in(progress);
    let title_color = t.text_primary;
    let row_color = t.text_secondary;
    let foot_color = t.text_muted;
    let hairline = t.hairline;
    let dot = theme::NODE_SELF;

    div()
        .absolute()
        .top(px(privacy::TOP + dy))
        .right(px(privacy::RIGHT))
        .w(px(privacy::WIDTH))
        .p(px(privacy::PAD))
        .rounded(px(privacy::RADIUS))
        .bg(t.card_bg)
        // 玻璃三件套：模糊半径 / 1px 描边 / 投影（数值与浮层卡同源）
        .backdrop_filter(vec![gpui::Filter::Blur(px(card::CARD_BLUR))])
        .border(px(card::CARD_BORDER))
        .border_color(theme::hsla(t.window_border))
        .shadow(vec![gpui::BoxShadow {
            color: theme::hsla(t.card_shadow),
            offset: gpui::point(px(0.0), px(card::CARD_SHADOW_Y)),
            blur_radius: px(card::CARD_SHADOW_BLUR),
            spread_radius: px(0.0),
            inset: false,
        }])
        .opacity(opacity)
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(px(theme::TEXT_BODY))
                .text_color(title_color)
                // 600 字重的语义：这是标题，不是又一行承诺
                .font_weight(gpui::FontWeight::SEMIBOLD)
                // 标题与三行之间留 8px（设计稿 `margin-bottom`）
                .mb(px(8.0))
                .child(SharedString::from(privacy::TITLE)),
        )
        .children(privacy::PROMISES.into_iter().map(move |promise| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(privacy::ROW_GAP))
                // 圆点是**品牌色**（设计稿 `.privacy__row::before`）：三行承诺是同一件事的三面
                .child(
                    div()
                        .flex_none()
                        .w(px(privacy::DOT))
                        .h(px(privacy::DOT))
                        .rounded(px(privacy::DOT / 2.0))
                        .bg(dot),
                )
                .child(
                    div()
                        .text_size(px(theme::TEXT_POPOVER_ROW))
                        .text_color(row_color)
                        .child(SharedString::from(promise)),
                )
        }))
        .child(
            div()
                .mt(px(9.0))
                .pt(px(9.0))
                .border_t(px(1.0))
                .border_color(theme::hsla(hairline))
                .text_size(px(theme::TEXT_POPOVER_FOOT))
                .text_color(foot_color)
                .child(SharedString::from(privacy::footnote())),
        )
}
