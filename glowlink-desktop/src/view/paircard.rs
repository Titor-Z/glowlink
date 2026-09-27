//! View：配对卡。
//!
//! **格式与传输卡完全一致**（同一个 [`crate::view::card`] 外壳：磨砂玻璃、与拖拽区同宽同边距、
//! 右上角圆形 ✕）——两张浮层卡是一类东西，各写一套就会各偏一点
//! （配对卡曾经停在 `bottom: 12`，被拖拽区盖住，看起来「跑到拖拽按钮下面」）。
//!
//! 内容：`与 <设备> 配对` + 6 位码 + 一句状态说明。码是全窗最大的字（22px 等宽 `tnum`）——
//! 这是**唯一需要用户隔屏读数**的地方。设计约束：不引入新颜色（失败也是文案 + 抖动）。

use gpui::{
    div, px, Context, Div, FontWeight, ParentElement, SharedString, Stateful, Styled,
};

use crate::model::drag::{pair_state_note, PAIR_CODE_HINT};
use crate::theme::{self, Theme};
use crate::view::app::AppRoot;
use crate::view::card;
use glowlink_ipc::row::PairingState;

/// 配对卡：`与 <设备> 配对` + 6 位码 + 一句状态说明。
///
/// ✕ = **放弃这次配对**（本机主动取消，与「被拒」是两回事）。
pub fn card(
    t: &Theme,
    token: &str,
    peer: &str,
    code: &str,
    state: PairingState,
    cx: &mut Context<AppRoot>,
) -> Stateful<Div> {
    let token = token.to_string();
    let shell = card::chrome(
        t,
        &format!("pair-{token}"),
        cx.listener(move |this, _, _, cx| {
            this.abandon_pairing(&token);
            cx.notify();
        }),
    );

    let title = format!("与 {peer} 配对");
    // 码是这一屏最重要的东西：给足空间，别让 ✕ 挤到它
    let code_row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .w_full()
        .pr(px(card::title_reserve()))
        .child(
            div()
                .text_size(px(theme::TEXT_CODE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_primary)
                .child(SharedString::from(code.to_string())),
        );

    shell
        .gap(px(6.0))
        .child(
            div()
                .text_size(px(theme::TEXT_BODY))
                .text_color(t.text_secondary)
                .child(SharedString::from(title)),
        )
        .child(code_row)
        .child(
            div()
                .text_size(px(theme::TEXT_META))
                .text_color(t.text_hint)
                .child(SharedString::from(pair_state_note(state))),
        )
        .child(
            div()
                .text_size(px(theme::TEXT_META))
                .text_color(t.text_hint)
                // 提示只在「等对方输码」时有意义（其余状态那句话已经说清了）
                .child(SharedString::from(if state == PairingState::WaitingPeer {
                    PAIR_CODE_HINT
                } else {
                    ""
                })),
        )
}
