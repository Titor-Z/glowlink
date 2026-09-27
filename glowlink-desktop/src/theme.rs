//! 设计 token：`DESIGN.md` 前言（front matter）在代码里的唯一落地处。
//!
//! **唯一数值来源**。组件里不许出现裸 hex、裸字号——一律引这里，
//! 否则改了 `DESIGN.md` 而界面不动，两边会慢慢漂成两套设计。
//!
//! 为什么不用 `rgb(0x..)` / `rgba(..)`：它们都是普通函数、**不是 const**，
//! 而 `Rgba` 是 `palette::Alpha<Rgb, f32>`（字段是 `color` / `alpha`，
//! 分量名是 `red` / `green` / `blue`），用 `Rgba::new` 才能写进常量。

use gpui::{Hsla, Rgba};

/// 直接构造 `Rgba`（分量是 0..1 的 f32）。
const fn c(r: f32, g: f32, b: f32, a: f32) -> Rgba {
    Rgba::new(r, g, b, a)
}

/// 给 GPUI 元素用（`bg` / `text_color` / `border_color` / `box_shadow` 吃 `Hsla`）。
///
/// 转换用 gpui 自己的 `rgb_to_hsla`，不手写 RGB→HSL 公式。
/// token 仍然写 RGB（设计稿给的就是 RGB），只在这一个出口换成 Hsla。
pub fn hsla(color: Rgba) -> Hsla {
    gpui::rgb_to_hsla(color)
}

/// 分量按 0..255 写 → 0..1（与设计稿上的整数一一对应，便于逐条核对）。
const fn rgb8(r: u8, g: u8, b: u8) -> Rgba {
    c(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
}

/// 八位十六进制 `0xRRGGBBAA`（与 CSS 里写的值直接对应）。
const fn argb(hex: u32) -> Rgba {
    let a = (hex & 0xFF) as f32 / 255.0;
    let r = ((hex >> 24) & 0xFF) as f32 / 255.0;
    let g = ((hex >> 16) & 0xFF) as f32 / 255.0;
    let b = ((hex >> 8) & 0xFF) as f32 / 255.0;
    c(r, g, b, a)
}

/// 深色主题（**权威主题**：设计稿主态与导出的 PNG 都是深色）。
pub mod dark {
    use super::*;

    pub const DESK: Rgba = rgb8(0x0a, 0x0b, 0x10);
    /// 玻璃窗底：`rgba(22, 24, 32, 0.55)`，叠 OS 背板模糊。
    pub const WINDOW_BG: Rgba = argb(0x161820_8C);
    /// 1px 窗描边：白 8%。
    pub const WINDOW_BORDER: Rgba = argb(0xFFFFFF_14);
    pub const WINDOW_SHADOW: Rgba = argb(0x000000_80);

    /// 四种辉光色：**只编码设备类型**，全窗不出现第五种彩色。
    pub const NODE_SELF: Rgba = rgb8(0x7c, 0x8c, 0xff);
    pub const NODE_PHONE: Rgba = rgb8(0x00, 0xd2, 0xd3);
    pub const NODE_TABLET: Rgba = rgb8(0xa2, 0x9b, 0xfe);
    pub const NODE_DESKTOP: Rgba = rgb8(0xfd, 0x79, 0xa8);
    /// 未知类型用中性冷灰：不占用彩色通道。
    pub const NODE_UNKNOWN: Rgba = rgb8(0x8b, 0x8e, 0x9c);

    pub const ICON_SELF: Rgba = rgb8(0xa0, 0xaa, 0xff);
    pub const DISC: Rgba = argb(0x1C1E28_B3);
    pub const DISC_SELF: Rgba = argb(0x1E202C_99);
    /// 新拟物浮雕：光从左上来 → 上左白高光 + 下右黑阴影。
    pub const RELIEF_HI: Rgba = argb(0xFFFFFF_0F);
    pub const RELIEF_LO: Rgba = argb(0x000000_4D);
    pub const HALO: Rgba = argb(0x7C8CFF_33);
    pub const SELF_GLOW: Rgba = argb(0x7C8CFF_59);
    pub const NODE_GLOW: Rgba = argb(0x7C8CFF_33);

    pub const TEXT_PRIMARY: Rgba = argb(0xFFFFFF_E6);
    pub const TEXT_SECONDARY: Rgba = argb(0xFFFFFF_80);
    pub const TEXT_MUTED: Rgba = argb(0xFFFFFF_59);
    pub const TEXT_HINT: Rgba = argb(0xFFFFFF_66);

    pub const DROP_BORDER: Rgba = argb(0xFFFFFF_26);
    pub const DROP_HOVER_BG: Rgba = argb(0x7C8CFF_1A);
    pub const CARD_BG: Rgba = argb(0x14161E_B3);
    pub const CARD_SHADOW: Rgba = argb(0x000000_66);
    pub const BAR_BG: Rgba = argb(0xFFFFFF_14);
    /// 唯一允许的线性渐变。
    pub const PROGRESS_FROM: Rgba = rgb8(0x00, 0xd2, 0xd3);
    pub const PROGRESS_TO: Rgba = rgb8(0x7c, 0x8c, 0xff);
    pub const PCT: Rgba = rgb8(0x00, 0xd2, 0xd3);
    pub const XFER_NAME: Rgba = argb(0xFFFFFF_D9);
    pub const HAIRLINE: Rgba = argb(0xFFFFFF_14);
    pub const HOVER_BG: Rgba = argb(0xFFFFFF_0F);
    pub const ICONBUTTON_HOVER: Rgba = argb(0xFFFFFF_14);
}

/// 浅色主题（覆盖层：窗口底、浮雕方向、图标墨色不同；辉光色不变）。
pub mod light {
    use super::*;

    pub const DESK: Rgba = rgb8(0xe8, 0xe9, 0xed);
    pub const WINDOW_BG: Rgba = argb(0xFFFFFF_99);
    pub const WINDOW_BORDER: Rgba = argb(0x000000_0F);
    pub const WINDOW_SHADOW: Rgba = argb(0x181A22_3D);

    /// 规格外修正：`#a0aaff` 画在白盘上只有 2.13:1，压暗到同色相才读得出来。
    pub const ICON_SELF: Rgba = rgb8(0x5f, 0x6b, 0xe8);
    pub const TONE_INK_PHONE: Rgba = rgb8(0x00, 0x8a, 0x8c);
    pub const TONE_INK_TABLET: Rgba = rgb8(0x5f, 0x52, 0xc8);
    pub const TONE_INK_DESKTOP: Rgba = rgb8(0xc0, 0x34, 0x6b);

    pub const DISC: Rgba = argb(0xFFFFFF_D1);
    pub const DISC_SELF: Rgba = argb(0xFFFFFF_B8);
    /// 浅色浮雕方向反转：亮底上「光从下来」才符合直觉。
    pub const RELIEF_HI: Rgba = argb(0xFFFFFF_B3);
    pub const RELIEF_LO: Rgba = argb(0xD1D5DB_B3);
    pub const HALO: Rgba = argb(0x7C8CFF_47);
    pub const SELF_GLOW: Rgba = argb(0x7C8CFF_47);
    pub const NODE_GLOW: Rgba = argb(0x7C8CFF_38);

    pub const TEXT_PRIMARY: Rgba = rgb8(0x1a, 0x1c, 0x22);
    pub const TEXT_SECONDARY: Rgba = argb(0x1A1C22_AD);
    pub const TEXT_MUTED: Rgba = argb(0x1A1C22_9E);
    pub const TEXT_HINT: Rgba = argb(0x1A1C22_A3);

    pub const DROP_BORDER: Rgba = argb(0x1A1C22_24);
    pub const DROP_HOVER_BG: Rgba = argb(0x7C8CFF_1F);
    pub const CARD_BG: Rgba = argb(0xFFFFFF_C7);
    pub const CARD_SHADOW: Rgba = argb(0x181A22_29);
    pub const BAR_BG: Rgba = argb(0x1A1C22_14);
    pub const PROGRESS_FROM: Rgba = rgb8(0x00, 0xb9, 0xbd);
    pub const PROGRESS_TO: Rgba = rgb8(0x6c, 0x78, 0xf5);
    pub const PCT: Rgba = rgb8(0x00, 0xa8, 0xad);
    pub const XFER_NAME: Rgba = argb(0x1A1C22_D9);
    pub const HAIRLINE: Rgba = argb(0x1A1C22_14);
    pub const HOVER_BG: Rgba = argb(0x1A1C22_0D);
    pub const ICONBUTTON_HOVER: Rgba = argb(0x1A1C22_12);
}

/// 一套完整的主题色（深/浅两套，字段一一对应）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub desk: Rgba,
    pub window_bg: Rgba,
    pub window_border: Rgba,
    pub window_shadow: Rgba,
    pub icon_self: Rgba,
    pub disc: Rgba,
    pub disc_self: Rgba,
    pub relief_hi: Rgba,
    pub relief_lo: Rgba,
    pub halo: Rgba,
    pub self_glow: Rgba,
    pub node_glow: Rgba,
    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub text_muted: Rgba,
    pub text_hint: Rgba,
    pub drop_border: Rgba,
    pub drop_hover_bg: Rgba,
    pub card_bg: Rgba,
    pub card_shadow: Rgba,
    pub bar_bg: Rgba,
    pub progress_from: Rgba,
    pub progress_to: Rgba,
    pub pct: Rgba,
    pub xfer_name: Rgba,
    pub hairline: Rgba,
    pub hover_bg: Rgba,
    pub iconbutton_hover: Rgba,
    /// 是否浅色：浮雕方向、图标墨色要据此切换。
    pub is_light: bool,
}

impl Theme {
    /// 深色是**权威主题**：设计稿主态与导出的 PNG 都是深色。
    pub const DARK: Theme = Theme {
        desk: dark::DESK,
        window_bg: dark::WINDOW_BG,
        window_border: dark::WINDOW_BORDER,
        window_shadow: dark::WINDOW_SHADOW,
        icon_self: dark::ICON_SELF,
        disc: dark::DISC,
        disc_self: dark::DISC_SELF,
        relief_hi: dark::RELIEF_HI,
        relief_lo: dark::RELIEF_LO,
        halo: dark::HALO,
        self_glow: dark::SELF_GLOW,
        node_glow: dark::NODE_GLOW,
        text_primary: dark::TEXT_PRIMARY,
        text_secondary: dark::TEXT_SECONDARY,
        text_muted: dark::TEXT_MUTED,
        text_hint: dark::TEXT_HINT,
        drop_border: dark::DROP_BORDER,
        drop_hover_bg: dark::DROP_HOVER_BG,
        card_bg: dark::CARD_BG,
        card_shadow: dark::CARD_SHADOW,
        bar_bg: dark::BAR_BG,
        progress_from: dark::PROGRESS_FROM,
        progress_to: dark::PROGRESS_TO,
        pct: dark::PCT,
        xfer_name: dark::XFER_NAME,
        hairline: dark::HAIRLINE,
        hover_bg: dark::HOVER_BG,
        iconbutton_hover: dark::ICONBUTTON_HOVER,
        is_light: false,
    };

    pub const LIGHT: Theme = Theme {
        desk: light::DESK,
        window_bg: light::WINDOW_BG,
        window_border: light::WINDOW_BORDER,
        window_shadow: light::WINDOW_SHADOW,
        icon_self: light::ICON_SELF,
        disc: light::DISC,
        disc_self: light::DISC_SELF,
        relief_hi: light::RELIEF_HI,
        relief_lo: light::RELIEF_LO,
        halo: light::HALO,
        self_glow: light::SELF_GLOW,
        node_glow: light::NODE_GLOW,
        text_primary: light::TEXT_PRIMARY,
        text_secondary: light::TEXT_SECONDARY,
        text_muted: light::TEXT_MUTED,
        text_hint: light::TEXT_HINT,
        drop_border: light::DROP_BORDER,
        drop_hover_bg: light::DROP_HOVER_BG,
        card_bg: light::CARD_BG,
        card_shadow: light::CARD_SHADOW,
        bar_bg: light::BAR_BG,
        progress_from: light::PROGRESS_FROM,
        progress_to: light::PROGRESS_TO,
        pct: light::PCT,
        xfer_name: light::XFER_NAME,
        hairline: light::HAIRLINE,
        hover_bg: light::HOVER_BG,
        iconbutton_hover: light::ICONBUTTON_HOVER,
        is_light: true,
    };

    /// 换主题：`Theme` 里唯一的「行为」，避免界面各处自己判 `is_light`。
    pub fn pick(light: bool) -> Theme {
        if light {
            Theme::LIGHT
        } else {
            Theme::DARK
        }
    }

    /// 图标墨色：浅色下辉光色画在白盘上对比度不够，必须换成同色相压暗版。
    pub fn icon_ink(&self, tone: Rgba) -> Rgba {
        if !self.is_light {
            return tone;
        }
        if tone == dark::NODE_PHONE {
            light::TONE_INK_PHONE
        } else if tone == dark::NODE_TABLET {
            light::TONE_INK_TABLET
        } else if tone == dark::NODE_DESKTOP {
            light::TONE_INK_DESKTOP
        } else {
            light::ICON_SELF
        }
    }

    /// 图标按钮 hover 的（背景, 墨色）。
    ///
    /// 墨色必须由调用方放到 **svg 自己**身上（`svg()` 不继承父级 `text_color`），
    /// 所以这里把两个值一起返回，避免调用处只改了背景、图标却不变色。
    pub fn icon_hover(&self, hover: IconHover) -> (Rgba, Rgba) {
        match hover {
            IconHover::Neutral => (self.iconbutton_hover, self.text_primary),
            IconHover::Close => (CLOSE_HOVER_BG, CLOSE_HOVER_INK),
        }
    }

    /// 浮雕方向：浅色反转（亮底上「光从下来」才符合直觉）。
    pub fn relief(&self) -> (Rgba, Rgba) {
        (self.relief_hi, self.relief_lo)
    }
}

/// 设备类型 → 辉光色（**唯一映射表**，界面与测试都引它）。
pub fn device_tone(kind: glowlink_core::DeviceKind) -> Rgba {
    use glowlink_core::DeviceKind;
    match kind {
        DeviceKind::Phone => dark::NODE_PHONE,
        DeviceKind::Tablet => dark::NODE_TABLET,
        DeviceKind::Desktop => dark::NODE_DESKTOP,
        // 笔记本与本机共用品牌蓝：本机永远居中那枚蓝盘
        DeviceKind::Laptop => dark::NODE_SELF,
        DeviceKind::Unknown => dark::NODE_UNKNOWN,
    }
}

/// 本机节点的色（品牌主色）。
pub const NODE_SELF: Rgba = dark::NODE_SELF;

// ------------------------------------------------------- 窗口控制的 hover 色

/// 图标按钮 hover 的两种观感：功能图标是中性的白雾，**关闭按钮是红底白叉**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconHover {
    /// 功能图标（隐私浮层 / 收进托盘）：白 8% 背景 + 提亮墨色。
    Neutral,
    /// 关闭：macOS 红绿灯的关闭键本色。
    Close,
}

/// 关闭按钮 hover 的背景：`#ff5f57`（macOS 关闭键的红）。
///
/// **全窗唯一一处红**，语义只有一条「这个按钮动的是窗口，不是设备功能」。
/// 深浅两套主题共用：红在玻璃上两边都读得出来，分两套只会多一份要维护的值。
pub const CLOSE_HOVER_BG: Rgba = rgb8(0xff, 0x5f, 0x57);
/// 红底上的墨色：白。
pub const CLOSE_HOVER_INK: Rgba = rgb8(0xff, 0xff, 0xff);

/// 产品名。**只此一处**：标题栏文字与窗口标题都用它，不各写一遍。
pub const BRAND_NAME: &str = "萤光";

// ---------------------------------------------------------------- 尺寸 token
//
// 与 `DESIGN.md` 的 `layout` 段一一对应。**坐标只有一个基准：全窗**。

/// 窗体。
pub const WINDOW_W: f32 = 320.0;
pub const WINDOW_H: f32 = 500.0;
/// 塌缩态高度（只剩标题栏条）。
pub const WINDOW_COLLAPSED_H: f32 = 44.0;
pub const WINDOW_RADIUS: f32 = 20.0;
pub const TITLEBAR_H: f32 = 44.0;
pub const GRAPH_H: f32 = 396.0;

/// 节点直径。
pub const NODE_SELF_SIZE: f32 = 84.0;
pub const NODE_DEVICE_SIZE: f32 = 52.0;
/// 图标盒尺寸。**按外轮廓宽度补偿**（设计稿 `node-*` 的 `iconSize`）：
/// 手机机身最窄所以放大到 21，显示器横向宽所以收到 19，视觉重量才对得上。
/// **不要合并成一个数**。
pub const ICON_SELF: f32 = 22.0;
pub const ICON_PHONE: f32 = 21.0;
pub const ICON_TABLET: f32 = 18.0;
pub const ICON_DESKTOP: f32 = 19.0;
/// 笔记本：墨迹比显示器还宽（外轮廓 92% vs 72%），所以取最小一档。
pub const ICON_LAPTOP: f32 = 18.0;
/// 窗口默认落位时距屏幕可见区（不含 Dock/任务栏）的边距。
pub const WINDOW_MARGIN: f32 = 24.0;
/// 拖动到中心时的命中区 = 中心盘本身。
pub const CENTER_HIT: f32 = 84.0;

/// 本机中心盘的圆心（**全窗坐标**）。
///
/// 设备节点**没有**固定坐标：它们在以本机为心的环带里按设备 ID 散布
/// （见 `model/layout::place_devices`）——设计稿那三个槽位已废弃。
/// 本机盘圆心。**图区坐标**（= 设计稿 `.node--self` 的 `top:196`，直接用，不再加标题栏）。
///
/// 全项目只有这一套坐标系：节点图内。指针事件是唯一的例外，
/// 在 `view::app::graph_point` 那一处从窗口坐标换算进来。
pub const SELF_CENTER: (f32, f32) = (160.0, 196.0);
/// 空态提示的纵坐标（图区坐标）。
pub const EMPTY_NOTE_Y: f32 = 268.0;

/// 拖拽区与卡片。
pub const DROPZONE_H: f32 = 44.0;
pub const DROPZONE_INSET: f32 = 16.0;
/// 拖拽区虚线边框宽度（设计稿 1.5px）。
pub const DROPZONE_BORDER: f32 = 2.0;
pub const CARD_W: f32 = 200.0;
pub const POPOVER_W: f32 = 236.0;

/// 字号（严格按设计稿，最小 9px 只用于纯展示元信息）。
pub const TEXT_BRAND: f32 = 14.0;
pub const TEXT_DEVICE_LABEL: f32 = 10.0;
pub const TEXT_BODY: f32 = 11.0;
pub const TEXT_META: f32 = 9.0;
/// 配对码的字号：全窗最大的字（**唯一需要用户隔屏读数**的地方）。
pub const TEXT_CODE: f32 = 22.0;

/// 标题栏按钮簇内的间隔。
pub const GAP_ICON_BUTTON: f32 = 2.0;

/// 圆角。
pub const RADIUS_CARD: f32 = 12.0;
pub const RADIUS_DROP: f32 = 12.0;
pub const RADIUS_BUTTON: f32 = 8.0;
pub const RADIUS_ICONBUTTON: f32 = 6.0;
pub const RADIUS_POPOVER: f32 = 14.0;
