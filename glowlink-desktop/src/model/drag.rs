//! Model：拖动设备节点去做配对。
//!
//! 拖放本身由 gpui 的 `on_drag` / `on_drop` 承担（见 `view/app.rs`），
//! 这里只放**能单测**的那部分：拖的是什么、拖影多大、配对卡上写什么。

use crate::model::layout;
use glowlink_core::DeviceKind;
use glowlink_ipc::row::PairingState;

/// 拖动时被拖节点的放大比例（设计稿「轻微放大」，不夸张）。
pub const GHOST_SCALE: f32 = 1.08;

/// 拖影的三要素。`id` 是设备 ID（身份指纹），落点判定后靠它发起配对。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceDrag {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
}

/// 配对卡上的那句状态说明。
///
/// **失败不用新颜色**（设计稿硬约束）：状态全部靠文案区分，
/// 码本身永远是同一个 22px 等宽样式。
pub fn pair_state_note(state: PairingState) -> &'static str {
    match state {
        PairingState::WaitingPeer => "等待对方允许",
        PairingState::WaitingUser => "请在此设备上输入此码",
        PairingState::Paired => "已配对，下次免核对",
        PairingState::Denied => "对方拒绝了配对",
        PairingState::Abandoned => "已放弃配对",
        PairingState::Failed => "配对失败，可重新拖动",
    }
}

/// 发起侧出示码时的那句话（设计稿原文）。
pub const PAIR_CODE_HINT: &str = "请在对方设备上输入此码";
/// 接收侧（输入方）的提示：码在对方屏幕上，**自己这台没有码**。
pub const PAIR_INPUT_HINT: &str = "请输入对方屏幕上显示的 6 位数字";
/// 询问阶段（接收侧）：要不要允许这次配对。
pub const PAIR_ASK_HINT: &str = "允许后需要用对方屏幕上的 6 位数字确认";

/// 吸附余量：气泡盘与中心盘相切之外再给这么多（手抖余量）。
///
/// 口径：**气泡自己**贴到中心盘上就算吸附——不看指针。以前按指针算，
/// 抓在盘边上时指针要多走 20~30px 才吸得上（用户真机感觉「必须走到正中心」），
/// 而反方向又会「还没到就吸上」：同一个偏移在两侧表现相反。
pub const SNAP_MARGIN: f32 = 6.0;

/// 吸附判定：气泡圆心到中心盘圆心的距离 ≤ 双方半径 + 余量。
///
/// **界面高亮与最终吸附必须共用这一个函数**：高亮是承诺，亮了就得吸上
/// （apple-design skill §8「动作要预告结果」、§10「命中口径要一致」）。
/// 半径取**当前**值（气泡拖动中会放大到 1.08、连接后会缩小）。
pub fn snap_hit(node_center: layout::Point, self_center: layout::Point, self_radius: f32, node_radius: f32) -> bool {
    layout::distance(node_center, self_center) <= self_radius + node_radius + SNAP_MARGIN
}

/// 发起侧卡片「等多久还没答复才把码亮出来」。
///
/// 信任的设备几乎立刻答复（对端免码放行），这段时间内卡片根本不出现 ⇒「无感」。
/// 真需要人工核对（陌生设备、或**对端**不信任本机）时，码照样会亮，只是晚 0.8 秒。
pub const CODE_DELAY_MS: u64 = 800;

/// 发起侧卡片该不该显示？
///
/// - `Paired`：不打扰（吸附完成就是回执，不需要再告诉用户「已配对」）；
/// - `WaitingPeer`：先不打扰，等到超过 [`CODE_DELAY_MS`] 才亮码；
/// - 被拒 / 失败 / 本机放弃：如实说（这是用户需要知道的）。
pub fn show_pair_card(state: PairingState, waited_ms: u64) -> bool {
    match state {
        PairingState::Paired => false,
        PairingState::WaitingPeer => waited_ms >= CODE_DELAY_MS,
        _ => true,
    }
}
