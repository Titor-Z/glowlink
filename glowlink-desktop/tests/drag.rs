//! `model/drag.rs` 的测试：拖动配对的载荷与文案。
//!
//! 拖放本身由 gpui 承担（要真实窗口 + 真实指针），能测的就是这两样：
//! 拖的是什么、以及配对卡上写什么。

use glowlink_desktop::model::drag::{pair_state_note, snap_hit, DeviceDrag, GHOST_SCALE};
use glowlink_desktop::model::layout::{self, Point};
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

#[test]
fn the_snap_zone_is_symmetric_in_every_direction() {
    // 用户真机抱怨过「有的方向要多走、有的方向没到就吸上」——那是拿**指针**算的。
    // 现在只看气泡自己，所以四个方向必须一模一样。
    let center = layout::self_center();
    let self_radius = layout::SELF_RADIUS;
    let node_radius = layout::DEVICE_RADIUS * GHOST_SCALE;
    let inside = self_radius + node_radius + 6.0 - 1.0;
    let outside = self_radius + node_radius + 6.0 + 1.0;
    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        let hit = Point { x: center.x + dx * inside, y: center.y + dy * inside };
        let miss = Point { x: center.x + dx * outside, y: center.y + dy * outside };
        assert!(snap_hit(hit, center, self_radius, node_radius), "该吸附：{hit:?}");
        assert!(!snap_hit(miss, center, self_radius, node_radius), "该落空：{miss:?}");
    }
}

#[test]
fn the_snap_zone_is_size_aware() {
    // 半径取**当前**值：缩小后的已连接盘、放大中的拖影，判定半径都跟着变
    let center = layout::self_center();
    let far = Point { x: center.x + 74.0, y: center.y };
    assert!(snap_hit(far, center, layout::SELF_RADIUS, 28.0), "拖影大一圈，够得着");
    assert!(!snap_hit(far, center, layout::SELF_RADIUS, 17.0), "缩小的盘够不着就是够不着");
}

#[test]
fn a_trusted_peer_never_flashes_a_code_card() {
    use glowlink_desktop::model::drag::{show_pair_card, CODE_DELAY_MS};
    use glowlink_ipc::row::PairingState;
    // 已信任的设备：对端免码放行，卡片只是「短暂路过」——不该显示
    assert!(!show_pair_card(PairingState::Paired, 0), "已配对不该弹卡片");
    assert!(
        !show_pair_card(PairingState::WaitingPeer, CODE_DELAY_MS - 1),
        "还没等够时间就别亮码（默认无感）"
    );
    // 等够了还没答复 ⇒ 说明对端要人工核对，这时必须把码亮出来
    assert!(show_pair_card(PairingState::WaitingPeer, CODE_DELAY_MS));
    // 失败 / 被拒 / 放弃：用户需要知道，立刻显示
    for state in [
        PairingState::Denied,
        PairingState::Failed,
        PairingState::Abandoned,
    ] {
        assert!(show_pair_card(state, 0), "{state:?} 应当立刻显示");
    }
}
