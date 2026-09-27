//! View：顶部栏 44px —— 品牌点 + 产品名 + 两个图标按钮。

use gpui::{
    div, prelude::*, px, Animation, AnimationExt, BoxShadow, ElementId, IntoElement,
    ParentElement, Rgba, SharedString, Styled, Window, WindowControlArea,
};

use std::time::Duration;

use crate::theme::{self, Theme};

/// 品牌点的呼吸：圆点与它的辉光一起涨落（萤火虫的亮度是「体量」在变，不只是雾在变）。
///
/// 周期与中心光圈**刻意不同**（1.4s vs 3.0s）：两处同拍会显得整窗在集体抽搐，
/// 错相才像两只各飞各的虫子。
const BRAND_PERIOD_MS: u64 = 2400;
/// 圆点直径：8px 是设计稿的值，呼吸时往下收，不往上长（长出去会把标题挤动）。
const BRAND_DOT_MIN: f32 = 6.0;
const BRAND_DOT_MAX: f32 = 8.0;
/// 辉光半径与不透明度：**紧、但不淡**。
/// 靠加大 `spread_radius` 让光晕贴住圆点（实心一点、离得近），
/// 而不是把 blur 拉大——blur 一拉大就变成一块发光的雾，看不出是个点。
const BRAND_GLOW_MIN: f32 = 4.0;
const BRAND_GLOW_MAX: f32 = 10.0;
/// 光晕的扩散：让光紧贴圆点（0 → 1.5px 随呼吸一起长）。
const BRAND_SPREAD_MIN: f32 = 0.0;
const BRAND_SPREAD_MAX: f32 = 1.5;
const BRAND_GLOW_ALPHA_MIN: f32 = 0.35;
const BRAND_GLOW_ALPHA_MAX: f32 = 0.95;
/// 占位方块尺寸：**固定**，等于圆点的最大直径。
/// 只有占位固定，圆点在内部缩放才不会每帧把标题推来推去。
const BRAND_SLOT: f32 = 8.0;
/// 顶部栏的**拖动区**：品牌点 + 产品名，占满左侧剩余宽度。
///
/// 右侧的按钮簇**不在**这个拖动区里——否则拖动命中区会盖住按钮，
/// 点关闭变成拖窗（gpui-ce 的 `WM_NCHITTEST` 回调是「第一个命中的区域胜出」，
/// 父元素先于子元素入表）。
pub fn titlebar(t: &Theme) -> impl IntoElement {
    div()
        .id(ElementId::from("titlebar-drag"))
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
            // macOS 靠这句拖窗（内部是 `performWindowDragWithEvent`）；
            // Windows 是 `HTCAPTION` 生效，这里空跑一次也无害。
            window.start_window_move();
        })
        .child(brand_dot(t))
        .child(
            div()
                .min_w(px(0.0))
                .text_size(px(theme::TEXT_BRAND))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.text_primary)
                // 标题栏写**产品名**，不写设备名：设备名在中心节点的标签上
                .child(SharedString::new_static(theme::BRAND_NAME)),
        )
}

/// 品牌点：圆点 + 辉光，**两者一起呼吸**。
///
/// 恒亮的一个点读起来像指示灯，不像活的；产品名就叫萤光，这里就该是一只虫子。
/// 外层 8px 方块只负责占位（布局不参与呼吸），真正在缩放的是里层的圆点。
/// 里层靠 **flex 居中**而不是 `left/top` 算偏移：算偏移要自己除 2，
/// 一有取整误差就不是「原地缩放」而是「边缩边跑」。
fn brand_dot(t: &Theme) -> impl IntoElement {
    let dot = theme::NODE_SELF;
    // 色先取出来：动画回调要求 `'static`，闭包里不能持有 `&Theme`
    let glow_ink = t.icon_self;

    div()
        .w(px(BRAND_SLOT))
        .h(px(BRAND_SLOT))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(div().with_animation(
            ElementId::from("brand-dot"),
            Animation::new(Duration::from_millis(BRAND_PERIOD_MS))
                .repeat()
                .with_easing(|delta| (1.0 - (delta * std::f32::consts::PI * 2.0).cos()) * 0.5),
            move |el, delta| {
                let size = BRAND_DOT_MIN + (BRAND_DOT_MAX - BRAND_DOT_MIN) * delta;
                let blur = BRAND_GLOW_MIN + (BRAND_GLOW_MAX - BRAND_GLOW_MIN) * delta;
                let spread = BRAND_SPREAD_MIN + (BRAND_SPREAD_MAX - BRAND_SPREAD_MIN) * delta;
                let alpha =
                    BRAND_GLOW_ALPHA_MIN + (BRAND_GLOW_ALPHA_MAX - BRAND_GLOW_ALPHA_MIN) * delta;
                let color = Rgba::new(glow_ink.red, glow_ink.green, glow_ink.blue, alpha);
                el.w(px(size))
                    .h(px(size))
                    .rounded(px(size / 2.0))
                    .bg(dot)
                    .shadow(vec![BoxShadow {
                        color: theme::hsla(color),
                        offset: gpui::point(px(0.0), px(0.0)),
                        blur_radius: px(blur),
                        spread_radius: px(spread),
                        inset: false,
                    }])
            },
        ))
}

/// 图标按钮：22px 方块、6px 圆角、14px 图标（`text_color` 必填，否则 SVG 不画）。
///
/// `control` 是给平台层的窗口控制语义（`Close` 之类）：Windows 会据此返回
/// `HTCLOSE`，让系统给出原生 hover/点击行为；macOS 不用它（用 `on_click`）。
/// `hover` 决定鼠标经过的观感（见 `theme::IconHover`）；`ink` 是常态墨色
/// （服务开关靠它上色：开 = 品牌蓝）。
pub fn icon_button(
    id: &'static str,
    t: &Theme,
    icon: &'static str,
    ink: Rgba,
    control: Option<WindowControlArea>,
    hover: theme::IconHover,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let (hover_bg, hover_ink) = t.icon_hover(hover);
    // 分组名用按钮 id：每个按钮一个组，组 hover 才能把墨色传到子元素的 svg 上
    let group = SharedString::from(id);

    let button = div()
        .id(ElementId::from(id))
        .group(group.clone())
        .w(px(22.0))
        .h(px(22.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS_ICONBUTTON))
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .on_click(on_click);
    let button = match control {
        Some(area) => button.window_control_area(area),
        None => button,
    };
    button.child(
        gpui::svg()
            .path(icon)
            .w(px(14.0))
            .h(px(14.0))
            .text_color(ink)
            // `svg()` **不继承**父级 `text_color`，hover 的墨色只能在 svg 上给
            .group_hover(group, move |s| s.text_color(hover_ink)),
    )
}

/// 遮住整窗的 1px 内描边（设计稿的「内侧描边」：不占布局）。
pub fn window_border(t: &Theme) -> impl IntoElement {
    let color: Rgba = t.window_border;
    div()
        .absolute()
        .left(px(0.0))
        .top(px(0.0))
        .w(px(theme::WINDOW_W))
        .h_full()
        .border_1()
        .border_color(color)
        .rounded(px(theme::WINDOW_RADIUS))
}
