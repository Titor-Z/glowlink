//! `model/drop.rs` 的测试：拖拽区的文案与投放判定。
//!
//! 渲染测不动（要真实窗口 + 真实 OS 拖放），所以规则全部抽成纯函数钉在这里。

use glowlink_desktop::model::drop::{accept_target, DropState};

#[test]
fn idle_text_follows_the_selection() {
    // 设计稿（glowlink.js）：没选 → 「拖拽文件到这里」；选了 → 「拖到此处 → 甲机」
    assert_eq!(DropState::Idle.note(None), "拖拽文件到这里");
    assert_eq!(DropState::Idle.note(Some("甲机")), "拖到此处 → 甲机");
    // 悬停中说的是同一句话：拖入本身不改变「发给谁」
    assert_eq!(DropState::Over.note(Some("甲机")), "拖到此处 → 甲机");
}

#[test]
fn denied_text_never_shows_a_target() {
    // 被拒的原因是「还没选」——写上目标名会让人以为已经选好了
    let denied = DropState::Denied("先选择一台设备");
    assert_eq!(denied.note(Some("甲机")), "先选择一台设备");
    assert!(denied.is_denied());
    assert!(!DropState::Over.is_denied(), "悬停不是失败");
    assert!(!DropState::Idle.is_denied());
}

#[test]
fn a_single_device_does_not_need_to_be_selected_first() {
    // 用户拍板的语义：一台设备时直接发给它，多台才要求先点选
    assert_eq!(accept_target(&["甲机"], None).unwrap().as_deref(), Some("甲机"));
    assert_eq!(accept_target(&["甲机"], Some("甲机")).unwrap().as_deref(), Some("甲机"));
}

#[test]
fn several_devices_require_an_explicit_pick() {
    let devices = ["甲机", "乙机"];
    assert_eq!(accept_target(&devices, None), Err("先选择一台设备"));
    assert_eq!(accept_target(&devices, Some("乙机")).unwrap().as_deref(), Some("乙机"));
}

#[test]
fn no_devices_at_all_says_so() {
    // 「附近还没有设备」和「先选择一台设备」是两回事：
    // 前者用户去开对端，后者用户点一下就行
    assert_eq!(accept_target(&[], None), Err("附近还没有设备"));
    assert_eq!(accept_target(&[], Some("甲机")), Err("附近还没有设备"));
}

#[test]
fn a_stale_selection_is_not_silently_replaced() {
    // 选中的设备刚好下线时**不许换一台发出去**：用户以为发给了甲、实际发给了乙，
    // 是最难发现的那种错。宁可拒收并说清楚。
    assert_eq!(accept_target(&["乙机"], Some("甲机")), Err("选中的设备已不在了"));
    assert_eq!(accept_target(&["甲机", "乙机"], Some("丙机")), Err("选中的设备已不在了"));
}
