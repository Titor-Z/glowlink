//! View：配对码卡片（设计稿未定义，本案新增的最小浮层）。
//!
//! **唯一需要用户隔屏读数的地方**，所以码是全窗最大的字（22px 等宽 `tnum`）。
//! 设计约束：不引入新颜色——失败也是文案，不是红色。

use gpui::{div, prelude::*, px, FontWeight, IntoElement, SharedString, Styled};

use crate::model::drag::{pair_state_note, PAIR_CODE_HINT};
use crate::theme::{self, Theme};
use glowlink_ipc::row::PairingState;

/// 配对卡：`与 <设备> 配对` + 6 位码 + 一句状态说明。
pub fn card(
    t: &Theme,
    peer: &str,
    code: &str,
    state: PairingState,
) -> impl IntoElement {
    div()
        .absolute()
        // 横向居中、贴在节点图底部：本机盘在正中，卡片压在它上面会挡住锚点
        .left(px((theme::WINDOW_W - theme::CARD_W) / 2.0))
        .bottom(px(12.0))
        .w(px(theme::CARD_W))
        .p(px(16.0))
        .rounded(px(theme::RADIUS_CARD))
        .bg(t.card_bg)
        .border_1()
        .border_color(t.window_border)
        .shadow(vec![gpui::BoxShadow {
            color: theme::hsla(t.card_shadow),
            offset: gpui::point(px(0.0), px(8.0)),
            blur_radius: px(24.0),
            spread_radius: px(0.0),
            inset: false,
        }])
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.0))
        .child(
            div()
                .text_size(px(10.5))
                .text_color(t.text_muted)
                .child(SharedString::from(format!("与 {peer} 配对"))),
        )
        .child(
            // 码是给人念/给人敲的：等宽 + tnum，数字不跳
            div()
                .text_size(px(22.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_primary)
                .child(SharedString::from(code.to_string())),
        )
        .child(
            div()
                .text_size(px(9.5))
                .text_color(t.text_muted)
                .child(SharedString::from(match state {
                    PairingState::WaitingPeer => PAIR_CODE_HINT.to_string(),
                    other => pair_state_note(other).to_string(),
                })),
        )
        .child(
            div()
                .text_size(px(9.5))
                .text_color(t.text_hint)
                .child(SharedString::from(match state {
                    PairingState::WaitingPeer => pair_state_note(state).to_string(),
                    _ => String::new(),
                })),
        )
}
