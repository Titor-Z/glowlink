//! View：传输卡（设计稿 `xfer-card`）。
//!
//! 规格来自 DESIGN.md：**200px、贴右下（right 12 / bottom 70）、`card-bg` + 玻璃**，
//! 四行 —— 文件名 + 百分比（基线对齐）· 4px 渐变条 · 9px 元信息 · 结束时的按钮。
//!
//! 它是**状态报告**，不是装饰：所有数字都来自真实字节与耗时
//! （DESIGN.md：「Don't 写没有来源的指标」）。

use gpui::{
    div, prelude::*, px, Context, Div, ElementId, FontWeight, ParentElement,
    SharedString, Stateful, Styled,
};

use crate::model::transfer::{Action, CardNumbers};
use crate::theme::{self, Theme};
use crate::view::app::AppRoot;
use glowlink_ipc::row::{TransferDirection, TransferRow, TransferState};

/// 卡片的左右内边距 = 拖拽区的内缩：两者**同宽同边距**，上下对齐才是「一列」。
const CARD_INSET: f32 = crate::theme::DROPZONE_INSET;
/// 距窗口底边的距离：让开拖拽区 + 12px 缝。
const CARD_BOTTOM: f32 = crate::theme::DROPZONE_INSET + crate::theme::DROPZONE_H + 12.0;
/// 关闭按钮的直径（圆形底）。
const CLOSE_SIZE: f32 = 20.0;
/// 磨砂玻璃的模糊半径（设计稿：卡片底 20px backdrop blur）。
const CARD_BLUR: f32 = 20.0;
/// 进度条高度（设计稿 4px）。
const BAR_H: f32 = 4.0;
/// 小按钮高度（设计稿 `minibtn` 24px）。
const MINI_BTN_H: f32 = 24.0;
/// 「已传字节 / 总字节」那行与进度条之间的呼吸（设计稿 `spacing.md 0 spacing.sm`）。
const META_GAP: f32 = 8.0;

/// 传输卡。`fraction` 是**平滑后**的填充比例（0..1，平滑在 `AppRoot` 里做）。
pub fn card(
    t: &Theme,
    row: &TransferRow,
    numbers: &CardNumbers,
    fraction: f64,
    cx: &mut Context<AppRoot>,
) -> Stateful<Div> {
    let filled = fraction.clamp(0.0, 1.0) as f32;
    let direction = match row.direction {
        TransferDirection::Send => "发送",
        TransferDirection::Receive => "接收",
    };

    let mut bar = div().relative().h(px(BAR_H)).w_full().rounded_full().bg(t.bar_bg);
    if filled > 0.0 {
        bar = bar.child(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .h(px(BAR_H))
                // 宽度按比例：卡宽 - 两侧内边距（16 x 2）
                .w(px((theme::CARD_W - 32.0) * filled))
                .rounded_full()
                // 全项目**唯一**允许的线性渐变（DESIGN.md 的 colors 里写明只有它）
                .bg(gpui::linear_gradient(
                    90.0,
                    gpui::linear_color_stop(theme::hsla(t.progress_from), 0.0),
                    gpui::linear_color_stop(theme::hsla(t.progress_to), 1.0),
                )),
        );
    }

    div()
        .id(ElementId::Name(format!("xfer-{}", row.token).into()))
        .absolute()
        .bottom(px(CARD_BOTTOM))
        // 关闭 / 取消：圆形底、hover 变红（房规里第二处用红，与窗口关闭键同源）
        .child(close_button(t, row, cx))
        // 与下方拖拽区**同宽同边距**（320 − 16×2 = 288）：一列到底，不再是一张小卡
        .left(px(CARD_INSET))
        .right(px(CARD_INSET))
        .p(px(16.0))
        .rounded(px(theme::RADIUS_CARD))
        // 磨砂玻璃：半透明底 + backdrop blur（设计稿的玻璃拟物）。
        // 底下是玻璃窗，所以这里再叠一层模糊，卡片才像「浮在窗上的另一片玻璃」。
        .bg(t.card_bg)
        .backdrop_filter(vec![gpui::Filter::Blur(px(CARD_BLUR))])
        .shadow(vec![gpui::BoxShadow {
            color: theme::hsla(t.card_shadow),
            offset: gpui::point(px(0.0), px(8.0)),
            blur_radius: px(24.0),
            spread_radius: px(0.0),
            inset: false,
        }])
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .flex_row()
                .items_baseline()
                .justify_between()
                .gap(px(8.0))
                .child(
                    // 文件名：11px，超长省略号截断（卡只有 200px，绝不换行）
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .text_size(px(theme::TEXT_BODY))
                        .text_color(t.text_primary)
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(SharedString::from(row.name.clone())),
                )
                .child(
                    // 百分比：等宽数字（`tnum`，房规见 theme 的说明）——
                    // 字宽一变数字就会左右跳，看起来像在抖。
                    // 右侧留出关闭按钮的位置（圆形按钮压在同一行的右端）
                    div()
                        .flex_none()
                        .pr(px(CLOSE_SIZE + 6.0))
                        .text_size(px(theme::TEXT_BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.pct)
                        .child(SharedString::from(numbers.percent.clone())),
                ),
        )
        .child(div().h(px(META_GAP)).child(bar))
        .child(
            div()
                .text_size(px(theme::TEXT_META))
                .text_color(t.text_hint)
                .child(SharedString::from(format!("{} · {direction}", numbers.meta))),
        )
        .when_some(buttons(t, row, cx), |el, buttons| el.child(buttons))
}

/// 右上角的圆形按钮：**在传 = 取消**，结束 = 关掉卡片。
///
/// 同一个位置、同一个图标（✕），语义随状态变——用户不必学两个按钮。
/// hover 变红（`CLOSE_HOVER_BG`）：这是「会造成中断」的动作，必须一眼看出分量。
fn close_button(t: &Theme, row: &TransferRow, cx: &mut Context<AppRoot>) -> Stateful<Div> {
    let token = row.token.clone();
    let running = row.state == TransferState::Running;
    let (hover_bg, hover_ink) = t.icon_hover(theme::IconHover::Close);
    let (base_ink, _) = t.icon_hover(theme::IconHover::Neutral);
    div()
        .id(ElementId::Name(format!("xfer-close-{}", row.token).into()))
        .absolute()
        .right(px(6.0))
        .top(px(6.0))
        .w(px(CLOSE_SIZE))
        .h(px(CLOSE_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .text_size(px(theme::TEXT_META))
        .text_color(base_ink)
        .hover(move |style| style.bg(hover_bg).text_color(hover_ink))
        .child(SharedString::from("✕"))
        .on_click(cx.listener(move |this, _, _, cx| {
            if running {
                this.cancel_transfer(&token);
            } else {
                this.dismiss_transfer(&token);
            }
            cx.notify();
        }))
}

/// 结束后的按钮（设计稿：完成态才出现；「用户自己关」）。
///
/// 不做图标：房规里加一个图标要同时改文件与 `ICONS` 表，而这里文字更清楚
/// （「打开目录 / 关闭」本身就是动作名）。
fn buttons(t: &Theme, row: &TransferRow, cx: &mut Context<AppRoot>) -> Option<Div> {
    if row.state == TransferState::Running {
        return None;
    }
    let mut row_el = div().flex().flex_row().gap(px(8.0)).mt(px(10.0));
    if crate::model::transfer::action(row) == Some(Action::OpenFolder) {
        row_el = row_el.child(mini_button(
            t,
            "open",
            "打开目录",
            cx.listener(|this, _, _, cx| {
                this.open_received_folder();
                cx.notify();
            }),
        ));
    }
    Some(row_el)
}

/// 卡片里的小按钮：24px 高、8px 圆角、9px 文字，hover 只提亮底色。
fn mini_button(
    t: &Theme,
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    div()
        .id(ElementId::Name(format!("xfer-btn-{id}").into()))
        .flex()
        .flex_row()
        .items_center()
        .h(px(MINI_BTN_H))
        .px(px(10.0))
        .rounded(px(theme::RADIUS_BUTTON))
        .cursor_pointer()
        .hover(|style| style.bg(t.hover_bg))
        .text_size(px(theme::TEXT_META))
        .text_color(t.text_secondary)
        .child(SharedString::from(label))
        .on_click(on_click)
}
