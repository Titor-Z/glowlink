---
version: alpha
name: 萤光 GlowLink
description: >
  A local-first, decentralized near-field file transfer tool rendered as a 320×500 glass
  instrument window. Devices are luminous nodes floating in the dark around a single bright
  self-node; trust is established by dragging a remote node into the center. Four glow colors
  are the only saturated color in the window and they encode device type — nothing else.
  Neumorphic inset relief gives the glass discs physicality; light moves, nothing blinks.
  No card walls, no gradients-for-decoration, no emoji, no fifth color.
colors:
  # —— 权威值：深色主题（设计稿主态，CSS 为唯一来源）——
  desk: "#0a0b10"                     # 窗外桌面底色
  window: "rgba(22, 24, 32, 0.55)"    # 玻璃窗底（叠 OS 背板模糊 40px + 饱和 140%）
  window-border: "rgba(255, 255, 255, 0.08)"   # 1px 窗描边（内描边，不占布局）
  window-shadow: "0 20px 60px rgba(0, 0, 0, 0.5)"
  node-self: "#7c8cff"                # 本机辉光 / 品牌主色
  node-phone: "#00d2d3"
  node-tablet: "#a29bfe"
  node-desktop: "#fd79a8"
  node-unknown: "#8b8e9c"             # 中性冷灰：未知类型不占用彩色通道
  icon-self: "#a0aaff"                # 本机中心图标
  disc: "rgba(28, 30, 40, 0.7)"       # 设备盘
  disc-self: "rgba(30, 32, 44, 0.6)"  # 本机盘
  relief-hi: "rgba(255, 255, 255, 0.06)"   # 浮雕高光（上左）
  relief-lo: "rgba(0, 0, 0, 0.3)"          # 浮雕阴影（下右）
  halo: "rgba(124, 140, 255, 0.2)"    # 本机呼吸光圈
  self-glow: "rgba(124, 140, 255, 0.35)"
  node-glow: "rgba(124, 140, 255, 0.2)"
  text-primary: "rgba(255, 255, 255, 0.9)"
  text-secondary: "rgba(255, 255, 255, 0.5)"
  text-muted: "rgba(255, 255, 255, 0.35)"
  text-hint: "rgba(255, 255, 255, 0.4)"
  drop-border: "rgba(255, 255, 255, 0.15)"
  drop-hover-bg: "rgba(124, 140, 255, 0.1)"
  card-bg: "rgba(20, 22, 30, 0.7)"    # 传输卡 / 浮层底（叠应用内 backdrop blur 20px）
  card-shadow: "0 8px 28px rgba(0, 0, 0, 0.4)"
  bar-bg: "rgba(255, 255, 255, 0.08)"
  progress-from: "#00d2d3"            # 唯一允许的线性渐变
  progress-to: "#7c8cff"
  pct: "#00d2d3"
  xfer-name: "rgba(255, 255, 255, 0.85)"
  hairline: "rgba(255, 255, 255, 0.08)"
  hover-bg: "rgba(255, 255, 255, 0.06)"
  iconbutton-hover: "rgba(255, 255, 255, 0.08)"
  # —— 浅色主题覆盖（规格 §6 + 实测修正）——
  desk-light: "#e8e9ed"
  window-light: "rgba(255, 255, 255, 0.6)"
  window-border-light: "rgba(0, 0, 0, 0.06)"
  window-shadow-light: "0 20px 60px rgba(24, 26, 34, 0.24)"
  icon-self-light: "#5f6be8"          # 规格外修正：原 #a0aaff 在白盘仅 2.13:1
  tone-ink-phone-light: "#008a8c"     # 浅色下图标墨色（辉光色直接画上盘只有 1.9–2.9:1）
  tone-ink-tablet-light: "#5f52c8"
  tone-ink-desktop-light: "#c0346b"
  disc-light: "rgba(255, 255, 255, 0.82)"
  disc-self-light: "rgba(255, 255, 255, 0.72)"
  relief-hi-light: "rgba(255, 255, 255, 0.7)"    # 浅色浮雕方向反转
  relief-lo-light: "rgba(209, 213, 219, 0.7)"
  halo-light: "rgba(124, 140, 255, 0.28)"
  self-glow-light: "rgba(124, 140, 255, 0.28)"
  node-glow-light: "rgba(124, 140, 255, 0.22)"
  text-primary-light: "#1a1c22"
  text-secondary-light: "rgba(26, 28, 34, 0.68)"
  text-muted-light: "rgba(26, 28, 34, 0.62)"
  text-hint-light: "rgba(26, 28, 34, 0.64)"
  drop-border-light: "rgba(26, 28, 34, 0.14)"
  drop-hover-bg-light: "rgba(124, 140, 255, 0.12)"
  card-bg-light: "rgba(255, 255, 255, 0.78)"
  card-shadow-light: "0 8px 28px rgba(24, 26, 34, 0.16)"
  bar-bg-light: "rgba(26, 28, 34, 0.08)"
  progress-from-light: "#00b9bd"
  progress-to-light: "#6c78f5"
  pct-light: "#00a8ad"
  xfer-name-light: "rgba(26, 28, 34, 0.85)"
  hairline-light: "rgba(26, 28, 34, 0.08)"
  hover-bg-light: "rgba(26, 28, 34, 0.05)"
  iconbutton-hover-light: "rgba(26, 28, 34, 0.07)"

typography:
  # 不引入第三方字体：SF Pro（-apple-system）+ PingFang SC 回退。
  # 数值一律 tabular-nums（Rust 侧是字节流，位数会跳，表格数字防抖动）。
  brand:
    fontFamily: "SF Pro Text"
    fontSize: 14px
    fontWeight: 600
    letterSpacing: -0.01em
  device-label:
    fontFamily: "SF Pro Text"
    fontSize: 10px
    fontWeight: 400
    lineHeight: 1
  file-name:
    fontFamily: "SF Pro Text"
    fontSize: 11px
    fontWeight: 400
    lineHeight: 1.3
  percent:
    fontFamily: "SF Pro Text"
    fontSize: 11px
    fontWeight: 600
    fontFeature: "tnum"
  body:
    fontFamily: "SF Pro Text"
    fontSize: 11px
    fontWeight: 400
  meta:
    fontFamily: "SF Pro Text"
    fontSize: 9px
    fontWeight: 400
    lineHeight: 1.4
    fontFeature: "tnum"
  popover-title:
    fontFamily: "SF Pro Text"
    fontSize: 11px
    fontWeight: 600
  popover-row:
    fontFamily: "SF Pro Text"
    fontSize: 10.5px
    fontWeight: 400
    lineHeight: 1.7
  popover-foot:
    fontFamily: "SF Pro Text"
    fontSize: 9.5px
    fontWeight: 400
    lineHeight: 1.5
  # —— 配对：设计稿未定义，本案新增（沿用等宽数字 + 大字码，便于隔屏读）——
  pair-code:
    fontFamily: "SF Mono, Menlo, monospace"
    fontSize: 22px
    fontWeight: 600
    letterSpacing: 0.08em
    fontFeature: "tnum"

rounded:
  window: 20px
  popover: 14px
  card: 12px
  drop: 12px
  button: 8px
  iconbutton: 6px
  full: 9999px

spacing:
  xxs: 2px
  xs: 6px
  sm: 7px
  md: 8px
  lg: 10px
  xl: 12px
  xxl: 16px

layout:
  window-width: 320px
  window-height: 500px
  window-collapsed: 44px
  titlebar: 44px
  graph: 396px
  dropzone: 44px
  dropzone-inset: 16px
  card-width: 200px
  popover-width: 236px
  node-self: 84px
  node-device: 52px
  center-hit: 84px
  # 节点圆心 = 全窗坐标（不是 graph 内坐标）。CSS 里写 top:196px 是 graph 内坐标，
  # 加 44px 标题栏才是全窗坐标；两套基准是要消灭的东西，实现只认这一行。
  node-centers: "self 160,196（图区坐标）· 设备为散点，位置由设备 ID 派生（见 Layout）"
  empty-note: "160,312"

components:
  window:
    width: "{layout.window-width}"
    height: "{layout.window-height}"
    rounded: "{rounded.window}"
    backgroundColor: "{colors.window}"
    borderColor: "{colors.window-border}"
    boxShadow: "{colors.window-shadow}"
    backdropBlur: 40px
    backdropSaturate: 140%
  titlebar:
    height: "{layout.titlebar}"
    paddingX: "{spacing.xxl}"
    gap: "{spacing.md}"
    typography: "{typography.brand}"
  desktop-canvas:
    backgroundColor: "{colors.desk}"
  graph:
    height: "{layout.graph}"
  brand-dot:
    size: 8px
    backgroundColor: "{colors.node-self}"
    glow: "0 0 8px {colors.node-self}"
  iconbutton:
    width: 22px
    height: 22px
    rounded: "{rounded.iconbutton}"
    textColor: "{colors.text-secondary}"
    iconSize: 14px
  iconbutton-hover:
    backgroundColor: "{colors.iconbutton-hover}"
    textColor: "{colors.text-primary}"
  node-self:
    size: "{layout.node-self}"
    ringWidth: 2px
    ringColor: "rgba(124, 140, 255, 0.5)"
    glow: "0 0 24px {colors.self-glow}"
    discInset: 6px
    discBackground: "{colors.disc-self}"
    iconSize: 22px
    iconColor: "{colors.icon-self}"
    haloInset: -8px
    haloColor: "{colors.halo}"
    labelColor: "{colors.text-muted}"
  node-device:
    size: "{layout.node-device}"
    ringWidth: 1.5px
    ringColor: "color-mix(in oklab, tone 50%, transparent)"
    glow: "0 0 16px {colors.node-glow}"
    discInset: 4px
    discBackground: "{colors.disc}"
    discShadow: "inset 2px 2px 6px {colors.relief-hi}, inset -2px -2px 6px {colors.relief-lo}"
    iconSize: 18px
    labelOffset: "{spacing.xs}"
    labelColor: "{colors.text-secondary}"
  node-device-hover:
    ringColor: "color-mix(in oklab, tone 78%, transparent)"
    glow: "0 0 20px color-mix(in oklab, tone 34%, transparent)"
  node-device-selected:
    ringColor: "tone"
    glow: "0 0 22px color-mix(in oklab, tone 46%, transparent)"
    discBackground: "color-mix(in oklab, tone 16%, {colors.disc})"
  node-device-target:
    ringWidth: 1.5px
    glow: "0 0 24px color-mix(in oklab, tone 52%, transparent)"
  node-unknown-kind:
    ringColor: "{colors.node-unknown}"
  node-kind-self:
    ringColor: "{colors.node-self}"
  node-kind-phone:
    ringColor: "{colors.node-phone}"
  node-kind-tablet:
    ringColor: "{colors.node-tablet}"
  node-kind-desktop:
    ringColor: "{colors.node-desktop}"
  drop-hit-target:
    size: "{layout.center-hit}"
    positions: "{layout.node-centers}"
  dropzone:
    height: "{layout.dropzone}"
    marginX: "{layout.dropzone-inset}"
    marginBottom: "{spacing.xxl}"
    rounded: "{rounded.drop}"
    borderColor: "{colors.drop-border}"
    borderWidth: 1.5px
    borderStyle: dashed
    textColor: "{colors.text-muted}"
    typography: "{typography.body}"
  dropzone-over:
    borderColor: "{colors.node-self}"
    backgroundColor: "{colors.drop-hover-bg}"
    textColor: "{colors.text-secondary}"
  dropzone-denied:
    borderColor: "{colors.node-desktop}"
    textColor: "{colors.text-secondary}"
    motion: "shake 0.36s ease"
  xfer-card:
    width: "{layout.card-width}"
    bottom: 70px
    right: "{spacing.xl}"
    padding: "{spacing.xl}"
    rounded: "{rounded.card}"
    backgroundColor: "{colors.card-bg}"
    borderColor: "{colors.window-border}"
    boxShadow: "{colors.card-shadow}"
    backdropBlur: 20px
    textColor: "{colors.xfer-name}"
    typography: "{typography.file-name}"
    motion: "cardIn 0.24s cubic-bezier(0.22, 1, 0.36, 1)"
  xfer-meta:
    textColor: "{colors.text-hint}"
    typography: "{typography.meta}"
  progress-bar:
    height: 4px
    marginY: "{spacing.md} 0 {spacing.sm} 0"
    rounded: "{rounded.full}"
    backgroundColor: "{colors.bar-bg}"
    fill: "linear-gradient(90deg, {colors.progress-from}, {colors.progress-to})"
    percentColor: "{colors.pct}"
    typography: "{typography.percent}"
  minibtn:
    height: 24px
    rounded: "{rounded.button}"
    borderColor: "{colors.hairline}"
    textColor: "{colors.text-secondary}"
    typography: "{typography.device-label}"
  minibtn-hover:
    backgroundColor: "{colors.hover-bg}"
    textColor: "{colors.text-primary}"
  minibtn-accent:
    textColor: "{colors.pct}"
    borderColor: "color-mix(in oklab, {colors.pct} 40%, transparent)"
  privacy-popover:
    width: "{layout.popover-width}"
    top: 48px
    right: "{spacing.lg}"
    padding: "{spacing.xl}"
    rounded: "{rounded.popover}"
    backgroundColor: "{colors.card-bg}"
    boxShadow: "{colors.card-shadow}"
  privacy-title:
    typography: "{typography.popover-title}"
    textColor: "{colors.text-primary}"
  privacy-row:
    typography: "{typography.popover-row}"
    textColor: "{colors.text-secondary}"
  privacy-foot:
    typography: "{typography.popover-foot}"
    textColor: "{colors.text-muted}"
    borderTop: "1px solid {colors.hairline}"
  traybar:
    height: "{layout.window-collapsed}"
    backgroundColor: "{colors.window}"
    textColor: "{colors.text-secondary}"
    typography: "{typography.body}"
  wire-base:
    strokeWidth: 1px
    strokeColor: "tone"
    opacity: 0.22
  wire-flow:
    strokeWidth: 1.5px
    strokeDasharray: "5 9"
    motion: "flow 1s linear infinite"
  spinner:
    size: 12px
    trackColor: "rgba(124, 140, 255, 0.25)"
    headColor: "{colors.node-self}"
    motion: "spin 0.9s linear infinite"
  empty-note:
    position: "{layout.empty-note}"
    textColor: "{colors.text-muted}"
    typography: "{typography.body}"
  # —— 配对（设计稿未定义，本案新增；不引入新颜色，失败态只用文案 + 抖动）——
  pair-card:
    width: "{layout.card-width}"
    padding: "{spacing.xl}"
    rounded: "{rounded.card}"
    backgroundColor: "{colors.card-bg}"
    borderColor: "{colors.window-border}"
    boxShadow: "{colors.card-shadow}"
    backdropBlur: 20px
  pair-code:
    typography: "{typography.pair-code}"
    textColor: "{colors.text-primary}"
  pair-code-input:
    cellSize: 26px
    cellGap: "{spacing.xxs}"
    rounded: "{rounded.button}"
    borderColor: "{colors.drop-border}"
    textColor: "{colors.text-primary}"
    typography: "{typography.percent}"
  pair-code-input-error:
    borderColor: "{colors.node-desktop}"
    textColor: "{colors.text-secondary}"
    motion: "shake 0.36s ease"
---

# 萤光 GlowLink — 视觉系统

## Overview

**萤萤之光，可照旷野。**

GlowLink 是一个**闭源、本地优先、去中心化**的跨平台近场文件传输工具。它的界面不是"一个窗口里的功能"，而是**一张会呼吸的节点图**：本机是中心那枚 84px 的亮盘，附近设备是围绕它漂浮的 52px 小盘，一次传输就是一条从本机流向目标的光线。

情绪目标是**安静、可信、不张扬**。它读起来应该像一台仪器而不是一个应用：没有卡片海洋、没有渐变装饰、没有营销语言、没有 emoji。窗口是**玻璃**的——背后就是用户的桌面——所以它从不假装自己是个网页。

受众是每天在两三台设备之间倒文件的人（开发、设计、内容工作者）。他们要的是"拖一下就过去"，不是"注册一个账号"。

三条统领全文的判据：

1. **辉光即语义**。四种辉光色（本机蓝、手机青、平板紫、台式机粉）只用于**设备类型识别**，未知类型用中性冷灰，不占用彩色通道。
   - **唯一例外：鼠标经过“关闭”按钮时的红底（`#ff5f57`，macOS 关闭键本色）。**
     它只在 hover 态出现、只占一个 22px 方块、只表达一件事：“这个按钮动的是窗口，不是设备功能”。
     常驻色面里依旧没有第五种彩色。
2. **一次只强调一件事**。空态只有自机 + spinner；发现态只有节点；传输态才出现连线 + 传输卡；配对态才出现码卡片。四个状态**互斥**，不叠加。
3. **光会动，但不闪**。所有运动都是缓慢、连续、错相的（漂浮 7s、呼吸 3s）；不存在闪烁、弹跳、抢注意力。

窗口姿态是**锁死**的：320×500、圆角 20、1px 内描边、玻璃模糊 40px。任何内容不得撑破这个盒子——它是工具窗口，不是营销页。

## Colors

深色是**权威**主题（设计稿主态，CSS 是唯一数值来源）；浅色是覆盖层，所以深色用裸 token 名、浅色统一加 `-light` 后缀。两套只有**窗口底、文字层级、浮雕方向、进度条渐变**不同；四种辉光色**不随主题变化**。

- **Deep desk（`desk` #0a0b10）：** 窗外桌面底色，近黑、微冷。它保证玻璃窗永远有东西可透。
- **Glass window（`window`）：** `#161820` @ 55%，叠 OS 背板模糊。"玻璃"不是装饰——它承担产品的核心承诺：没有云端、没有中转，你看到的就是你机器上的东西。
- **Four glows（`node-self` / `node-phone` / `node-tablet` / `node-desktop`）：** 唯一允许饱和的四色。它们**只编码类型**，不编码状态；状态靠环的粗细、辉光强度、呼吸表达。
- **Neutral unknown（`node-unknown` #8b8e9c）：** 未知类型不占用彩色通道——给"我还没弄清你是谁"一个诚实的灰。
- **Ink ladder（`text-primary` / `-secondary` / `-muted` / `-hint`）：** 白色 @ 90 / 50 / 35 / 40%。层级靠透明度而不是灰度，才能压在玻璃上而不会在模糊背景上"糊成一块"。
- **Progress gradient（`progress-from` → `progress-to`）：** `#00d2d3 → #7c8cff` 是**唯一允许的线性渐变**，且只出现在进度条填充上。

### 对比度实测（按真实复合底色计算，非按 token 纸面值）

窗口底：深色 `rgba(22,24,32,.55)` 叠 `#0a0b10` → **#111219**；浅色 `rgba(255,255,255,.6)` 叠 `#e8e9ed` → **#f6f6f8**。
传输卡底：深色 → **#13151c**；浅色 → **#fdfdfd**。

| 前景 | 底色 | 实测 | AA |
|---|---|---|---|
| `text-primary` 深 | #111219 | 15.11:1 | ✅ |
| `text-secondary` 深 | #111219 | 5.29:1 | ✅ |
| `pct` #00d2d3 深 | 传输卡 | 9.69:1 | ✅ |
| 四色图标 深 | 节点盘 | 6.98–9.19:1 | ✅ |
| `text-primary-light` | #f6f6f8 | 15.78:1 | ✅ |
| `text-secondary-light` | #f6f6f8 | 5.66:1 | ✅ |
| `text-muted-light` | #f6f6f8 | 4.65:1 | ✅ |
| `text-hint-light` | #f6f6f8 | 5.00:1 | ✅ |
| 四种浅色图标墨 | 节点盘 | 4.12–5.86:1 | ✅（非文字门槛 3:1）|
| `text-hint` 深（白 40%）| 传输卡 | **3.83:1** | ❌ 见「Do's and Don'ts」 |
| `text-muted` 深（白 35%）| #111219 | **3.21:1** | ❌ 见「Do's and Don'ts」 |
| `pct-light` #00a8ad | 传输卡 | **2.87:1** | ❌ 见「Do's and Don'ts」 |

浅色图标墨色是**规格外修正**：`#a0aaff` 画在白盘上只有 2.13:1，实测不可读；改用同色相压暗版（`icon-self-light` / `tone-ink-*-light`）后升到 4.1–5.9:1。**这个是既成事实，不要再改回去。**

### 浅色覆盖映射（深色为基，浅色只改这几项）

| 深色 token | 浅色 token |
|---|---|
| `{colors.desk}` | `{colors.desk-light}` |
| `{colors.window}` | `{colors.window-light}` |
| `{colors.window-border}` | `{colors.window-border-light}` |
| `{colors.window-shadow}` | `{colors.window-shadow-light}` |
| `{colors.icon-self}` | `{colors.icon-self-light}` |
| 辉光色当图标墨色（四色） | `{colors.tone-ink-phone-light}` · `{colors.tone-ink-tablet-light}` · `{colors.tone-ink-desktop-light}` · `{colors.icon-self-light}` |
| `{colors.disc}` | `{colors.disc-light}` |
| `{colors.disc-self}` | `{colors.disc-self-light}` |
| `{colors.relief-hi}` / `{colors.relief-lo}` | `{colors.relief-hi-light}` / `{colors.relief-lo-light}`（方向反转）|
| `{colors.halo}` · `{colors.self-glow}` · `{colors.node-glow}` | `{colors.halo-light}` · `{colors.self-glow-light}` · `{colors.node-glow-light}` |
| `{colors.text-primary}` · `-secondary` · `-muted` · `-hint` | `{colors.text-primary-light}` · `{colors.text-secondary-light}` · `{colors.text-muted-light}` · `{colors.text-hint-light}` |
| `{colors.drop-border}` · `{colors.drop-hover-bg}` | `{colors.drop-border-light}` · `{colors.drop-hover-bg-light}` |
| `{colors.card-bg}` · `{colors.card-shadow}` | `{colors.card-bg-light}` · `{colors.card-shadow-light}` |
| `{colors.bar-bg}` | `{colors.bar-bg-light}` |
| `{colors.progress-from}` → `{colors.progress-to}` | `{colors.progress-from-light}` → `{colors.progress-to-light}` |
| `{colors.pct}` | `{colors.pct-light}` |
| `{colors.xfer-name}` | `{colors.xfer-name-light}` |
| `{colors.hairline}` · `{colors.hover-bg}` · `{colors.iconbutton-hover}` | `{colors.hairline-light}` · `{colors.hover-bg-light}` · `{colors.iconbutton-hover-light}` |

四种辉光色（`{colors.node-self}` / `{colors.node-phone}` / `{colors.node-tablet}` / `{colors.node-desktop}`）**不随主题变化**；浅色下靠浮雕阴影与墨色图标维持立体感。

## Typography

只用一个家族：**SF Pro**（`-apple-system`）+ 中文回退 **PingFang SC**。不引入第三方字体——节点图是工具窗口，不是营销页；系统字体还能在中文与 emoji 混排时自动挑最合适的字形。

字号严格按设计稿，**最小 9px 是底线**（仅用于卡片的纯展示元信息）：

- **品牌名（14/600，tracking -0.01em）：** 标题栏那一个“萤光”就是身份，不是标题。
  **只用两个字**：不需要“GlowLink”缀在后面——产品名是“萤光”，英文名只在仓库/文档/窗标题的无障碍文本里出现。
- **设备标签（10/400）：** 节点下方的设备名，白色 50%。10px 是**可读下限**，再小就该考虑裁掉信息而不是缩小字。
- **文件名（11/400，行高 1.3）：** 传输卡里的文件名，超长省略号截断——不换行，卡只有 200px。
- **百分比（11/600，`tnum`）：** 传输卡的百分比。**必须等宽数字**：字宽变化会让数字左右抖，看起来像在"跳"。
- **元信息（9/400，`tnum`）：** 速度、剩余时间、卡片脚注。纯展示，不承载操作。
- **正文（11/400）：** 拖拽区提示、托盘条文案。
- **浮层三档（11/600 · 10.5/400 · 9.5/400）：** 隐私承诺的标题 / 条目 / 脚注。

**等宽数字的例外说明**：`tnum` 不是"用等宽字体"，而是**开启等宽数字特性**——正文仍是 SF Pro，只有数字对齐。GPUI 侧对应 `FontFeatures`（`tnum`）。

## Layout

布局是**绝对坐标**，不是流式：这正是一张节点图。窗口 320×500 分三段——标题栏 44 / 节点图 396 / 拖拽区 44 + 16 外边距。卡片与浮层是**浮在图上的层**，不参与分段。

**节点圆心（图区坐标 = 节点图内，从标题栏下沿算起）**：

| 节点 | 圆心 | 直径 |
|---|---|---|
| 本机（中心） | 160, 196 | 84px（连接后 ×2/3，圆心不动） |
| 设备 | **不固定**，见下 | 52px（连接后 ×2/3，拖动中 ×1.08） |
| 空态提示 | 160, 268 | — |

> ⚠️ **坐标基准只有一个：图区**（就是 CSS 里 `.node--self { top: 196px }` 的那一套，直接照抄）。
> 唯一换算在指针进入时（窗口坐标 − 44 标题栏）。**圆心 → 左上角也只许换算一处**
> （`node::placed(child, center, size)` 自己减半）：曾经布局用全窗坐标、渲染容器用图区坐标，
> 设备整体低了 44px、拖动又跳 44px；也曾经把圆心直接当左上角用，中心盘一上来就跑偏。

**设备位置：以本机为中心的散点（不是固定槽位）**

设计稿那三个槽位（手机 66,86 / 平板 254,86 / 台式机 160,364）**已废弃**：真实设备数不受设计稿约束，
按模板摆出来只会有“行星轨道”感。要的是**神经元**——一圈疏密不匀、远近不一的散点：

- 位置由**设备 ID** 派生（稳定散列 + 排斥迭代）；同一台设备永远是同一个位置，
  不随帧、不随设备表顺序变（`model/layout.rs` 的 `place_devices`）。
- 三条硬约束：**距本机盘心 ≥ 92px**（不压住本机）、**两圆心 ≥ 60px**（不互相叠）、
  **圆心留在 30..290 × 56..396 内**（盘与设备名都在图内）。
- 半径与角度都随机：全是同一个半径就成了同心圆轨道，这正是要避免的。
- 漂浮（`drift 7s`）的相位也由设备 ID 派生——错相而非同拍。
- 反过来：本机中心盘**不漂**，它是图面的锚点。

- **吸附判定 = 气泡自己贴到中心盘上**：`距离 ≤ 本机盘半径 + 气泡当前半径 + 6px`
  （≈ 42 + 28 + 6 = 76px，拖动中气泡 ×1.08）。**不看指针**——抓在盘边上时指针与气泡能差 20~30px，
  按指针算会「一边要多走、另一边没到就吸上」。判定、中心盘高亮、松手吸附**共用同一个函数**：
  高亮是承诺，亮了就必须吸上。灯亮 0.08s、灯灭 0.25s。落在别处回弹（带着松手速度）。
- **中心盘是钉死的不动体**：进物理世界，会**把压上来的气泡挤开**，而自己绝不被推动；
  它与设备**同一层**（重叠由物理消除，不靠画序遮挡）。断开后它恢复全尺寸时会推开周围的气泡。
- **连接槽位由中心分配**：第 1 台固定**正上方（12 点）**，其余**顺时针均匀**（`slot_angles(n)`）；
  断开后剩下的一起压实重排（走弹簧滑过去）。槽位间隔从几何算（相邻两个缩小盘半径和 + 缝），
  最多 6 台；超了由物理推开，不叠成一坨。
- 设备节点默认 opacity 0（空态不显示），发现态与传输态才亮起；入场缩放 0.9 → 1。
- 传输卡贴右下（right 12 / bottom 70），宽 200；隐私浮层贴上右（top 48 / right 10），宽 236。
- 塌缩态：窗口高度收到 44px（只剩标题栏条），图与拖拽区淡出。

间距刻度只有七档：2 / 6 / 7 / 8 / 10 / 12 / 16。**没有 5、没有 15**——相邻层级靠 6 与 7 的微差制造呼吸，别自创刻度。

## Elevation & Depth

深度**不靠投影**，靠**新拟物浮雕**与**辉光**。这是本案最容易做错的一段。

1. **浮雕 = inset 双向阴影**。盘的 `inset 2px 2px 6px` 白 6%（上左高光）+
   `inset -2px -2px 6px` 黑 30%（下右阴影）——光从左上来，盘因此"鼓起来"。**浅色主题方向反转**（白在左下、灰在右上），因为在亮底上"光源从下来"才符合直觉。
2. **辉光 = 外环 + 一圈柔光，不用投影**。环是 1.5px（本机 2px）的半透明 tone 色；辉光是 `0 0 16px`（本机 24px）——**零模糊偏移**，只有扩散。GPUI 没有 radial gradient，用**同心圆 alpha 曲线**实现，不要试图用投影冒充。
3. **真正的投影只有两处**：窗口（`0 20px 60px`，让玻璃浮在桌面上）与卡片（`0 8px 28px`，让卡片浮在玻璃上）。两者都是"层"的语义，不是"元件"的语义。

层级顺序（低 → 高）：节点图连线 < 节点 < 拖拽区 < 传输卡 / 配对卡 < 隐私浮层 < 托盘条 < 1px 窗描边。

> 实现注记：两张浮层卡（传输 / 配对）**同一个外壳**（`view/card.rs`）：左右各 16（与拖拽区同边距）、
> 底边在拖拽区之上 12px、12px 圆角、右上角直径 20 的圆形 ✕；
> **拟物玻璃三件套**（设计稿 `.xfer` / `.privacy`，缺一件就不像玻璃）：
> `card-bg` 半透明底（0.7 / 0.78）+ `backdrop_filter` 20px + **1px `--win-border` 描边** + `0 8px 28px` 投影
> （hover 红底 `#ff5f57`）。**必须画在拖拽区之后**——否则会被拖拽区盖住，
> 真机上表现为「卡片跑到拖拽按钮下面」。两卡同槽位，同时出现时配对卡优先（一次只强调一件事）。
> ✕ 的语义随状态变：在传 = 取消传输、结束 = 关卡片、配对中 = 放弃配对（`PAIR_CANCEL`）、
> 接收侧请求 = 拒绝（协议要求「必须有个答案」，不能只是收起）。
>
> 配对卡两段（接收侧）：① 询问 —— 「「X」想与这台设备配对」+ 拒绝 / 允许；② 允许后 —— 6 格输入
> （每格 26×30、6px 间距、22px 等宽数字，**当前格只提亮底色**、不引入第五种颜色）+ 确认。
> 出示侧只有一张：设备名 + 22px 码 + 「请在对方设备上输入此码」。
> 输错不改颜色：文案「校验码不符，还剩 N 次机会」+ 清空重输（3 次用尽后请求消失、卡片自动收起）。
>
> 投放（device-first）实现注记：单击设备 = 选中（再点取消），拖拽区文案随之变「拖到此处 → 甲机」；
> 未选设备时丢文件按设计稿拒收（边框转台式机粉 + 抖动 0.36s + 「先选择一台设备」，1.8s 后收回）；
> 命中区是**整窗**（拖拽区是提示与状态，不是 44px 小按钮）；点击拖拽区 = 系统文件对话框。
> **文案口吻**（用户拍板）：只说用户在这台机器上看得见的事——
> 状态用陈述（「附近没有设备」「已暂停查找设备」），动作说收益（「把设备拖到中间即可发送」），
> **不写「让对方…」「去打开另一台设备的…」这类冲着对端发的指令**（那是把实现说成了命令）。
> 也不用我们的词：不写「服务」（守护进程），也不写控件位置（「右上角的开关」）。
>
> 中央空态与拖拽区**各说各的**：空态说搜到什么（「附近没有设备」/「已暂停查找设备」），
> 拖拽区说这个框现在的能耐（「暂无可发送的设备」/「把设备拖到中间即可发送」/「拖到此处 → 甲机」）。
> 同一个信息不许在两处出现。
>
> 空态（`empty-note`）的实现口径：它是**过渡报告**，不是常驻说明——
> 「正在搜索附近设备…」（转，10s）→「附近还没有设备 · 让对方也打开 GlowLink」（停转，6s）→ **收起**；
> 服务关着/正在启动时说「服务已关闭 · 打开右上角的开关」/「正在启动服务…」（各留几秒）。
> 常驻的那句在拖拽区（「先打开另一台设备的 GlowLink」）。只有「正在搜索」时才转圈。
>
> 拖拽区文案的**判定顺序**（用户拍板）：服务状态 → 有没有设备 → **有没有连接** → 「拖到此处 → 目标」。
> 「已连接」= 拖到中心连上的那台（环上）；**发现到的陌生设备不是发送目标**，
> 这时写「先把设备拖到中间连接」（同一个手势的提示，不是一句拒绝）。
>
> 拖拽区文案（在冻结的设计稿之外补的两条，用户拍板）：**没设备**时写「先打开另一台设备的 GlowLink」
> （不写「拖拽文件到这里」——那是邀请一个必然被拒的动作）；**多台未选**时提前写「先选择一台设备」
> （原来是丢进去才拒）。只有一台时「一台免选」，文案直接写「拖到此处 → 甲机」。
> 拒收态永不显示目标名。
>
> 空态实现注记：设计稿 `.empty-note` 的 spinner 在实现里是**按相位重画的弧**
> （gpui 无 `rotate`，`canvas` + `PathBuilder`）；搜满 10s 后文案改
> 「附近还没有设备」并停转（不再假装在搜，也不吃每帧重绘）。
>
> 接收侧待确认卡（同样外壳）：`「X」想发给你` + `文件名 · 大小` + 22px 校验码 +
> 「请与发送方屏幕上的校验码核对，一致再接受」+ 拒绝 / 接受；✕ = 拒绝；多于一个请求时补「另有 N 个在等」。
>
> 一次投放多个文件时，卡片文件名右侧出现 `2/5`（单个文件不写 `1/1`），整批结束时元信息换成
> 「5 个文件 · 共 12.4 MB」；✕ = 取消整批（在传/排队时）或收起卡片（已结束时）。

## Shapes

**圆角即尺度**：窗口 20px 是唯一的"大圆角"，其余全部收敛到 6–14px。

- 20px —— 窗体（与 macOS 原生窗口圆角一致，靠系统给，不自己画）
- 14px —— 隐私浮层（比卡片更"轻"，圆一点显得可关）
- 12px —— 传输卡 / 配对卡 / 拖拽区（同一"块"的尺度）
- 8px —— 小按钮（minibtn、码输入的格子）
- 6px —— 图标按钮（22px 的方块，圆角超过 8 会像药片）
- 9999px —— 品牌点、进度条

**圆形只给节点**。设备是"盘"，所以是正圆 + 内缩盘；不要给节点加圆角方框。

## Components

### 节点（node）

节点是**四层同心结构**（从外到内）：呼吸光圈（本机与传输目标才有）→ 外环（1.5/2px tone 色 + 零模糊辉光）→ 盘（内缩 4/6px，inset 浮雕）→ 图标（18/22px）。设备名在盘下方 6px、10px、白色 50%。

状态只有四种，且都用**同一条通道**（环的强度）表达，不引入新颜色：

| 状态 | 表现 |
|---|---|
| 默认 | 环 50% tone，辉光 16px |
| hover | 环 78% tone，辉光 20px 34% |
| 选中 | 环满 tone，辉光 22px 46%，盘染 16% tone |
| 传输中（本机 + 目标） | 环满 tone，辉光 24px 52%，光圈呼吸加快到 1.5s |

图标尺寸按**外轮廓宽度补偿**：手机机身最窄，给 21px；平板 18px；显示器横向宽，给 19px；本机 22px。**不要统一成一个数**——统一的像素数不等于统一的视觉重量。

### 拖拽区（dropzone）

44px 高、12px 圆角、1.5px 虚线 15%。hover 只提亮文字；dragover 换本机蓝描边 + 10% 同色底；**未选设备就投放 = 拒绝**（换粉描边 + 抖动 0.36s + 文案改「先选择一台设备」1.8s 后回退）。这是产品原则的界面化：点对点，不猜目的地。

### 传输中的提示：交替高亮（不是连线）

**连线已废弃**（2026-09-28 定案）：环上的气泡本来就贴在本机盘边上，一条线几乎没有信息量，
而「光在两台之间来回跳」既说清了**哪两台**在传，又说清了**还在传**（灯一停 = 传完）。

| 状态 | 表现 |
|---|---|
| 已连接（静止） | 缩小后的盘 + 环拉满 + 常驻呼吸；**不画线** |
| 正在传数据 | 本机亮 → 对端亮 → 本机亮…（周期 1.2s，各半周期一条升余弦包络），呼吸加快到 1.5s |

- 强度在「已连接」与「亮起」之间插值：设备环 0.5→1.0、辉光 16/0.2→26/0.55。
  幅度拉这么大是为了**读得出来**——「在传」不放第五种颜色，全靠这一亮一暗说。
- 任一时刻**只有一端亮**（交替而不是同闪）：`model::transfer::pulse()` 是纯函数，有测试钉住。

### 传输卡（xfer-card）

200px、右下角、`card-bg` + 20px backdrop blur。四行：文件名 + 百分比（baseline 对齐）、4px 渐变条、9px 元信息、完成后的两个小按钮。**卡片只在传输态出现**，完成后不自动消失（用户自己关）。

进度条填充有 0.12s 线性过渡——防止 IPC 进度跳跃看起来像"抽动"。

**实现注记（2026-09-28 落地）**：

- 填充的 0.12s 过渡用**弹簧**做（`response = 0.12`，临界阻尼）：GPUI 没有 CSS transition，
  而弹簧本来就是全项目唯一的运动原语（见《气泡动画GPUI实践.md》§2）。
  百分比**文字**取未平滑的真实值——数字要准，条要顺。
- **宽度与下方拖拽区一致**（320 − 16×2 = 288、左右各 16 边距）：一列到底，不再是一张小卡。
- 底是**磨砂玻璃**：半透明底 + `backdrop_filter` 20px（设计稿的玻璃拟物）。
- **右上角圆形 ✕**：在传 = **取消传输**，结束 = 关掉卡片（同一个位置同一个图标，语义随状态变）。
  hover 变红 `#ff5f57`（房规第二处用红，与窗口关闭键同源）。
- 元信息行只有两个数：`2.4 MB / 3.9 MB · 剩余 4 秒`——不写方向、不写速度
  （剩余时间已经是速度的结论，两个一起放是重复）。速度不明时只有前半段，**不编「剩 0 秒」**。
- 卡片里的数字全部来自真实字节与耗时：速度是**1 秒滑动窗口**（`(最新字节−窗口最早字节)/真实间隔`），
  速度为 0 时**不显示速度也不显示 ETA**（不编造「剩 0 秒」）。
- 元信息行：`<发送|接收>中 · 已传/总量 · 速度 · 剩余`；完成/被拒/失败各有自己的文案，
  失败时**如实转述守护进程给的原因**。
- 按钮：完成后才出现。**只有收下来的文件**给「打开目录」（发送方本机本来就有这个文件）；
  「关掉卡片」由右上角那个 ✕ 承担（关掉的编号记在本机，不再弹回）。
- 层级：卡片画在拖拽区**之上**（按本节的层级顺序）。

### 隐私浮层（privacy-popover）

点标题栏的**盾牌**图标打开：236px、14px 圆角、三条承诺 + 一条脚注。**内容是事实陈述，不是宣传**：端口号、机制、加密状态都必须与实现一致。点外部或 Esc 关闭。

- 图标从设计稿的锁改成**盾牌**（用户拍板）：锁说的是"加密"，盾牌说的是"保护与承诺"，
  而这一层的内容正是承诺（无云端 / 无中转 / 无账号 / 无日志）——三者里它更贴题。
  画法按图标房规：14 画布 + 1.3 描边 + 平顶圆肩（圆角与全项目的圆角语言同源，
  也是与通用盾牌区分开的地方）。
- **落地注意**：三条承诺与脚注都必须与实现一致——脚注里的端口是 **9267**（设计稿示例写的是 8888），
  机制是"UDP 广播发现 + TCP 点对点传输，文件端到端加密"。

### 配对（pair-card / pair-code-input）—— 本案新增

设计稿没有这一态，这里定义**最小新增**，全部复用既有 token，**不新增颜色**：

- **拖动中**：被拖节点跟随指针并轻微放大，中心盘高亮（环满 tone），松手落在中心 84px 内才生效，否则回弹。
- **发起侧等待**：中心盘显示"等待对方允许"，无卡片。
- **发起侧出示码**：`pair-card` 浮出，22px 等宽 `tnum` 的 6 位码 + 一句"请在此设备上输入此码"。这是**唯一需要用户隔屏读数的地方**，所以码必须是全窗最大的字。
- **接收侧询问**：卡片 + 「XX 请求配对」+ 允许 / 拒绝两个按钮。**先问后输**：未点允许不出现输入框。
- **接收侧输入**：6 个 26px 方格，只吃数字键与退格，填满自动提交。
- **失败**：文案 + 抖动（`pair-code-input-error`），错误 3 次作废。**失败绝不用红色**——粉是"台式机"的语义色，借来当错误色会污染类型通道。

### 已信任节点标识（待定）

设计稿没有这个信息，**当前不做特殊标记**。理由：类型通道已被四种辉光占用，把"状态"塞进同一个通道会污染语义；而"本机信任对方 ≠ 对方信任本机"，视觉上标了反而容易让用户以为对方也认自己。**待有真实使用反馈后再定**（候选：环改实线 / 图标旁加小锁点 / 不标）。

## Do's and Don'ts

- **Do** 让辉光色只出现在节点环、图标与进度条填充上；其余一切——面、线、文字——都是中性色。
- **Don't** 新增第五种彩色。错误、警告、成功**一律不用新颜色**：错误用文案 + 抖动，成功用文案 + 环的强度。
  （已记录的例外：关闭按钮 hover 的红底，见“辉光即语义”那条；除它之外不要再开第二个口子。）
- **Do** 用绝对坐标实现节点图；**Don't** 把它改成流式布局或加滚动。
- **Do** 用等宽数字（`tnum`）渲染一切会跳的数字（百分比、速度、剩余时间、字节）。
- **Don't** 用投影表达"元件浮起"——那是浮雕（inset）的职责；投影只给"层"（窗口、卡片）。
- **Don't** 在窗口内做接收确认弹窗：接收确认走**系统级通知**（PRD §4.3），窗口内只做待确认的**最小提示条**。
- **Don't** 放主题开关、平台切换、设备选择器这类"设计器控件"在窗口里。
- **Don't** 写没有来源的指标（"10× 更快"）；传输卡里的所有数字必须来自真实字节与耗时。
- **Don't** 用 emoji 当图标，不要手绘 SVG 人物，不要"左侧色条卡片"。
- **Do** 让浮层内容可核对：隐私承诺里的端口与机制必须与实现逐字一致。

### 已知对比度缺口（保持设计值 + 用法约束；AA 安全值待定）

三处是设计稿的取值，按真实复合底色算**不达 AA 4.5:1**。处理原则：**保留设计值**（保真优先），但**约束用法**，同时把候选修正值列在这里待定：

| token | 现状 | 实测 | 用法约束 | 候选 AA 安全值 |
|---|---|---|---|---|
| `text-hint`（深，白 40%） | 传输卡的 9px 速度/剩余时间 | 3.83:1 | 只承载**辅助**信息，缺了不影响理解；不得用它标状态 | 白 46% ≈ #808184（4.68:1）|
| `text-muted`（深，白 35%） | 拖拽提示、空态提示 | 3.21:1 | 同上；**不得承载操作语义** | 白 46% ≈ #7e7f83（4.67:1）|
| `pct-light` #00a8ad | 浅色传输卡百分比 | 2.87:1 | 11px/600 的数字，属**可读性真问题** | #007d81（4.86:1）|

**待定**：这三处是否切换到候选值（会牺牲与设计稿的像素一致）；其中 `pct-light` 建议优先修，因为它承载的是用户真正在读的数字。
