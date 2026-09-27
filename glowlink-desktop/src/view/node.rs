//! View：节点（本机中心盘 84px，设备盘 52px）。
//!
//! 四层同心结构（从外到内）：呼吸光圈 → 外环（1.5/2px tone 色 + **零模糊**辉光）
//! → 盘（内缩 4/6px，inset 双向浮雕）→ 图标。
//!
//! GPUI 没有 radial gradient，辉光用**独立的同心圆 + 零偏移 box-shadow**画，
//! 不要拿投影冒充（投影有偏移，看起来像"浮起"而不是"发光"）。

use gpui::{
    div, prelude::*, px, BoxShadow, Context, Div, ElementId, FontWeight,
    IntoElement, Render, Rgba, SharedString, Stateful, Styled, Window,
};


use crate::model::layout::NodeSize;
use crate::theme::{self, Theme};

/// 呼吸：**大小与不透明度一起推进**。
///
/// 只做明暗变化是不行的：光圈是 1px 的环、色本身就只有 20% alpha（`colors.halo`），
/// 明暗一变化在玻璃上几乎看不出来，看起来就是个静态圆环。
/// 所以让它**扩张 + 淡出**（像一次呼吸），这才是设计稿说的「光会动，但不闪」。
/// 呼吸过程里外圈再向外扩多少像素（起始外径 = 直径 + 16，即 `haloInset: -8px`）。
const HALO_MAX_GROW: f32 = 12.0;

/// 呼吸的相位 → 扩张量与不透明度（纯函数，好测）。
///
/// 相位 `0..1` 走一圈：0 时最小最亮，1 时最大最淡。
pub fn halo_span(phase: f32) -> (f32, f32) {
    let ease = (1.0 - (phase * std::f32::consts::TAU).cos()) * 0.5;
    (HALO_MAX_GROW * ease, 1.0 - ease)
}

/// 呼吸一圈要多久（秒）。
pub const HALO_PERIOD_SECS: f32 = 3.0;

/// 把子元素摆到**当前**位置上（绝对定位，不套任何动画）。
///
/// 位置由 `model::motion` 每帧算好，渲染层只负责画。这里不再有「一次性过渡动画」，
/// 也就没有「动画 id 该带哪些状态」这类只在运行时才暴露的坑（我们为此栽过两次）。
///
/// 收的是**圆心**、自己减半个尺寸：GPUI 用左上角定位，而全项目的几何都是圆心
/// （命中判定、碰撞、槽位）。换算只留在这一处——曾经有人在调用点先减一次、
/// 也有人直接当左上角用，于是「中心盘一上来就跑偏」。
pub fn placed<E: IntoElement + 'static>(
    child: E,
    center: crate::model::layout::Point,
    size: f32,
) -> impl IntoElement {
    div()
        .absolute()
        .left(px(center.x - size / 2.0))
        .top(px(center.y - size / 2.0))
        .child(child)
}

/// 呼吸圈的动画 id：从节点自己的 id 派生。
///
/// 辉光：零模糊偏移，只有扩散。
fn glow(color: Rgba, radius: f32) -> BoxShadow {
    BoxShadow {
        color: theme::hsla(color),
        offset: gpui::point(px(0.0), px(0.0)),
        blur_radius: px(radius),
        spread_radius: px(0.0),
        inset: false,
    }
}

/// 新拟物浮雕：**inset 双向**——上左高光 + 下右阴影（浅色下方向相反）。
fn relief(t: &Theme, offset: f32, blur: f32) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: theme::hsla(t.relief_hi),
            offset: gpui::point(px(offset), px(offset)),
            blur_radius: px(blur),
            spread_radius: px(0.0),
            inset: true,
        },
        BoxShadow {
            color: theme::hsla(t.relief_lo),
            offset: gpui::point(px(-offset), px(-offset)),
            blur_radius: px(blur),
            spread_radius: px(0.0),
            inset: true,
        },
    ]
}

/// 节点外观的可变部分（默认态 → hover → 选中 → 传输目标）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeLook {
    /// 外环的不透明度（设计稿：默认 50%，hover 78%，选中/目标 100%）。
    pub ring_alpha: f32,
    /// 辉光半径与不透明度。
    pub glow_radius: f32,
    pub glow_alpha: f32,
    /// 盘上染的 tone 比例（选中态用）。
    pub disc_tint: f32,
    /// 是否显示呼吸光圈。
    pub halo: bool,
    /// 盘相对节点的内缩（本机 6px，设备 4px）。
    pub disc_inset: f32,
    /// 覆盖辉光色。
    ///
    /// 只有一种用法：把设备拖到本机头上时，中心盘染成**那台设备的色**——
    /// 「正在把哪一台拖过来」不用另开颜色通道，用现有的四个语义色就说清了。
    pub tone_override: Option<Rgba>,
}

impl NodeLook {
    /// 设备的默认态。
    pub fn device_default() -> Self {
        NodeLook {
            ring_alpha: 0.5,
            glow_radius: 16.0,
            glow_alpha: 0.2,
            disc_tint: 0.0,
            halo: false,
            disc_inset: 4.0,
            tone_override: None,
        }
    }

    pub fn device_hover() -> Self {
        NodeLook {
            ring_alpha: 0.78,
            glow_radius: 20.0,
            glow_alpha: 0.34,
            ..Self::device_default()
        }
    }

    pub fn device_selected() -> Self {
        NodeLook {
            ring_alpha: 1.0,
            glow_radius: 22.0,
            glow_alpha: 0.46,
            disc_tint: 0.16,
            ..Self::device_default()
        }
    }

    /// 传输中**交替高亮**的一帧：`t` = 0 暗、1 亮（由 `transfer::pulse` 给）。
    ///
    /// 在「已连接」与「亮起来」之间插值：环从 0.5 到 1.0、辉光从 16/0.2 到 26/0.55。
    /// 幅度拉这么大是为了**读得出来**——「在传」没有专属颜色，全靠这一亮一暗说。
    pub fn device_pulse(t: f32) -> Self {
        let base = Self::device_connected();
        Self {
            ring_alpha: base.ring_alpha + (1.0 - base.ring_alpha) * t,
            glow_radius: base.glow_radius + (26.0 - base.glow_radius) * t,
            glow_alpha: base.glow_alpha + (0.55 - base.glow_alpha) * t,
            halo: true,
            ..base
        }
    }

    /// 本机盘在交替高亮里的一帧（与设备同款起伏，两边一起跳才读得出「这一对」）。
    pub fn self_pulse(t: f32) -> Self {
        let base = Self::self_node();
        Self {
            ring_alpha: base.ring_alpha + (1.0 - base.ring_alpha) * t,
            glow_radius: base.glow_radius + (28.0 - base.glow_radius) * t,
            glow_alpha: base.glow_alpha + (0.6 - base.glow_alpha) * t,
            ..base
        }
    }

    /// 传输中（设计稿：环拉满、辉光 24px / 52%、光圈呼吸加快到 1.5s）。
    ///
    /// 颜色仍归设备类型——「在传」不是第五种颜色，它靠**亮度与节奏**说。
    pub fn device_transfer() -> Self {
        NodeLook {
            ring_alpha: 1.0,
            glow_radius: 24.0,
            glow_alpha: 0.52,
            halo: true,
            ..Self::device_default()
        }
    }

    /// 传输时本机盘的样子（与设备同款高亮：两端一起亮，才读得出「这一对在传」）。
    pub fn self_transfer() -> Self {
        NodeLook { halo: true, ..Self::self_node() }
    }

    /// 已连接：**保留高亮**（环拉满 + 辉光加强 + 常驻呼吸）。
    ///
    /// 不做「变色」：已连接不是第五种状态色，它是「这台我认识」——
    /// 颜色继续归设备类型（四色语义不被动摇）。
    pub fn device_connected() -> Self {
        NodeLook {
            ring_alpha: 1.0,
            glow_radius: 22.0,
            glow_alpha: 0.46,
            halo: true,
            ..Self::device_default()
        }
    }

    pub fn device_target() -> Self {
        NodeLook {
            ring_alpha: 1.0,
            glow_radius: 24.0,
            glow_alpha: 0.52,
            halo: true,
            ..Self::device_default()
        }
    }

    /// 本机盘被「某台设备正拖过来」时的样子：由 `t`（0→1 过渡值）插值得到。
    ///
    /// `tone` 是被拖那台的辉光色：越接近命中，中心盘越染上它的色。
    /// 除了「环拉满 + 辉光加强」，**色的变化**才是「松手就会连上它」的直觉提示。
    pub fn self_node_blended(t: f32, tone: Rgba) -> Self {
        let base = Self::self_node();
        NodeLook {
            ring_alpha: base.ring_alpha + (1.0 - base.ring_alpha) * t,
            glow_radius: base.glow_radius + (30.0 - base.glow_radius) * t,
            glow_alpha: base.glow_alpha + (0.5 - base.glow_alpha) * t,
            disc_tint: 0.16 * t,
            tone_override: Some(mix(theme::NODE_SELF, tone, t)),
            ..base
        }
    }

    /// 本机中心盘：环 2px、内缩 6px、常驻呼吸。
    pub fn self_node() -> Self {
        NodeLook {
            ring_alpha: 0.5,
            glow_radius: 24.0,
            glow_alpha: 0.35,
            disc_tint: 0.0,
            halo: true,
            disc_inset: 6.0,
            tone_override: None,
        }
    }
}

/// 两个色之间线性插值（`t` = 0 取 `a`，1 取 `b`）。
pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    Rgba::new(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
        a.alpha + (b.alpha - a.alpha) * t,
    )
}

fn with_alpha(color: Rgba, alpha: f32) -> Rgba {
    Rgba::new(color.red, color.green, color.blue, color.alpha * alpha)
}

fn tinted(base: Rgba, tone: Rgba, ratio: f32) -> Rgba {
    Rgba::new(
        base.red * (1.0 - ratio) + tone.red * ratio,
        base.green * (1.0 - ratio) + tone.green * ratio,
        base.blue * (1.0 - ratio) + tone.blue * ratio,
        base.alpha,
    )
}

/// 这枚节点是谁：本机，还是某类设备。
///
/// 色、图标、尺寸都从它派生。之前是调用方分别传 `size` 与 `tone`，两者
/// 理论上可以不一致（改一处忘一处就会出现「粉色的手机图标装在 84px 盘上」），
/// 现在不可能不一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeWho {
    /// 本机中心盘（品牌蓝、显示器图标、22px、环 2px）。
    SelfNode,
    /// 周围设备：色按类型、图标按类型、尺寸**按外轮廓宽度补偿**。
    Device(glowlink_core::DeviceKind),
}

impl NodeWho {
    fn is_self(self) -> bool {
        matches!(self, NodeWho::SelfNode)
    }

    fn node_size(self) -> NodeSize {
        if self.is_self() { NodeSize::SelfNode } else { NodeSize::Device }
    }

    fn tone(self) -> Rgba {
        match self {
            NodeWho::SelfNode => theme::NODE_SELF,
            NodeWho::Device(kind) => theme::device_tone(kind),
        }
    }

    /// 图标路径：本机显示器；设备按类型（平板/手机/台式机/未知各一枚）。
    fn icon(self) -> &'static str {
        use glowlink_core::DeviceKind;
        match self {
            NodeWho::SelfNode => "icons/monitor.svg",
            NodeWho::Device(DeviceKind::Phone) => "icons/phone.svg",
            NodeWho::Device(DeviceKind::Tablet) => "icons/tablet.svg",
            NodeWho::Device(DeviceKind::Desktop) => "icons/desktop.svg",
            NodeWho::Device(DeviceKind::Laptop) => "icons/laptop.svg",
            NodeWho::Device(DeviceKind::Unknown) => "icons/unknown.svg",
        }
    }

    /// 图标盒尺寸：**按外轮廓宽度补偿**（设计稿 node-self/phone/tablet/desktop 的 iconSize）。
    ///
    /// 统一的像素数 != 统一的视觉重量：手机机身最窄要放大到 21，
    /// 显示器横向宽要收到 19，两者看起来才一个分量。
    fn icon_size(self) -> f32 {
        use glowlink_core::DeviceKind;
        match self {
            NodeWho::SelfNode => theme::ICON_SELF,
            NodeWho::Device(DeviceKind::Phone) => theme::ICON_PHONE,
            NodeWho::Device(DeviceKind::Tablet) => theme::ICON_TABLET,
            NodeWho::Device(DeviceKind::Desktop) => theme::ICON_DESKTOP,
            // 笔记本的墨迹是全部图标里**最宽**的（外轮廓占画布 92%，显示器只有 72%），
            // 所以按「外轮廓宽度补偿」落到更小一档：18px 时墨迹高 12px，
            // 与显示器在 19px 时的 12.7px 基本一致。
            NodeWho::Device(DeviceKind::Laptop) => theme::ICON_LAPTOP,
            NodeWho::Device(DeviceKind::Unknown) => theme::ICON_TABLET,
        }
    }
}

/// 画一枚节点：`rect` 是**全窗坐标**。
pub struct NodeSpec<'a> {
    /// 节点自己的 id（同时派生呼吸圈的动画 id）。
    ///
    /// **不带坐标**：位置一律由调用方的容器决定——曾经的坑是「节点内部按 rect 定位、
    /// 外面又套一层同样按 rect 定位的容器」，于是设备被画到两倍坐标上，跑出窗口。
    /// 现在节点根本没有坐标可用，这种写法在类型上就不可能了。
    pub id: ElementId,
    pub who: NodeWho,
    pub label: &'a str,
    pub look: NodeLook,
    /// 缩放（拖影 1.08；正常 1.0）。
    pub scale: f32,
    /// 呼吸相位 `0..1`（由界面时钟推进，见 [`halo_span`]）。
    pub halo: f32,
}
/// 节点元素：盘 + 环 + 图标 + 名字。
///
/// **它自己不摆位置**：位置一律由调用方决定（见 [`placed`]）。
/// 曾经的坑：节点内部按 `rect` 定位、外面又套了一层同样按 `rect` 定位的容器，
/// 于是设备被画到两倍坐标上——320px 宽的窗口装不下，看着就是「设备全没了」。
///
/// 返回 `Stateful<Div>`（**带 id 才算「可交互元素」**）：拖动、点击、hover
/// 都要求它；位置与运动都归调用方和 `model::motion` 管。
pub fn node(t: &Theme, spec: NodeSpec<'_>) -> Stateful<Div> {
    let node_size = spec.who.node_size();
    let tone = spec.look.tone_override.unwrap_or_else(|| spec.who.tone());
    let diameter = node_size.diameter() * spec.scale;
    let radius = diameter / 2.0;
    let disc_size = diameter - spec.look.disc_inset * 2.0;
    let ring_width = if spec.who.is_self() { 2.0 } else { 1.5 };
    let disc_bg = if spec.look.disc_tint > 0.0 {
        tinted(t.disc, tone, spec.look.disc_tint)
    } else {
        t.disc
    };
    let glow_color = with_alpha(tone, spec.look.glow_alpha);
    let ring_color = with_alpha(tone, spec.look.ring_alpha);
    let icon_ink = t.icon_ink(tone);
    let icon_box = spec.who.icon_size() * spec.scale;
    let label_color = if spec.who.is_self() {
        t.text_muted
    } else {
        t.text_secondary
    };
    let label_top = if spec.who.is_self() { 6.0 } else { 6.0 };
    let halo = spec.look.halo.then(|| {
        // 起始外径 = 直径 + 16（两边各 −8px），呼吸时以中心为轴向外长。
        //
        // 呼吸的相位来自 `spec.halo`（由界面时钟每帧推进）：**不用 gpui 的
        // `Animation::repeat()`** —— 那会让框架持续请求整窗重绘（实测空闲态
        // 60fps 重绘吃掉一半 CPU），而这点慢动作 20fps 就够顺了。
        let span = diameter + 16.0;
        let halo_color = t.halo;
        let (grow, opacity) = halo_span(spec.halo);
        let size = span + grow;
        div()
            .absolute()
            .left(px(-8.0 - grow / 2.0))
            .top(px(-8.0 - grow / 2.0))
            .w(px(size))
            .h(px(size))
            .rounded(px(size / 2.0))
            .border_1()
            .border_color(halo_color)
            // 越扩越淡：扩张到最大时正好淡到 0，下一轮从 1 重新开始
            .opacity(opacity)
    });

    let base = div()
        .absolute()
        .w(px(diameter))
        .h(px(diameter))
        .children(halo)
        // 盘：内缩 + inset 双向浮雕
        .child(
            div()
                .absolute()
                .left(px(spec.look.disc_inset))
                .top(px(spec.look.disc_inset))
                .w(px(disc_size))
                .h(px(disc_size))
                .rounded(px(disc_size / 2.0))
                .bg(disc_bg)
                .shadow(relief(t, 2.0, 6.0)),
        )
        // 图标
        .child(
            div()
                .absolute()
                .left(px((diameter - icon_box) / 2.0))
                .top(px((diameter - icon_box) / 2.0))
                .w(px(icon_box))
                .h(px(icon_box))
                .child(
                    gpui::svg()
                        .path(spec.who.icon())
                        .w(px(icon_box))
                        .h(px(icon_box))
                        .text_color(icon_ink),
                ),
        )
        // 外环 + 零模糊辉光
        .child(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .w(px(diameter))
                .h(px(diameter))
                .rounded(px(radius))
                .border(px(ring_width))
                .border_color(ring_color)
                .shadow(vec![glow(glow_color, spec.look.glow_radius)]),
        )
        // 设备名
        .child(
            div()
                .absolute()
                .top(px(diameter + label_top))
                .w(px(diameter + 60.0))
                .left(px(-30.0))
                .flex()
                .justify_center()
                .text_size(px(theme::TEXT_DEVICE_LABEL))
                .font_weight(FontWeight::NORMAL)
                .text_color(label_color)
                .child(SharedString::from(spec.label.to_string())),
        );

    // id 由**初始位置**派生：跨帧必须稳定，否则动画每帧从头开始、拖动也会断。
    // 坐标本身由调用方的漂浮动画每帧覆盖；这里给静态值兜底（没有它，
    // 在「动画未参与 / 被 reduce_motion 冻结」的那一帧会落到 (0,0)，也就是左上角）。
    base.id(spec.id)
}

/// 断开连接的小按钮：贴在节点右上角（只在已连接时出现）。
///
/// 墨色走中性、hover 提亮——按「辉光即语义」，接口类图标不借四色里的任何一个。
/// 返回 `Stateful<Div>` 是为了让调用方接 `on_click`（它才有 `cx`）。
pub fn disconnect_button(t: &Theme, id: ElementId) -> Stateful<Div> {
    div()
        .absolute()
        // 略微探出盘外：贴在盘角上会压住图标
        .right(px(-6.0))
        .top(px(-6.0))
        .w(px(16.0))
        .h(px(16.0))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|style| style.bg(t.hover_bg))
        .child(
            gpui::svg()
                .path("icons/unlink.svg")
                .w(px(12.0))
                .h(px(12.0))
                .text_color(t.text_secondary),
        )
        .id(id)
}

/// 拖动时 gpui 要求挂一个「被拖起来的东西」，但**我们不画它**：
/// 被拖起来的正是那枚节点自己（它在运动系统里 1:1 跟指针），再画一个拖影
/// 等于同一枚盘画两遍。所以这里只回一个零尺寸的占位。
pub struct DragHandle;

impl Render for DragHandle {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(0.0)).h(px(0.0))
    }
}
