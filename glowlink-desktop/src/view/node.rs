//! View：节点（本机中心盘 84px，设备盘 52px）。
//!
//! 四层同心结构（从外到内）：呼吸光圈 → 外环（1.5/2px tone 色 + **零模糊**辉光）
//! → 盘（内缩 4/6px，inset 双向浮雕）→ 图标。
//!
//! GPUI 没有 radial gradient，辉光用**独立的同心圆 + 零偏移 box-shadow**画，
//! 不要拿投影冒充（投影有偏移，看起来像"浮起"而不是"发光"）。

use gpui::{
    div, prelude::*, px, Animation, AnimationExt, BoxShadow, Context, Div, ElementId, FontWeight,
    IntoElement, Render, Rgba, SharedString, Stateful, Styled, Window,
};

use std::time::Duration;

use crate::model::drag::{GHOST_SCALE, GHOST_SCALE_MIN};
use crate::model::layout::{NodeSize, Rect};
use crate::theme::{self, Theme};

/// 呼吸：**大小与不透明度一起推进**。
///
/// 只做明暗变化是不行的：光圈是 1px 的环、色本身就只有 20% alpha（`colors.halo`），
/// 明暗一变化在玻璃上几乎看不出来，看起来就是个静态圆环。
/// 所以让它**扩张 + 淡出**（像一次呼吸），这才是设计稿说的「光会动，但不闪」。
const HALO_PERIOD_MS: u64 = 3000;
/// 呼吸过程里外圈再向外扩多少像素（起始外径 = 直径 + 16，即 `haloInset: -8px`）。
const HALO_MAX_GROW: f32 = 12.0;

/// 漂浮：外围设备在慢速椭圆轨道上来回漂（设计稿 `drift 7s`）。
///
/// 幅度取自设计稿关键帧：横向 +3.5 / −2.5、纵向 −4.5 / +3。
/// 我们只用一个正弦对（sin/cos 就是一圈椭圆），所以取对称的极值包住原区间：
/// 横向 ±3.0、纵向 ±3.75。（设计稿还带 -1.2° 旋转，gpui-ce 没有 rotate，这一项舍去。）
const FLOAT_PERIOD_MS: u64 = 7000;
const FLOAT_AMP_X: f32 = 3.0;
const FLOAT_AMP_Y: f32 = 3.75;

fn breath() -> Animation {
    // 设计稿：呼吸 3s（传输中加快到 1.5s，等有传输态再按状态给不同周期）。
    // `with_animation` 会自动尊重系统的「减少动态效果」（`App::reduce_motion` 时停在起始态）。
    Animation::new(Duration::from_millis(HALO_PERIOD_MS))
        .repeat()
        .with_easing(|delta| (1.0 - (delta * std::f32::consts::PI * 2.0).cos()) * 0.5)
}

/// 漂浮用**线性**推进：自己取正弦，缓动在这里没意义（要的就是匀速转圈）。
fn drift() -> Animation {
    Animation::new(Duration::from_millis(FLOAT_PERIOD_MS)).repeat()
}

/// 把子元素放进一个绝对定位的容器里，每帧按 `pos(delta)` 摆位置。
///
/// 这是本文件唯一允许「动画改位置」的地方：`with_animation` 只能改它**自己**的样式，
/// 而它的回调是 `Fn`（不能把子元素移进去），所以必须是
/// 「先 `.child(...)` 建容器，再 `.with_animation(...)` 动容器」这个顺序。
pub fn positioned<E: IntoElement + 'static>(
    child: E,
    id: ElementId,
    animation: Animation,
    pos: impl Fn(f32) -> (f32, f32) + 'static,
) -> impl IntoElement {
    div().absolute().child(child).with_animation(id, animation, move |el, delta| {
        let (x, y) = pos(delta);
        el.left(px(x)).top(px(y))
    })
}

/// 漂浮动画（本机不套：它是图面的锚点）。
pub fn drift_animation() -> Animation {
    drift()
}

/// 漂浮动画的 id（与呼吸圈、各节点都不能撞）。
pub fn drift_id(rect: Rect) -> ElementId {
    ElementId::NamedInteger(
        SharedString::new_static("drift"),
        ((rect.x as u64) << 32) | rect.y as u64,
    )
}

/// 第 `delta` 帧的漂浮位置：以原位为中心的慢椭圆（错相由 `layout::place_devices` 给）。
pub fn drift_offset(rect: Rect, phase: f32, delta: f32) -> (f32, f32) {
    let theta = (delta + phase) * std::f32::consts::PI * 2.0;
    (rect.x + FLOAT_AMP_X * theta.sin(), rect.y + FLOAT_AMP_Y * theta.cos())
}

/// 呼吸圈的动画 id：同一帧里可能有多个圈（本机 + 拖拽目标），必须各不相同。
/// 用节点位置当后缀：同一个节点跨帧保持同一个 id，动画才不会每帧重头开始。
fn halo_id(rect: Rect) -> ElementId {
    ElementId::NamedInteger(
        SharedString::new_static("halo"),
        ((rect.x as u64) << 32) | rect.y as u64,
    )
}

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
    pub rect: Rect,
    pub who: NodeWho,
    pub label: &'a str,
    pub look: NodeLook,
    /// 缩放（拖影 1.08；正常 1.0）。
    pub scale: f32,
}
/// 节点元素：盘 + 环 + 图标 + 名字。
///
/// 返回 `Stateful<Div>`（**带 id 才算「可交互元素」**）：拖动、点击、hover
/// 都要求它；而漂浮动画改由调用方用 [`drifting`] 套上——两者顺序不能反。
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
        // 起始外径 = 直径 + 16（两边各 −8px），呼吸时以中心为轴向外长
        let span = diameter + 16.0;
        // 色先取出来：动画回调要求 `'static`，闭包里不能持有 `&Theme`
        let halo_color = t.halo;
        div().absolute().with_animation(halo_id(spec.rect), breath(), move |el, delta| {
            let grow = HALO_MAX_GROW * delta;
            let size = span + grow;
            el.left(px(-8.0 - grow / 2.0))
                .top(px(-8.0 - grow / 2.0))
                .w(px(size))
                .h(px(size))
                .rounded(px(size / 2.0))
                .border_1()
                .border_color(halo_color)
                // 越扩越淡：扩张到最大时正好淡到 0，下一轮从 1 重新开始
                .opacity(1.0 - delta)
        })
    });

    let base = div()
        .absolute()
        // 先给一个静态坐标：动画每帧会覆盖它，但不能没有——
        // 没有的话在「动画未参与/被 reduce_motion 冻结」的那一帧会跑到 (0,0)。
        .left(px(spec.rect.x))
        .top(px(spec.rect.y))
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
    base.id(ElementId::NamedInteger(
        SharedString::new_static("node"),
        ((spec.rect.x as u64) << 32) | spec.rect.y as u64,
    ))
}

/// 拖动时的「拖影」：跟着指针走的那一枚。
///
/// gpui 把拖影当成**独立的一层**画在最上面，并由它自己摆位置（拖动源的原点 +
/// 指针位移），所以这里从 (0,0) 开始画即可。
pub struct DragGhost {
    pub name: String,
    pub kind: glowlink_core::DeviceKind,
}

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::DARK;
        let (name, kind) = (self.name.clone(), self.kind);
        div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .with_animation(
                ElementId::Name("ghost-appear".into()),
                // 一次性 140ms：抓起时轻轻放大一下。id 固定，所以只播一次——
                // 每次 Render 都重播的话，就是每帧重启动画（看着像卡住）
                Animation::new(Duration::from_millis(140))
                    .with_easing(gpui::ease_out_quint()),
                move |el, delta| {
                    let scale = GHOST_SCALE_MIN + (GHOST_SCALE - GHOST_SCALE_MIN) * delta;
                    el.opacity(0.65 + 0.35 * delta).child(node(
                        &t,
                        NodeSpec {
                            rect: Rect { x: 0.0, y: 0.0, size: 0.0 },
                            who: NodeWho::Device(kind),
                            label: &name,
                            look: NodeLook::device_default(),
                            scale,
                        },
                    ))
                },
            )
    }
}
