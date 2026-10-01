//! `model/drop.rs` 的测试：拖拽区的文案与投放判定。
//!
//! 渲染测不动（要真实窗口 + 真实 OS 拖放），所以规则全部抽成纯函数钉在这里。

// 判定按**设备 ID**（不是名字）：名字可以重，而「发给谁」必须唯一。
use glowlink_desktop::model::drop::{accept_target, DropState};
use glowlink_desktop::model::state::ServiceState;

#[test]
fn the_zone_note_always_says_what_can_happen_now() {
    use glowlink_desktop::model::drop::Zone;
    // 一台都没有：说清**现状**（不是「拖拽文件到这里」那种必然被拒的邀请，
    // 也不是「去打开另一台设备」那种冲着对端发的指令）
    let empty = Zone { service: ServiceState::On, device_count: 0, connected: 0, selected: None, only: None };
    assert_eq!(DropState::Idle.note(empty), "暂无可发送的设备");
    assert_eq!(DropState::Over.note(empty), "暂无可发送的设备");

    // 多台、没选：**提前**说「先选择一台设备」（用户拍板：别等丢进去才拒）
    let many = Zone { service: ServiceState::On, device_count: 3, connected: 3, selected: None, only: None };
    assert_eq!(DropState::Idle.note(many), "先选择一台设备");
    assert_eq!(DropState::Over.note(many), "先选择一台设备");

    // 只有一台：**免选**，它就是目标——把「发给谁」摆在明面上
    let single = Zone { service: ServiceState::On, device_count: 1, connected: 1, selected: None, only: Some("甲机") };
    assert_eq!(single.target(), Some("甲机"));
    assert_eq!(DropState::Idle.note(single), "拖到此处 → 甲机");

    // 选中了：显示目标名（多台时也一样）；悬停不改变「发给谁」
    let picked = Zone { service: ServiceState::On, device_count: 3, connected: 3, selected: Some("乙机"), only: None };
    assert_eq!(DropState::Idle.note(picked), "拖到此处 → 乙机");
    assert_eq!(DropState::Over.note(picked), "拖到此处 → 乙机");
}

#[test]
fn denied_text_never_shows_a_target() {
    use glowlink_desktop::model::drop::Zone;
    // 被拒的原因是「还没选」——写上目标名会让人以为已经选好了
    let denied = DropState::Denied("先选择一台设备");
    let picked = Zone { service: ServiceState::On, device_count: 3, connected: 3, selected: Some("甲机"), only: None };
    assert_eq!(denied.note(picked), "先选择一台设备");
    assert!(denied.is_denied());
    assert!(!DropState::Over.is_denied(), "悬停不是失败");
    assert!(!DropState::Idle.is_denied());
}

#[test]
fn a_single_device_does_not_need_to_be_selected_first() {
    // 用户拍板的语义：一台设备时直接发给它，多台才要求先点选
    assert_eq!(accept_target(&["甲机"], &["甲机"], None).unwrap(), "甲机");
    assert_eq!(accept_target(&["甲机"], &["甲机"], Some("甲机")).unwrap(), "甲机");
}

#[test]
fn several_devices_require_an_explicit_pick() {
    let devices = ["甲机", "乙机"];
    assert_eq!(accept_target(&devices, &devices, None), Err("先选择一台设备"));
    assert_eq!(accept_target(&devices, &devices, Some("乙机")).unwrap(), "乙机");
}

#[test]
fn no_devices_at_all_says_so() {
    // 「附近没有设备」和「先选择一台设备」是两回事：
    // 前者用户去开对端，后者用户点一下就行
    assert_eq!(accept_target(&[], &[], None), Err("附近没有设备"));
    assert_eq!(accept_target(&[], &[], Some("甲机")), Err("附近没有设备"));
}

#[test]
fn a_stale_selection_is_not_silently_replaced() {
    // 选中的设备刚好下线时**不许换一台发出去**：用户以为发给了甲、实际发给了乙，
    // 是最难发现的那种错。宁可拒收并说清楚。
    assert_eq!(accept_target(&["乙机"], &["乙机"], Some("甲机")), Err("那台设备已不在了"));
    assert_eq!(accept_target(&["甲机", "乙机"], &["甲机", "乙机"], Some("丙机")), Err("那台设备已不在了"));
}

#[test]
fn the_zone_says_the_service_is_off_before_anything_else() {
    use glowlink_desktop::model::drop::Zone;
    use glowlink_desktop::model::state::ServiceState;
    // 服务关着的时候，说什么「几台设备 / 发给谁」都是空话：先说要紧的
    let off = Zone { service: ServiceState::Off, device_count: 0, ..Default::default() };
    assert_eq!(DropState::Idle.note(off), "已暂停查找设备");
    let starting = Zone { service: ServiceState::Starting, device_count: 0, ..Default::default() };
    assert_eq!(DropState::Idle.note(starting), "正在启动…");
    // 服务在跑才轮到设备那几句话
    let on = Zone {
        service: ServiceState::On,
        device_count: 1,
        connected: 1,
        only: Some("甲机"),
        selected: None,
    };
    assert_eq!(DropState::Idle.note(on), "拖到此处 → 甲机");
}

#[test]
fn sending_requires_a_connection() {
    // 用户拍板的流程规则：**发现 ≠ 可以发**。刚看见的陌生设备还没配对，
    // 拖拽区不能写「拖到此处 → 它」——那是承诺一件流程不允许的事。
    use glowlink_desktop::model::drop::{accept_target, DropState, Zone, CONNECT_FIRST, CONNECT_HINT};
    use glowlink_desktop::model::state::ServiceState;

    // 有一台设备，但没连：拒收，且说清要做什么
    assert_eq!(accept_target(&["a"], &[], None), Err(CONNECT_FIRST));
    // 选了一台没连的：也是同一句（不是「不在了」——设备就在那儿，只是没连）
    assert_eq!(accept_target(&["a"], &[], Some("a")), Err(CONNECT_FIRST));
    // 连上之后才行；一台免选
    assert_eq!(accept_target(&["a"], &["a"], None), Ok("a".to_string()));

    // 文案同一条规矩：有设备但没连接 → 「先把设备拖到中间连接」
    let seen = Zone { service: ServiceState::On, device_count: 1, connected: 0, ..Default::default() };
    assert_eq!(DropState::Idle.note(seen), CONNECT_HINT);
    // 已连接一台 → 才写「拖到此处 → 甲机」
    let linked = Zone {
        service: ServiceState::On,
        device_count: 1,
        connected: 1,
        only: Some("甲机"),
        selected: None,
    };
    assert_eq!(DropState::Idle.note(linked), "拖到此处 → 甲机");
    // 已连接多台、没选 → 先选择
    let many = Zone { service: ServiceState::On, device_count: 3, connected: 3, ..Default::default() };
    assert_eq!(DropState::Idle.note(many), "先选择一台设备");
}

#[test]
fn a_selected_but_unconnected_device_is_not_a_target() {
    use glowlink_desktop::model::drop::{accept_target, DropState, Zone, CONNECT_FIRST, CONNECT_HINT};
    use glowlink_desktop::model::state::ServiceState;
    // 选中了甲（已连）、乙没连：目标仍是甲；乙不参与
    assert_eq!(accept_target(&["a", "b"], &["a"], Some("b")), Err(CONNECT_FIRST));
    // 选中的那台已连接、另一台没连：就用选中的
    assert_eq!(accept_target(&["a", "b"], &["a"], Some("a")), Ok("a".to_string()));
    // 已连接两台、选中其中一台 → 就是它
    assert_eq!(accept_target(&["a", "b"], &["a", "b"], Some("b")), Ok("b".to_string()));
    // 文案：选中的没连 → 也不能写目标名
    let z = Zone {
        service: ServiceState::On,
        device_count: 2,
        connected: 0,
        only: None,
        selected: Some("乙机"),
    };
    assert_eq!(DropState::Idle.note(z), CONNECT_HINT);
}
