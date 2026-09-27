//! View：浮层卡片的**共用外壳**（传输卡与配对卡同一个格式）。
//!
//! 为什么要有这个文件：两张卡原来各写一套定位与样式，于是配对卡停在 `bottom: 12`
//! 被拖拽区盖住、看起来「跑到拖拽按钮下面去了」。**格式只有一个定义就不会再分叉**：
//!
//! - 位置：左右各 `DROPZONE_INSET`（与拖拽区**同宽同边距**）、底边在拖拽区之上 12px
//! - 材质：磨砂玻璃（半透明底 + `backdrop_filter` 20px）+ 圆角 + 投影
//! - 右上角一个圆形 ✕（语义由调用方给：在传 = 取消、结束 = 关闭、配对 = 放弃）
//!
//! 层级：**画在拖拽区之后**（浮层压在拖拽区上，而不是被它盖住）。

use gpui::{div, prelude::*, px, Div, ElementId, Stateful, Styled};

use crate::theme::{self, Theme};

/// 卡片离窗口底边的距离：让开拖拽区 + 12px 缝。
pub const CARD_BOTTOM: f32 = theme::DROPZONE_INSET + theme::DROPZONE_H + 12.0;
/// 左右边距（= 拖拽区的内缩：同一列才对齐）。
pub const CARD_INSET: f32 = theme::DROPZONE_INSET;
/// 磨砂玻璃的模糊半径（设计稿：卡片底 20px backdrop blur）。
const CARD_BLUR: f32 = 20.0;
/// 关闭按钮直径。
pub const CLOSE_SIZE: f32 = 20.0;

/// 卡片外壳：磨砂玻璃 + 圆角 + 投影 + 右上角 ✕，返回可继续往里塞内容的 `Div`。
///
/// ✕ 的语义由调用方决定（同一个位置同一个图标，用户不必学两个按钮）。
pub fn chrome(
    t: &Theme,
    id: &str,
    on_close: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    div()
        // 带 id 才算可交互元素：卡片里的子元素（✕ / 按钮）要靠它区分彼此
        .id(ElementId::Name(id.to_string().into()))
        .absolute()
        .left(px(CARD_INSET))
        .right(px(CARD_INSET))
        .bottom(px(CARD_BOTTOM))
        .p(px(16.0))
        .rounded(px(theme::RADIUS_CARD))
        .bg(t.card_bg)
        // 磨砂玻璃：底下是玻璃窗，这里再叠一层模糊，卡片才像「浮在窗上的另一片玻璃」
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
        .child(close_button(t, id, on_close))
}

/// 右上角的圆形 ✕（hover 变红：会造成中断的动作，必须一眼看出分量）。
fn close_button(
    t: &Theme,
    id: &str,
    on_close: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    let (hover_bg, hover_ink) = t.icon_hover(theme::IconHover::Close);
    let (base_ink, _) = t.icon_hover(theme::IconHover::Neutral);
    div()
        .id(ElementId::Name(format!("{id}-close").into()))
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
        .child(gpui::SharedString::from("✕"))
        .on_click(on_close)
}

/// 标题行右侧要给 ✕ 留出的宽度（免得长文字压到按钮底下）。
pub fn title_reserve() -> f32 {
    CLOSE_SIZE + 6.0
}

/// 卡片里的小按钮：24px 高、8px 圆角、9px 文字，hover 只提亮底色。
///
/// 三张卡（传输 / 发起配对 / 接收配对）都要它，所以放在共用处——
/// 各写一份的话，圆角与高度迟早各偏 1px。
pub fn mini_button(
    t: &Theme,
    id: &str,
    label: &str,
    strong: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    // 「重要动作」与「次要动作」的差别只用**字色**表达（不加第五种颜色）
    let ink = if strong { t.text_primary } else { t.text_secondary };
    div()
        .id(ElementId::Name(format!("card-btn-{id}").into()))
        .flex()
        .flex_row()
        .items_center()
        .h(px(MINI_BTN_H))
        .px(px(10.0))
        .rounded(px(theme::RADIUS_BUTTON))
        .cursor_pointer()
        .hover(|style| style.bg(t.hover_bg))
        .text_size(px(theme::TEXT_META))
        .text_color(ink)
        .child(gpui::SharedString::from(label.to_string()))
        .on_click(on_click)
}

/// 小按钮高度（设计稿 `minibtn` 24px）。
pub const MINI_BTN_H: f32 = 24.0;
