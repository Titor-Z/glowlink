//! View：配对卡。
//!
//! **格式与传输卡完全一致**（同一个 [`crate::view::card`] 外壳：磨砂玻璃、与拖拽区同宽同边距、
//! 右上角圆形 ✕）——两张浮层卡是一类东西，各写一套就会各偏一点
//! （配对卡曾经停在 `bottom: 12`，被拖拽区盖住，看起来「跑到拖拽按钮下面」）。
//!
//! 内容：`与 <设备> 配对` + 6 位码 + 一句状态说明。码是全窗最大的字（22px 等宽 `tnum`）——
//! 这是**唯一需要用户隔屏读数**的地方。设计约束：不引入新颜色（失败也是文案 + 抖动）。

use gpui::{
    div, px, Context, Div, FontWeight, InteractiveElement, ParentElement, SharedString, Stateful,
    Styled,
};

use crate::model::drag::{pair_state_note, PAIR_ASK_HINT, PAIR_CODE_HINT, PAIR_INPUT_HINT};
use crate::model::paircode::{CodeInput, CODE_LEN};
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

/// 接收侧卡片：对端发起配对，等本机用户拍板。
///
/// 两段（设计稿）：先说清「谁想跟你配对」→ 允许之后才出 6 格输入。
/// 码**不在本机**：它显示在对方屏幕上（这是配对安全模型的核心——
/// 一串只有两台机器算得出来、且必须经由人的眼睛传过去的数字）。
///
/// `stage` 由调用方持有：允许之前不显示输入框（否则用户会先去敲码，
/// 而这时候码还没在对方屏幕上出现）。
pub fn request_card(
    t: &Theme,
    request: &RequestCard,
    input: &CodeInput,
    focus: &gpui::FocusHandle,
    cx: &mut Context<AppRoot>,
) -> Stateful<Div> {
    let token = request.token.clone();
    let reject_token = request.token.clone();
    // ✕ = 拒绝：协议里待确认的请求**必须**有个答案（不能「只是收起」），
    // 不答的话对方要一直等到超时；所以 ✕ 与「拒绝」是同一个动作。
    let shell = card::chrome(
        t,
        &format!("pairreq-{}", request.token),
        cx.listener(move |this, _, _, cx| {
            this.reject_pairing(reject_token.clone());
            cx.notify();
        }),
    );

    let head = div().flex().flex_col().gap(px(4.0)).pr(px(card::title_reserve())).child(
        div()
            .text_size(px(theme::TEXT_BODY))
            .text_color(t.text_primary)
            .child(SharedString::from(format!("「{}」想与这台设备配对", request.peer))),
    );

    if !request.allowed {
        // 第一段：只问「允不允许」
        return shell
            .gap(px(8.0))
            .child(head)
            .child(hint(t, PAIR_ASK_HINT))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .mt(px(2.0))
                    .child(card::mini_button(
                        t,
                        "pair-reject",
                        "拒绝",
                        false,
                        cx.listener(move |this, _, _, cx| {
                            this.reject_pairing(token.clone());
                            cx.notify();
                        }),
                    ))
                    .child(card::mini_button(
                        t,
                        "pair-allow",
                        "允许",
                        true,
                        cx.listener(|this, _, _, cx| {
                            this.allow_pairing();
                            cx.notify();
                        }),
                    )),
            );
    }

    // 第二段：6 格输入（键盘只吃数字与退格，见 `model/paircode.rs`）
    let cells = input.cells();
    let cursor = input.cursor();
    let mut row = div().flex().flex_row().justify_center().gap(px(6.0)).w_full();
    for (index, cell) in cells.iter().enumerate() {
        // 当前格稍亮：**不加第五种颜色**，只用底色/字色深浅
        let (bg, ink) = if index == cursor && cursor < CODE_LEN {
            (t.hover_bg, t.text_primary)
        } else {
            (t.bar_bg, t.text_primary)
        };
        row = row.child(
            div()
                .w(px(26.0))
                .h(px(30.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(theme::RADIUS_BUTTON))
                .bg(bg)
                .text_size(px(theme::TEXT_CODE))
                .text_color(ink)
                .child(SharedString::from(cell.clone())),
        );
    }

    // 键位挂在卡片自己身上（`track_focus`）：只有它能收键，别的部件不受影响
    let complete = input.is_complete();
    shell
        .track_focus(focus)
        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
            // 只认「一个字符」的键：带 cmd/ctrl 的组合键（cmd+Q 之类）留给系统
            if event.keystroke.modifiers.modified() {
                return;
            }
            if this.code_key(&event.keystroke.key) {
                cx.notify();
            }
        }))
        .gap(px(8.0))
        .child(head)
        .child(hint(t, PAIR_INPUT_HINT))
        .child(row)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(hint(
                    t,
                    if request.wrong {
                        // 输错不新增颜色：文案 + 「还剩几次」
                        "校验码不符，还剩 1 次机会"
                    } else if request.remaining <= 1 {
                        "校验码不符次数已用尽"
                    } else {
                        ""
                    },
                ))
                .child(card::mini_button(
                    t,
                    "pair-confirm",
                    "确认",
                    complete,
                    cx.listener(|this, _, _, cx| {
                        this.submit_pair_code();
                        cx.notify();
                    }),
                )),
        )
}

/// 卡片里的说明文字（统一字号与颜色）。
fn hint(t: &Theme, text: &str) -> Div {
    div()
        .text_size(px(theme::TEXT_META))
        .text_color(t.text_hint)
        .child(SharedString::from(text.to_string()))
}

/// 接收侧卡片要显示的东西（由 `AppRoot` 从快照里得到）。
#[derive(Clone, Debug, PartialEq)]
pub struct RequestCard {
    pub token: String,
    pub peer: String,
    /// 剩余可试次数（守护进程给的）。
    pub remaining: u8,
    /// 用户是否已经点了「允许」（本机界面状态，不进协议）。
    pub allowed: bool,
    /// 上一次提交的码不对（用来提示重输）。
    pub wrong: bool,
}
