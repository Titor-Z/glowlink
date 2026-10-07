//! View：设置面板（画板 `settings-dark-a1f3` / `settings-light-c7d9`）。
//!
//! 一块盖在节点图上的磨砂浮层（标题栏齿轮点开），五组：
//! **配对名单**（删除）· **外观**（主题分段控件）· **接收**（目录）·
//! **节点图**（两个开关）· **关于**（版本 + 隐私承诺）。
//!
//! 玻璃材质与浮层卡同源（[`crate::view::card`] 的常量）——同一种材质只有一处定义。
//! 入场动画复用 `privacy::pop_in`（三次缓出 + 4px 下落），不养自己的时钟。
//!
//! 没做的（见 AGENTS.md「设置页待做清单」）：
//! - **配对改名**：行内文本输入要碰 IME / `EntityInputHandler`，先不做（名单只读 + 删除）；
//! - **关闭窗口时**（收进托盘 / 退出）：托盘还没落地，先摆这个选项等于撒谎。

use gpui::prelude::*;
use std::sync::{Arc, Mutex};

use gpui::{
    canvas, div, px, AnyElement, Bounds, Context, Div, ElementId, FontWeight, IntoElement,
    ParentElement, Pixels, Rgba, SharedString, Stateful, Styled,
};

use crate::controller::{DeviceView, TrustedView};
use crate::model::namefield::NameField;
use crate::model::settings::{Settings, ThemeChoice};
use crate::theme::{self, Theme};
use crate::view::app::AppRoot;
use crate::view::titlebar;

/// 面板尺寸（设计稿 `.panel`）。
pub const HEAD_H: f32 = 36.0;
pub const PAD_X: f32 = 16.0;
pub const PAD_BOTTOM: f32 = 12.0;
/// 分段控件：内边距 2；**整行宽**（设计稿 `.seg` 在 `row--stack` 里不设宽，
/// 只有内联变体 `.seg--inline` 才是 184——主题这颗没有内联）。
pub const SEG_FULL_W: f32 = theme::WINDOW_W - PAD_X * 2.0;
const SEG_PAD: f32 = 2.0;
/// 配对名单卡片之间的间距（设计稿 `.pairs { gap: var(--space-2) }` = 8px）。
pub const PAIRS_GAP: f32 = 8.0;
/// 开关（设计稿 `.switch`）：34×20，滑块 16。
pub const SWITCH_W: f32 = 34.0;
pub const SWITCH_H: f32 = 20.0;
const SWITCH_KNOB: f32 = 16.0;
const SWITCH_PAD: f32 = 2.0;
/// 配对名单行（设计稿 `.pair`）。
const PAIR_PAD_Y: f32 = 9.0;
const PAIR_PAD_X: f32 = 10.0;

/// 设置页的字阶（glowdrop 新版 tokens 的「桌面可读档」）。
///
/// **只作用于设置页**：不动 `theme.rs` 的全局字号——主窗（节点图 / 卡片 / 拖拽区）
/// 仍按旧设计稿的 11/10/9 档。两者可以并存（设置页将来若独立成窗口更该如此）。
const FS_BODY: f32 = 13.0;
const FS_LABEL: f32 = 11.0;
const FS_CAPTION: f32 = 10.5;
const FS_HEADING: f32 = 15.0;

/// 单行文字的行高。
///
/// **必须显式给**：gpui 默认行高 1.618（黄金比），比设计稿的 1.2~1.4 高出三分之一。
/// 五组叠起来就超出窗高，最后一行（隐私承诺）被滚动视口切掉半截——用户看到的就是
/// 「只显示了一半」。这里按设计稿的 leading 钉死（标题 13px 是设计稿的显式值）。
const LH_BODY: f32 = 16.0;
const LH_LABEL: f32 = 13.0;
const LH_CAPTION: f32 = 14.5;

/// 改名输入框注册输入处理器需要的三样东西（paint 期用）。
pub struct RenameCtx {
    /// 界面实体：`EntityInputHandler` 要靠它把 IME 回调送回 `AppRoot`。
    pub entity: gpui::Entity<AppRoot>,
    pub focus: gpui::FocusHandle,
    /// 输入框位置槽（paint 时写入，IME 候选窗定位读取）。
    pub bounds: Arc<Mutex<Option<Bounds<Pixels>>>>,
}

/// 品牌强调色（深色 = 品牌蓝；浅色 = 压暗版，白底上才读得出）。
fn accent(t: &Theme) -> Rgba {
    if t.is_light {
        theme::light::ICON_SELF
    } else {
        theme::NODE_SELF
    }
}

/// 强调色的**文字**形态（选中项的墨色）。
fn ink(t: &Theme) -> Rgba {
    t.icon_self
}

/// 给强调色套一个不透明度（分段滑块 30%、开关辉光 45% 与设计稿一致）。
fn with_alpha(color: Rgba, alpha: f32) -> Rgba {
    Rgba::new(color.red, color.green, color.blue, alpha)
}

/// 整个面板（`progress` 是入场进度，由 `AppRoot` 的时钟算好）。
pub fn panel(
    t: &Theme,
    settings: &Settings,
    trusts: &[TrustedView],
    devices: &[DeviceView],
    receive_dir: Option<&str>,
    confirm_del: Option<&str>,
    rename: Option<(&str, &NameField)>,
    ctx: &RenameCtx,
    progress: f32,
    seg_pos: f32,
    cx: &mut Context<AppRoot>,
) -> Stateful<Div> {
    let (opacity, dy) = crate::model::privacy::pop_in(progress);

    div()
        .id("settings-panel")
        .absolute()
        .top(px(theme::TITLEBAR_H + dy))
        .left_0()
        .right_0()
        .bottom_0()
        .px(px(PAD_X))
        .pt(px(2.0))
        .pb(px(PAD_BOTTOM))
        .flex()
        .flex_col()
        // **挡住下层的鼠标事件**：不挡的话拖拽区、节点都还能点（浮层只管画，
        // 命中测试还是会落到下面的元素上）。面板盖住的范围就是它的范围，
        // 标题栏（齿轮要能点）不在这块区域里，不受影响。
        .occlude()
        // 磨砂：卡片色打底（设计稿 `color-mix(card 68%, transparent)`）+ backdrop 模糊，
        // 节点图透出来变成柔光斑——**不铺死底**，玻璃没有可透的东西就退化成一块死灰。
        .bg(with_alpha(t.card_bg, t.card_bg.alpha * 0.68))
        .backdrop_filter(vec![gpui::Filter::Blur(px(24.0))])
        .opacity(opacity)
        .child(head(t, cx))
        .child(body(
            t,
            settings,
            trusts,
            devices,
            receive_dir,
            confirm_del,
            rename,
            ctx,
            seg_pos,
            cx,
        ))
}

/// 面板头：返回 + 「设置」，下缘一条发丝线。
fn head(t: &Theme, cx: &mut Context<AppRoot>) -> Div {
    div()
        .flex_none()
        .h(px(HEAD_H))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .border_b(px(1.0))
        .border_color(theme::hsla(t.hairline))
        .child(titlebar::icon_button(
            "settings-back",
            t,
            "icons/back.svg",
            t.text_secondary,
            None,
            theme::IconHover::Neutral,
            cx.listener(|this, _, _, cx| {
                this.close_settings();
                cx.notify();
            }),
        ))
        .child(
            div()
                .text_size(px(FS_HEADING))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_primary)
                .child(SharedString::from("设置")),
        )
}

/// 面板体：可滚动（内容比图区高时别挤死）。
fn body(
    t: &Theme,
    settings: &Settings,
    trusts: &[TrustedView],
    devices: &[DeviceView],
    receive_dir: Option<&str>,
    confirm_del: Option<&str>,
    rename: Option<(&str, &NameField)>,
    ctx: &RenameCtx,
    seg_pos: f32,
    cx: &mut Context<AppRoot>,
) -> Stateful<Div> {
    div()
        .id("settings-body")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        // 行高向下继承：正文一档（各标题/说明再各自覆盖）
        .line_height(px(LH_BODY))
        .child(group(
            t,
            "配对名单",
            // 空态没有卡可改可删，说明那句指路是空话（画板 settings-empty 的口径）
            (!trusts.is_empty()).then_some(PAIR_HINT),
            vec![pairs(t, trusts, devices, confirm_del, rename, ctx, cx)],
            false,
        ))
        .child(group(
            t,
            "外观",
            None,
            vec![stack_row(t, "主题")
                .child(seg_theme(t, settings.theme, seg_pos, cx))
                .into_any_element()],
            false,
        ))
        .child(group(
            t,
            "接收",
            None,
            vec![dir_row(t, receive_dir, cx).into_any_element()],
            false,
        ))
        .child(group(
            t,
            "节点图",
            None,
            vec![
                row(t, "空态显示本机名字")
                    .child(switch(
                        t,
                        "self-name-switch",
                        settings.self_name_in_empty,
                        cx.listener(|this, _, _, cx| {
                            let next = !this.settings.self_name_in_empty;
                            this.set_self_name_in_empty(next);
                            cx.notify();
                        }),
                    ))
                    .into_any_element(),
                row(t, "空态提示常驻")
                    .child(switch(
                        t,
                        "persistent-switch",
                        settings.note_persistent,
                        cx.listener(|this, _, _, cx| {
                            let next = !this.settings.note_persistent;
                            this.set_note_persistent(next);
                            cx.notify();
                        }),
                    ))
                    .into_any_element(),
            ],
            true,
        ))
        .child(group(
            t,
            "关于",
            None,
            vec![
                row(t, "版本")
                    .child(value(t, &format!("v{}", env!("CARGO_PKG_VERSION"))))
                    .into_any_element(),
                privacy_row(t, cx).into_any_element(),
            ],
            true,
        ))
}

/// 一组：标题（+ 可选说明）与若干行。
///
/// 间距严格照设计稿：组与组之间 12（`.group`），标题与说明各自
/// `margin-bottom: 4`（不是说明自己往上贴）。`hairline_between` 对应
/// `.rows > * + * { border-top: 1px }`——只有「节点图」「关于」这类
/// `.rows` 容器里的行才有分隔线；配对卡是 `.pairs`（用 gap），外观组没有。
fn group(
    t: &Theme,
    title: &str,
    hint: Option<&str>,
    rows: Vec<AnyElement>,
    hairline_between: bool,
) -> Div {
    let mut group = div()
        .mt(px(12.0))
        .flex()
        .flex_col()
        .child(
            div()
                .mb(px(4.0))
                .text_size(px(FS_LABEL))
                .font_weight(FontWeight::NORMAL)
                .line_height(px(LH_LABEL))
                .text_color(t.text_muted)
                .child(SharedString::from(title)),
        )
        .when_some(hint, |el, hint| {
            el.child(
                div()
                    .mb(px(4.0))
                    .text_size(px(FS_CAPTION))
                    .line_height(px(LH_CAPTION))
                    .text_color(t.text_muted)
                    .child(SharedString::from(hint)),
            )
        });
    for (index, row) in rows.into_iter().enumerate() {
        if hairline_between && index > 0 {
            group = group.child(
                div()
                    .border_t(px(1.0))
                    .border_color(theme::hsla(t.hairline))
                    .child(row),
            );
        } else {
            group = group.child(row);
        }
    }
    group
}

/// 一行：左标签，右侧由调用方补（开关 / 值 / 箭头）。
fn row(t: &Theme, label: &str) -> Div {
    div()
        .min_h(px(28.0))
        .py(px(4.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(label_el(t, label))
}

/// `.row--stack`：标签在上、控件在下（主题的分段控件要占整行），行内 gap 4。
fn stack_row(t: &Theme, label: &str) -> Div {
    div()
        .py(px(4.0))
        .flex()
        .flex_col()
        .items_stretch()
        .gap(px(4.0))
        .child(label_el(t, label))
}

fn label_el(t: &Theme, label: &str) -> Div {
    div()
        .text_size(px(FS_BODY))
        .text_color(t.text_secondary)
        .child(SharedString::from(label))
}

fn value(t: &Theme, text: &str) -> Div {
    div()
        .text_size(px(FS_BODY))
        .text_color(t.text_muted)
        .child(SharedString::from(text))
}

const PAIR_HINT: &str = "点铅笔改名 · 改名后保留原名 · 名字在扫描与连接时显示";

/// 配对名单为空时那枚幽灵卡上的一行字（画板 `settings-empty`）。
const PAIR_EMPTY: &str = "还没有已配对的设备";

/// 配对名单：一行一张卡（画板 `settings-pair-states-b6c1`）。
///
/// 三态互斥（同一行一次只有一个）：常态（铅笔 + 垃圾桶）→
/// 编辑态（输入框 + 保存/取消）→ 删除确认态（后果说明 + 保留/删除）。
fn pairs(
    t: &Theme,
    trusts: &[TrustedView],
    devices: &[DeviceView],
    confirm_del: Option<&str>,
    rename: Option<(&str, &NameField)>,
    ctx: &RenameCtx,
    cx: &mut Context<AppRoot>,
) -> AnyElement {
    if trusts.is_empty() {
        // 空态是一枚**描边幽灵卡**（不是裸文字）：跟配对卡同壳，但无底、无内容、居中一行，
        // 一眼看出是「这里空着」而不是「这里有台设备」。
        return div()
            .py(px(PAIR_PAD_Y))
            .px(px(PAIR_PAD_X))
            .rounded(px(theme::RADIUS_CARD))
            .border(px(1.0))
            .border_color(theme::hsla(t.hairline))
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_size(px(FS_BODY))
                    .line_height(px(LH_BODY))
                    .text_color(t.text_muted)
                    .child(SharedString::from(PAIR_EMPTY)),
            )
            .into_any_element();
    }
    div()
        .flex()
        .flex_col()
        .gap(px(PAIRS_GAP))
        .children(
            trusts
                .iter()
                .map(|trust| {
            let fingerprint = trust.fingerprint.clone();
            let editing = rename.map(|(fp, _)| fp == fingerprint).unwrap_or(false);
            let deleting = confirm_del == Some(fingerprint.as_str());
            let current = rename
                .and_then(|(fp, field)| (fp == fingerprint).then(|| field.text().to_string()))
                .unwrap_or_else(|| {
                    if trust.name.is_empty() {
                        short(&fingerprint)
                    } else {
                        trust.name.clone()
                    }
                });

            // 中段：编辑态是输入框，删除确认态是后果说明，常态是名字堆叠
            let middle = if editing {
                rename_input(t, ctx, cx).into_any_element()
            } else if deleting {
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(px(FS_CAPTION))
                    .line_height(px(LH_CAPTION))
                    .text_color(t.text_secondary)
                    .child(SharedString::from(PAIR_DEL_NOTE))
                    .into_any_element()
            } else {
                // 双行堆叠：别名在上；与原名不同时，原名以小字垫在下面
                let original = devices
                    .iter()
                    .find(|device| device.id == fingerprint)
                    .map(|device| device.name.clone())
                    .filter(|name| *name != current);
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(FS_BODY))
                            .text_color(t.text_primary)
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(SharedString::from(current)),
                    )
                    .when_some(original, |el, original| {
                        el.child(
                            div()
                                .text_size(px(FS_CAPTION))
                                .line_height(px(LH_CAPTION))
                                .text_color(t.text_muted)
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(SharedString::from(original)),
                        )
                    })
                    .into_any_element()
            };

            // 尾段：随状态换动作
            let actions = if editing {
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(mini_button(
                        t,
                        "trust-save",
                        "保存",
                        true,
                        cx.listener(|this, _, _, cx| {
                            this.save_rename();
                            cx.notify();
                        }),
                    ))
                    .child(mini_button(
                        t,
                        "trust-cancel",
                        "取消",
                        false,
                        cx.listener(|this, _, _, cx| {
                            this.cancel_rename();
                            cx.notify();
                        }),
                    ))
                    .into_any_element()
            } else if deleting {
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(mini_button(
                        t,
                        "trust-keep",
                        "保留",
                        false,
                        cx.listener(|this, _, _, cx| {
                            this.cancel_delete_trust();
                            cx.notify();
                        }),
                    ))
                    .child(mini_button(
                        t,
                        "trust-confirm",
                        "删除",
                        true,
                        cx.listener(|this, _, _, cx| {
                            let fingerprint = this.confirm_delete.clone().unwrap_or_default();
                            this.confirm_delete_trust(&fingerprint);
                            cx.notify();
                        }),
                    ))
                    .into_any_element()
            } else {
                let name_for_edit = if trust.name.is_empty() {
                    short(&fingerprint)
                } else {
                    trust.name.clone()
                };
                let edit_fingerprint = fingerprint.clone();
                let del_fingerprint = fingerprint.clone();
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(
                        titlebar::icon_button(
                            SharedString::from(format!("trust-edit-{fingerprint}")),
                            t,
                            "icons/pencil.svg",
                            t.text_secondary,
                            None,
                            theme::IconHover::Neutral,
                            cx.listener(move |this, _, _, cx| {
                                this.begin_rename(&edit_fingerprint, &name_for_edit);
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        titlebar::icon_button(
                            SharedString::from(format!("trust-del-{fingerprint}")),
                            t,
                            "icons/trash.svg",
                            t.text_secondary,
                            None,
                            theme::IconHover::Neutral,
                            cx.listener(move |this, _, _, cx| {
                                this.begin_delete_trust(&del_fingerprint);
                                cx.notify();
                            }),
                        ),
                    )
                    .into_any_element()
            };

            // 色点：在线设备用它的类型色，离线的用中性灰；带一点同色辉光（设计稿）
            let tone = devices
                .iter()
                .find(|device| device.id == fingerprint)
                .map(|device| theme::device_tone(device.kind))
                .unwrap_or(theme::dark::NODE_UNKNOWN);
            let glow = Rgba::new(tone.red, tone.green, tone.blue, 0.6);

            div()
                .py(px(PAIR_PAD_Y))
                .px(px(PAIR_PAD_X))
                .rounded(px(theme::RADIUS_CARD))
                .bg(t.hover_bg)
                .hover(move |style| style.bg(t.bar_bg))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .flex_none()
                        .w(px(8.0))
                        .h(px(8.0))
                        .rounded(px(4.0))
                        .bg(tone)
                        .shadow(vec![gpui::BoxShadow {
                            color: theme::hsla(glow),
                            offset: gpui::point(px(0.0), px(0.0)),
                            blur_radius: px(6.0),
                            spread_radius: px(0.0),
                            inset: false,
                        }]),
                )
                    .child(middle)
                    .child(actions)
            })
            .collect::<Vec<_>>(),
        )
        .into_any_element()
}

const PAIR_DEL_NOTE: &str = "删除后，下次连接要重新配对。";

/// 改名输入框：预填当前名字，回车保存、Esc 取消、退格删除。
///
/// 打印字符（含中文 IME）**不在这里接**：注册了输入处理器之后它们走系统文本栈
/// （`canvas` 的 paint 回调里 `window.handle_input`），回到 `AppRoot` 的
/// `EntityInputHandler`。这里的 canvas 尺寸为 0 但铺满输入框，
/// 它唯一的职责是把输入框的 bounds 报给平台（IME 候选窗要定位）。
fn rename_input(t: &Theme, ctx: &RenameCtx, cx: &mut Context<AppRoot>) -> Div {
    let brand = accent(t);
    div()
        .flex_1()
        .min_w(px(0.0))
        .h(px(24.0))
        .px(px(8.0))
        .rounded(px(theme::RADIUS_BUTTON))
        .border_1()
        .border_color(theme::hsla(t.hairline))
        .bg(t.bar_bg)
        .flex()
        .flex_row()
        .items_center()
        .track_focus(&ctx.focus)
        // 获得焦点时描边走主色（设计稿 `.pair-input:focus { border-color: accent 45% }`）：
        // 输入框画出来不等于看得出它在接受输入
        .focus(move |s| s.border_color(theme::hsla(with_alpha(brand, 0.45))))
        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
            if this.rename_key(&event.keystroke.key) {
                // 拦下冒泡：Esc 不许把整个设置面板一起关掉
                cx.stop_propagation();
                cx.notify();
            }
        }))
        .child(
            div()
                .min_w(px(0.0))
                .flex_1()
                .text_size(px(FS_BODY))
                .text_color(t.text_primary)
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(cx.entity().read(cx).rename_text())),
        )
        .child(
            canvas(
                |_, _, _| (),
                {
                    let entity = ctx.entity.clone();
                    let focus = ctx.focus.clone();
                    let bounds_slot = ctx.bounds.clone();
                    move |bounds: Bounds<Pixels>, _, window, cx| {
                        *bounds_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(bounds);
                        window.handle_input(
                            &focus,
                            gpui::ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
}

/// 16 位指纹 → 8 位短码（与面板/信任列表同一条规则）。
fn short(fingerprint: &str) -> String {
    fingerprint.chars().take(8).collect()
}

/// 主题分段控件：跟随系统 / 深色 / 浅色，滑块色块落在选中档上。
fn seg_theme(t: &Theme, current: ThemeChoice, seg_pos: f32, cx: &mut Context<AppRoot>) -> Div {
    let options = [
        (ThemeChoice::System, "跟随系统"),
        (ThemeChoice::Dark, "深色"),
        (ThemeChoice::Light, "浅色"),
    ];
    let item_w = (SEG_FULL_W - SEG_PAD * 2.0) / options.len() as f32;
    let brand = accent(t);

    let mut seg = div()
        .relative()
        .w_full()
        .p(px(SEG_PAD))
        .flex()
        .flex_row()
        .rounded(px(theme::RADIUS_BUTTON))
        .bg(t.bar_bg)
        // 滑块：绝对定位在底层，位置由 `seg_pos`（浮点档位）给——
        // 点击后 200ms 内滑过去，不是跳过去
        .child(
            div()
                .absolute()
                .top(px(SEG_PAD))
                .bottom(px(SEG_PAD))
                .left(px(SEG_PAD + item_w * seg_pos))
                .w(px(item_w))
                .rounded(px(theme::RADIUS_ICONBUTTON))
                .bg(with_alpha(brand, 0.3)),
        );
    for (choice, label) in options {
        let is_selected = choice == current;
        seg = seg.child(
            div()
                .id(ElementId::Name(format!("theme-{:?}", choice).into()))
                .relative()
                .flex_1()
                .py(px(4.0))
                .cursor_pointer()
                .rounded(px(theme::RADIUS_ICONBUTTON))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_theme(choice);
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .justify_center()
                        .text_size(px(FS_BODY))
                        .font_weight(if is_selected {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if is_selected { ink(t) } else { t.text_secondary })
                        .child(SharedString::from(label)),
                ),
        );
    }
    seg
}

/// 接收目录行：整行可点（调系统目录选择器）。
fn dir_row(
    t: &Theme,
    receive_dir: Option<&str>,
    cx: &mut Context<AppRoot>,
) -> Stateful<Div> {
    let display = receive_dir.map(dir_name).unwrap_or_else(|| "—".into());
    let hover = t.hover_bg;
    let fg = t.text_primary;
    div()
        .id("settings-dir-row")
        .group("settings-dir-row")
        .min_h(px(28.0))
        .py(px(4.0))
        .px(px(8.0))
        .mx(px(-8.0))
        .rounded(px(theme::RADIUS_ICONBUTTON))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .hover(move |s| s.bg(hover))
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            this.choose_receive_dir();
            cx.notify();
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    gpui::svg()
                        .path("icons/folder.svg")
                        .w(px(12.0))
                        .h(px(12.0))
                        .text_color(t.text_muted),
                )
                .child(label_el(t, "接收目录")),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .min_w(px(0.0))
                .child(
                    div()
                        .min_w(px(0.0))
                        .max_w(px(150.0))
                        .text_size(px(FS_BODY))
                        .text_color(t.text_muted)
                        .whitespace_nowrap()
                        .text_ellipsis()
                        // 悬停时值也跟着提亮（设计稿 `.dir-row:hover .row__value`）：
                        // 整行是一个入口，指针进来时两段文字一起变，不像只有一半活着
                        .group_hover("settings-dir-row", move |s| s.text_color(fg))
                        .child(SharedString::from(display)),
                )
                .child(
                    gpui::svg()
                        .path("icons/chevron.svg")
                        .w(px(12.0))
                        .h(px(12.0))
                        .text_color(t.text_muted),
                ),
        )
}

/// 接收目录的显示：**只给目录名**（设计稿 `当前目录名`）——
/// 路径太长会挤掉右箭头，而整行本身就是一个可点的入口，用户认名字比认前缀快。
fn dir_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// 隐私承诺行：点开标题栏盾牌那张浮层（内容在 `model::privacy`，这里只是另一个入口）。
fn privacy_row(t: &Theme, cx: &mut Context<AppRoot>) -> Stateful<Div> {
    div()
        .id("settings-privacy-row")
        .min_h(px(28.0))
        .py(px(4.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            // 设置面板先收起：承诺浮层是全窗的事，别让两层玻璃叠着抢注意力
            this.close_settings();
            this.toggle_privacy();
            cx.notify();
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    gpui::svg()
                        .path("icons/shield.svg")
                        .w(px(12.0))
                        .h(px(12.0))
                        .text_color(t.text_muted),
                )
                .child(label_el(t, "隐私承诺")),
        )
        .child(
            gpui::svg()
                .path("icons/chevron.svg")
                .w(px(12.0))
                .h(px(12.0))
                .text_color(t.text_muted),
        )
}

/// 开关：34×20，滑块白圆点。开 = 品牌色底 + 一点辉光；关 = 灰条。
/// 两个信号（位置 + 颜色）同时给：只靠颜色在灰底上读不出来（标题栏开关同一条经验）。
fn switch(
    t: &Theme,
    id: &'static str,
    on: bool,
    on_toggle: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let brand = accent(t);
    let knob_travel = SWITCH_W - SWITCH_PAD * 2.0 - SWITCH_KNOB;
    div()
        .id(ElementId::from(id))
        .w(px(SWITCH_W))
        .h(px(SWITCH_H))
        .flex_none()
        .relative()
        .rounded(px(SWITCH_H / 2.0))
        .cursor_pointer()
        .on_click(on_toggle)
        .map(|el| {
            if on {
                el.bg(brand).shadow(vec![gpui::BoxShadow {
                    color: theme::hsla(with_alpha(brand, 0.45)),
                    offset: gpui::point(px(0.0), px(0.0)),
                    blur_radius: px(10.0),
                    spread_radius: px(0.0),
                    inset: false,
                }])
            } else {
                el.bg(t.bar_bg)
            }
        })
        .child(
            div()
                .absolute()
                .top(px(SWITCH_PAD))
                .left(px(SWITCH_PAD + if on { knob_travel } else { 0.0 }))
                .w(px(SWITCH_KNOB))
                .h(px(SWITCH_KNOB))
                .rounded(px(SWITCH_KNOB / 2.0))
                .bg(gpui::Rgba::new(1.0, 1.0, 1.0, 0.85))
                .shadow(vec![gpui::BoxShadow {
                    color: theme::hsla(Rgba::new(0.0, 0.0, 0.0, 0.25)),
                    offset: gpui::point(px(0.0), px(1.0)),
                    blur_radius: px(2.0),
                    spread_radius: px(0.0),
                    inset: false,
                }]),
        )
}

/// 设置页的小按钮（设计稿 `.minibtn`）：22 高、圆角 8、1px 描边、正文档字号。
///
/// 与卡片里那颗 `card::mini_button` **刻意不复用**：卡片按旧设计稿用 9px 元信息档，
/// 设置页按新版 tokens 用 13px 正文档——复用会把卡片一起顶大。
fn mini_button(
    t: &Theme,
    id: &str,
    label: &str,
    accent_btn: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    let brand = accent(t);
    let ink = if accent_btn { ink(t) } else { t.text_secondary };
    let border = if accent_btn {
        with_alpha(brand, 0.4)
    } else {
        t.hairline
    };
    let hover = t.hover_bg;
    let strong = t.text_primary;
    div()
        .id(ElementId::Name(format!("settings-btn-{id}").into()))
        .flex()
        .flex_row()
        .items_center()
        .h(px(22.0))
        .px(px(10.0))
        .rounded(px(theme::RADIUS_BUTTON))
        .border(px(1.0))
        .border_color(theme::hsla(border))
        .cursor_pointer()
        .text_size(px(FS_BODY))
        .text_color(ink)
        .hover(move |s| s.bg(hover).text_color(strong))
        .child(SharedString::from(label.to_string()))
        .on_click(on_click)
}
