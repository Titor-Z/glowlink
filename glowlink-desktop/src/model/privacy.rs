//! Model：隐私浮层（标题栏盾牌点开的那张卡）。
//!
//! 全部是纯数据与纯函数，所以能单测——而这块**必须**测得动：
//! 设计稿的原话是「内容是事实陈述，不是宣传」，也就是说这里的每个字都要与实现对得上。
//! 尤其那条脚注里的端口号：写死在文案里迟早会和 `DISCOVERY_PORT` 漂移，
//! 所以它从 core 的常量生成，测试再把两者钉在一起。

use std::time::Duration;

use glowlink_core::DISCOVERY_PORT;

/// 三条承诺（设计稿原文，**顺序也是规格**）。
///
/// 这三条是产品的立身之本，不是形容词：不经过任何服务器、不留账号、不留日志。
pub const PROMISES: [&str; 3] = ["无云端 · 无中转", "无注册 · 无账号", "无遥测 · 无日志"];

/// 浮层标题。
pub const TITLE: &str = "隐私承诺";

/// 脚注：告诉用户**怎么自己验**，并交代机制与加密状态。
///
/// 端口来自 `glowlink_core::DISCOVERY_PORT`（唯一来源）：设计稿示例写的是 8888，
/// 那是早期端口，实现早已改成 9267 —— 事实陈述里出现过期数字，比不写更伤。
pub fn footnote() -> String {
    format!("可自行抓包验证：仅 UDP 广播 {DISCOVERY_PORT} 与 TCP 点对点数据，文件端到端加密。")
}

/// 浮层尺寸（设计稿 `.privacy` 的数值，一处定义）。
pub const WIDTH: f32 = 236.0;
pub const TOP: f32 = 48.0;
pub const RIGHT: f32 = 10.0;
pub const RADIUS: f32 = 14.0;
pub const PAD: f32 = 12.0;
/// 每行前面那颗圆点的直径（设计稿 `4px`）与行间距。
pub const DOT: f32 = 4.0;
pub const ROW_GAP: f32 = 8.0;
/// 入场时长（设计稿 `cardIn 0.2s`）。
pub const IN_MS: u64 = 200;
/// 入场时的位移量（设计稿的 `cardIn` 是从上方 4px 落下来）。
pub const IN_SHIFT: f32 = 4.0;

/// 入场进度 → （不透明度，向下位移）。
///
/// 曲线用三次缓出（≈ 设计稿的 `cubic-bezier(.22,1,.36,1)`）：快起慢收，
/// 玻璃卡片这样落下才像「材料到位」，而不是「淡入一个矩形」。
///
/// 进度**夹到 0..1**：调用方可能给出略超一步的值（帧率抖动），
/// 超过就按落定算，免得卡片越跑越远。
pub fn pop_in(progress: f32) -> (f32, f32) {
    let t = progress.clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t).powi(3);
    (eased, IN_SHIFT * (1.0 - eased))
}

/// 入场还要不要继续（跑完就不必每帧重绘）。
pub fn animating(progress: f32) -> bool {
    progress < 1.0
}

/// 入场时长（`Duration` 形态，给时钟用）。
pub const IN: Duration = Duration::from_millis(IN_MS);
