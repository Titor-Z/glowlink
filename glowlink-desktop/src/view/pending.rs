//! View：接收侧待确认卡（别人要发文件给你，等你拍板）。
//!
//! 格式与其它浮层卡完全一致（同一个 [`crate::view::card`] 外壳）。
//! 内容四样，缺一不可：
//! **谁发来的** · **什么文件多大** · **6 位校验码**（要跟对方屏幕对） · 拒绝 / 接受。
//!
//! 为什么码要占这么大地方：接受是不可轻率的动作，而核对码是唯一的把关手段
//! （已信任设备根本不会走到这里——守护进程直接放行）。提示语也明写
//! 「请与**发送方**屏幕上的校验码核对」——同一句话服务两个角色必然对一个是错的。

use gpui::{
    div, px, Context, Div, FontWeight, ParentElement, SharedString, Stateful, Styled,
};

use crate::controller::PendingView;
use crate::model::format::human_size;
use crate::model::transfer::{pending_queue_note, PENDING_HINT};
use crate::theme::{self, Theme};
use crate::view::app::AppRoot;
use crate::view::card;

/// 待确认卡。`total` 是当前排队的请求总数（多于 1 时补一句「另有 N 个」）。
pub fn card(
    t: &Theme,
    pending: &PendingView,
    total: usize,
    cx: &mut Context<AppRoot>,
) -> Stateful<Div> {
    let reject_token = pending.token.clone();
    // ✕ = 拒绝：待确认请求**必须**有个答案（不答对方要等到超时）
    let shell = card::chrome(
        t,
        &format!("recv-{}", pending.token),
        cx.listener(move |this, _, _, cx| {
            this.reject_receive(reject_token.clone());
            cx.notify();
        }),
    );

    let accept_token = pending.token.clone();
    shell
        .gap(px(6.0))
        .child(
            div()
                .text_size(px(theme::TEXT_BODY))
                .text_color(t.text_primary)
                .pr(px(card::title_reserve()))
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(format!(
                    "「{}」想发给你",
                    pending.peer
                ))),
        )
        .child(
            div()
                .text_size(px(theme::TEXT_META))
                .text_color(t.text_secondary)
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(format!(
                    "{} · {}",
                    pending.name,
                    human_size(pending.size)
                ))),
        )
        .child(
            // 码：全窗最大的字，且和提示同屏——核对这件事不该需要滚动或切屏
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .w_full()
                .text_size(px(theme::TEXT_CODE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_primary)
                .child(SharedString::from(pending.code.clone())),
        )
        .child(hint(t, PENDING_HINT, total))
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .mt(px(2.0))
                .child(card::mini_button(
                    t,
                    "recv-reject",
                    "拒绝",
                    false,
                    cx.listener(move |this, _, _, cx| {
                        this.reject_receive(accept_token.clone());
                        cx.notify();
                    }),
                ))
                .child(card::mini_button(
                    t,
                    "recv-accept",
                    "接受",
                    true,
                    cx.listener(|this, _, _, cx| {
                        this.accept_receive();
                        cx.notify();
                    }),
                )),
        )
}

/// 提示行（核对说明 + 「还有几个在等」）。
fn hint(t: &Theme, text: &str, total: usize) -> Div {
    let mut line = text.to_string();
    if let Some(note) = pending_queue_note(total) {
        line.push_str(" · ");
        line.push_str(&note);
    }
    div()
        .text_size(px(theme::TEXT_META))
        .text_color(t.text_hint)
        .child(SharedString::from(line))
}
