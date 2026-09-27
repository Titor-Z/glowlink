//! `model/drag.rs` 的测试：拖动配对的载荷与文案。
//!
//! 拖放本身由 gpui 承担（要真实窗口 + 真实指针），能测的就是这两样：
//! 拖的是什么、以及配对卡上写什么。

use glowlink_desktop::model::drag::{pair_state_note, DeviceDrag, GHOST_SCALE};
use glowlink_core::DeviceKind;
use glowlink_ipc::row::PairingState;

#[test]
fn every_pairing_state_has_its_own_sentence() {
    // 设计稿硬约束：失败不用新颜色，状态全靠文案区分——
    // 所以每种状态都必须有话说，漏一个就会显示一句空话。
    let states = [
        PairingState::WaitingPeer,
        PairingState::WaitingUser,
        PairingState::Paired,
        PairingState::Denied,
        PairingState::Failed,
    ];
    let mut notes: Vec<&str> = Vec::new();
    for state in states {
        let note = pair_state_note(state);
        assert!(!note.is_empty(), "{state:?} 没有文案");
        assert!(!notes.contains(&note), "{state:?} 与别人共用一句文案：{note}");
        notes.push(note);
    }
    assert_eq!(pair_state_note(PairingState::Denied), "对方拒绝了配对");
}

#[test]
fn the_ghost_is_only_slightly_bigger() {
    // 设计稿「轻微放大」：太大就变成「拿起一个气球」，看着不像是同一枚节点
    assert!(GHOST_SCALE > 1.0 && GHOST_SCALE <= 1.15, "实际 {GHOST_SCALE}");
}

#[test]
fn drag_payload_carries_the_identity_not_just_the_name() {
    // 重名设备是真实存在的（同一台机器跑两个 HOME 就是同一个名字）：
    // 配对必须按 ID 发起，不能按名字
    let a = DeviceDrag { id: "aaa".into(), name: "同名机".into(), kind: DeviceKind::Desktop };
    let b = DeviceDrag { id: "bbb".into(), name: "同名机".into(), kind: DeviceKind::Desktop };
    assert_ne!(a, b);
    assert_eq!(a.id, "aaa");
}
