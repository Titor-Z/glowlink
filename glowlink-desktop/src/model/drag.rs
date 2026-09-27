//! Model：拖动设备节点去做配对。
//!
//! 拖放本身由 gpui 的 `on_drag` / `on_drop` 承担（见 `view/app.rs`），
//! 这里只放**能单测**的那部分：拖的是什么、拖影多大、配对卡上写什么。

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
        PairingState::Failed => "配对失败，可重新拖动",
    }
}

/// 发起侧出示码时的那句话（设计稿原文）。
pub const PAIR_CODE_HINT: &str = "请在此设备上输入此码";
