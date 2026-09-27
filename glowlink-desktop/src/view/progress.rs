//! View：传输卡（设计稿 `xfer-card`）。
//!
//! 规格来自 DESIGN.md：**磨砂玻璃、与拖拽区同宽同边距、四行** ——
//! 文件名 + 百分比（基线对齐）· 4px 渐变条 · 9px 元信息 · 结束时的按钮。
//! 外框（位置/材质/右上角 ✕）由 [`crate::view::card`] 统一提供：
//! 两张浮层卡是同一个格式，格式只有一处定义就不会再分叉。
//!
//! 它是**状态报告**，不是装饰：所有数字都来自真实字节与耗时
//! （DESIGN.md：「Don't 写没有来源的指标」）。

use gpui::{
    div, prelude::*, px, Context, Div, FontWeight, ParentElement, SharedString, Stateful, Styled,
};

use crate::model::transfer::{Action, CardNumbers};
use crate::theme::{self, Theme};
use crate::view::app::AppRoot;
use crate::view::card;
use glowlink_ipc::row::{TransferDirection, TransferRow, TransferState};

/// 进度条高度（设计稿 4px）。
const BAR_H: f32 = 4.0;
/// 「已传 / 总量 · 剩余」那行与进度条之间的呼吸（设计稿 `spacing.md 0 spacing.sm`）。
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
    let running = row.state == TransferState::Running;
    let token = row.token.clone();

    let mut bar = div().relative().h(px(BAR_H)).w_full().rounded_full().bg(t.bar_bg);
    if filled > 0.0 {
        bar = bar.child(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .h(px(BAR_H))
                // 宽度按比例：卡宽（窗口 − 两侧边距）− 两侧内边距
                .w(px(
                    (theme::WINDOW_W - 2.0 * card::CARD_INSET - 32.0) * filled,
                ))
                .rounded_full()
                // 全项目**唯一**允许的线性渐变（DESIGN.md 的 colors 里写明只有它）
                .bg(gpui::linear_gradient(
                    90.0,
                    gpui::linear_color_stop(theme::hsla(t.progress_from), 0.0),
                    gpui::linear_color_stop(theme::hsla(t.progress_to), 1.0),
                )),
        );
    }

    // ✕：在传 = 取消传输，结束 = 关掉卡片（同一位置同一图标，语义随状态变）
    let shell = card::chrome(
        t,
        &format!("xfer-{}", row.token),
        cx.listener(move |this, _, _, cx| {
            if running {
                this.cancel_transfer(&token);
            } else {
                this.dismiss_transfer(&token);
            }
            cx.notify();
        }),
    );

    shell
        .child(
            div()
                .flex()
                .flex_row()
                .items_baseline()
                .justify_between()
                .gap(px(8.0))
                .child(
                    // 文件名：11px，超长省略号截断（绝不换行）
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
                    // 百分比：等宽数字（`tnum`）——字宽一变数字就会左右跳，看着像在抖。
                    // 右侧给 ✕ 留位置
                    div()
                        .flex_none()
                        .pr(px(card::title_reserve()))
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

/// 结束后的按钮（设计稿：完成态才出现）。
///
/// 只有**收下来的**文件才给「打开目录」——发送方本机本来就有这个文件。
/// 「关掉卡片」由右上角那个 ✕ 承担（同一个动作不必有两个入口）。
fn buttons(t: &Theme, row: &TransferRow, cx: &mut Context<AppRoot>) -> Option<Div> {
    if row.state == TransferState::Running {
        return None;
    }
    if crate::model::transfer::action(row) != Some(Action::OpenFolder) {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_row()
            .gap(px(8.0))
            .mt(px(10.0))
            .child(card::mini_button(
                t,
                "open",
                "打开目录",
                false,
                cx.listener(|this, _, _, cx| {
                    this.open_received_folder();
                    cx.notify();
                }),
            )),
    )
}
