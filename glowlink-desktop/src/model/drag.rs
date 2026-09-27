//! Model：拖动设备节点去做配对。
//!
//! 拖放本身由 gpui 的 `on_drag` / `on_drop` 承担（见 `view/app.rs`），
//! 这里只放**能单测**的那部分：拖的是什么、拖影多大、配对卡上写什么。

use glowlink_core::DeviceKind;
use glowlink_ipc::row::PairingState;

/// 拖动时被拖节点的放大比例（设计稿「轻微放大」，不夸张）。
pub const GHOST_SCALE: f32 = 1.08;
/// 抓起的瞬间从这里长到 [`GHOST_SCALE`]：一步到位会显得很硬。
pub const GHOST_SCALE_MIN: f32 = 1.0;
/// 吸附判定半径（圆心距）。任务书 v1.0：**小于 50px 才算吸附成功**。
///
/// 比设计稿那句「中心 84px 的盘本身」（盘半径 42）略宽松 8px：
/// 盘缘再往外一点也算，手上不抖就好。
pub const SNAP_RADIUS: f32 = 50.0;
/// 位置发生变化的过渡时长（回弹、被撞开、连接重排都走它）。
pub const MOVE_MS: u64 = 280;
/// 节点淡出/淡入的时长（拖动开始时原件淡出、松手后淡入）。
pub const FADE_MS: u64 = 140;
/// 中心盘「被拖过来」的过渡时长（色与强度一起插值）。
pub const CENTER_TINT_MS: u64 = 220;

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
