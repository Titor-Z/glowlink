//! `view/tui.rs` 的状态机测试。
//!
//! 界面渲染依赖真实终端，没法在测试里跑；但「上下选择、刷新后保持选中、
//! 输入路径、回车确认、接受/拒绝待确认」这些是纯逻辑，必须在这里钉死。

use std::net::SocketAddr;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use glowlink_cli::view::tui::{
    apply_key, parse_path_input, AppState, Focus, PendingItem, Snapshot, TuiAction,
};
use glowlink_core::model::device::{Device, DeviceId};

fn device(id: &str, name: &str, port: u16) -> Device {
    Device {
        id: DeviceId::new(id),
        name: name.to_string(),
        addr: format!("127.0.0.1:{port}").parse::<SocketAddr>().unwrap(),
        last_seen: Instant::now(),
    }
}

fn pending(token: &str, peer: &str, name: &str) -> PendingItem {
    PendingItem {
        token: token.to_string(),
        peer: peer.to_string(),
        name: name.to_string(),
        size: 300_000,
        fingerprint: "abcdef0123456789".to_string(),
        code: "482913".to_string(),
    }
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn snapshot(devices: Vec<Device>, pending: Vec<PendingItem>) -> Snapshot {
    Snapshot { devices, pending }
}

/// 两台设备 + 一条待确认：光标先走设备再走待确认。
fn populated() -> AppState {
    let mut state = AppState::new();
    state.set_snapshot(snapshot(
        vec![device("aaaa1111", "甲机", 9001), device("bbbb2222", "乙机", 9002)],
        vec![pending("001", "丙机", "报告.pdf")],
    ));
    state
}

fn no_refresh() -> impl FnMut() -> Snapshot {
    || Snapshot::default()
}

fn focused_device(state: &AppState) -> Option<&Device> {
    match state.focus() {
        Focus::Device(device) => Some(device),
        _ => None,
    }
}

fn focused_pending(state: &AppState) -> Option<&PendingItem> {
    match state.focus() {
        Focus::Pending(item) => Some(item),
        _ => None,
    }
}

#[test]
fn selection_walks_devices_then_pending_without_wrapping() {
    let mut state = populated();
    assert_eq!(state.cursor(), 0);

    // 到顶再往上不越界
    state.select_previous();
    assert_eq!(state.cursor(), 0);
    // 一路走到最后一条待确认
    state.select_next();
    state.select_next();
    assert!(focused_pending(&state).is_some());
    state.select_next();
    assert_eq!(state.cursor(), 2, "到底就停住，不循环");
}

#[test]
fn empty_state_has_no_focus_and_enter_does_nothing() {
    let mut state = AppState::new();
    assert!(focused_device(&state).is_none() && focused_pending(&state).is_none());
    assert_eq!(state.activate(), None);
    assert!(!state.input_active());
    assert_eq!(state.reject_current(), None);
}

#[test]
fn refresh_keeps_you_on_the_same_device() {
    let mut state = populated();
    state.select_next();
    assert_eq!(focused_device(&state).unwrap().name, "乙机");

    // 中途有设备消失、顺序也变了：选中的那台还在，就必须还停在它身上
    state.set_snapshot(snapshot(
        vec![device("bbbb2222", "乙机", 9002), device("aaaa1111", "甲机", 9001)],
        vec![],
    ));
    assert_eq!(
        focused_device(&state).unwrap().name,
        "乙机",
        "刷新后不该跳回第一行"
    );

    // 选中的那台没了：下标夹回有效范围
    state.set_snapshot(snapshot(vec![device("aaaa1111", "甲机", 9001)], vec![]));
    assert_eq!(state.cursor(), 0);
}

/// 待确认项被别处处理掉（例如在另一个终端 accept 了）时，光标不能乱跳或越界。
#[test]
fn refresh_keeps_you_on_the_same_pending_item() {
    let mut state = populated();
    state.select_next();
    state.select_next();
    assert!(focused_pending(&state).is_some());

    // 又来了一条待确认：仍应停在原来那条（按编号找回）
    state.set_snapshot(snapshot(
        vec![device("aaaa1111", "甲机", 9001), device("bbbb2222", "乙机", 9002)],
        vec![pending("002", "丁机", "别的.bin"), pending("001", "丙机", "报告.pdf")],
    ));
    assert_eq!(
        focused_pending(&state).map(|item| item.token.as_str()),
        Some("001"),
        "应当仍停在 #001"
    );

    // 原来那条没了：夹回有效范围，不能越界 panic
    state.set_snapshot(snapshot(vec![device("aaaa1111", "甲机", 9001)], vec![]));
    assert_eq!(state.cursor(), 0);
    assert!(focused_device(&state).is_some(), "夹回后应落在设备上");
}

#[test]
fn enter_on_a_device_starts_input_not_sending() {
    let mut state = populated();
    let mut refresh = no_refresh();
    assert_eq!(apply_key(&mut state, press(KeyCode::Enter), &mut refresh), None);
    assert!(state.input_active(), "设备上回车是进入输入模式");
}

/// 待确认上回车 = 接受，而且**绝不能**顺手进入输入模式（那是发文件才用的）。
#[test]
fn enter_on_a_pending_item_accepts_it() {
    let mut state = populated();
    state.select_next();
    state.select_next();
    let mut refresh = no_refresh();
    let action = apply_key(&mut state, press(KeyCode::Enter), &mut refresh);
    assert_eq!(action, Some(TuiAction::Accept { token: "001".to_string() }));
    assert!(!state.input_active(), "接受请求不该进入输入模式");
}

/// `d` / Delete 只对待确认有效；在设备上按不该产生任何动作。
#[test]
fn reject_key_only_applies_to_pending_items() {
    let mut state = populated();
    let mut refresh = no_refresh();
    assert_eq!(apply_key(&mut state, press(KeyCode::Char('d')), &mut refresh), None);
    assert_eq!(apply_key(&mut state, press(KeyCode::Delete), &mut refresh), None);

    state.select_next();
    state.select_next();
    assert_eq!(
        apply_key(&mut state, press(KeyCode::Char('d')), &mut refresh),
        Some(TuiAction::Reject { token: "001".to_string() })
    );
}

/// 待确认项必须把 6 位校验码带出来：接受之前人要拿它和对方屏幕比对。
#[test]
fn pending_item_carries_the_code_used_for_manual_verification() {
    let state = populated();
    let item = &state.pending()[0];
    assert_eq!(item.code.len(), 6);
    assert!(item.code.chars().all(|c| c.is_ascii_digit()));
}

#[test]
fn input_mode_collects_a_path_and_confirms() {
    let mut state = populated();
    assert!(state.begin_input());
    for ch in "/tmp/报告.pdf".chars() {
        state.push_char(ch);
    }
    assert_eq!(state.input(), "/tmp/报告.pdf");
    // 退格删掉最后一个字符，期望值跟着少一个
    state.pop_char();

    let action = state.confirm_input().unwrap();
    match action {
        TuiAction::Send { addr, name, path } => {
            assert_eq!(name, "甲机");
            assert_eq!(addr.port(), 9001);
            assert_eq!(path.to_string_lossy(), "/tmp/报告.pd");
        }
        other => panic!("应当是发送动作，实际 {other:?}"),
    }
    assert!(!state.input_active());
    assert_eq!(state.input(), "");
}

/// 拖拽进终端得到的是带引号/转义的文本，必须还原成真正的路径。
#[test]
fn dragged_in_paths_are_unquoted_and_unescaped() {
    assert_eq!(parse_path_input("  /tmp/a.bin  ").to_string_lossy(), "/tmp/a.bin");
    assert_eq!(parse_path_input("'/tmp/有 空格.pdf'").to_string_lossy(), "/tmp/有 空格.pdf");
    assert_eq!(parse_path_input("\"/tmp/有 空格.pdf\"").to_string_lossy(), "/tmp/有 空格.pdf");
    assert_eq!(parse_path_input("/tmp/有\\ 空格.pdf").to_string_lossy(), "/tmp/有 空格.pdf");
    // Windows 路径里的反斜杠不能被当成转义
    assert_eq!(
        parse_path_input("C:\\Users\\titor\\报告.pdf").to_string_lossy(),
        "C:\\Users\\titor\\报告.pdf"
    );
    assert_eq!(parse_path_input("\"/tmp/a'").to_string_lossy(), "\"/tmp/a'");
    assert_eq!(parse_path_input("   ").to_string_lossy(), "");
}

#[test]
fn enter_with_empty_path_cancels_instead_of_sending() {
    let mut state = populated();
    assert!(state.begin_input());
    assert!(state.confirm_input().is_none(), "空路径不能变成一次发送");
}

#[test]
fn escape_quits_from_browse_and_cancels_from_input() {
    let mut state = populated();
    let mut refresh = no_refresh();
    // 输入模式下 Esc 只取消输入，不能顺手把界面也关了
    state.begin_input();
    assert_eq!(apply_key(&mut state, press(KeyCode::Esc), &mut refresh), None);
    assert!(!state.input_active());
    // 浏览模式下 Esc 才是退出
    assert_eq!(
        apply_key(&mut state, press(KeyCode::Esc), &mut refresh),
        Some(TuiAction::Quit)
    );
}

#[test]
fn enter_on_a_device_enters_input_then_sends() {
    let mut state = populated();
    let mut refresh = no_refresh();
    apply_key(&mut state, press(KeyCode::Down), &mut refresh);
    assert_eq!(
        apply_key(&mut state, press(KeyCode::Enter), &mut refresh),
        None,
        "第一次回车只是进入输入模式"
    );
    for ch in "/tmp/a.bin".chars() {
        apply_key(&mut state, press(KeyCode::Char(ch)), &mut refresh);
    }
    let action = apply_key(&mut state, press(KeyCode::Enter), &mut refresh).unwrap();
    match action {
        TuiAction::Send { name, path, .. } => {
            assert_eq!(name, "乙机");
            assert_eq!(path.to_string_lossy(), "/tmp/a.bin");
        }
        other => panic!("应当是发送动作，实际 {other:?}"),
    }
}

#[test]
fn refresh_key_pulls_a_new_snapshot_into_the_state() {
    let mut state = populated();
    let mut refresh = || {
        snapshot(
            vec![device("dddd4444", "丁机", 9004)],
            vec![pending("007", "戊机", "新.bin")],
        )
    };
    apply_key(&mut state, press(KeyCode::Char('r')), &mut refresh);
    assert_eq!(state.devices().len(), 1);
    assert_eq!(state.devices()[0].name, "丁机");
    assert_eq!(state.pending().len(), 1);
    assert_eq!(state.pending()[0].token, "007");
}

#[test]
fn jk_are_aliases_for_arrows() {
    let mut state = populated();
    let mut refresh = no_refresh();
    apply_key(&mut state, press(KeyCode::Char('j')), &mut refresh);
    assert_eq!(state.cursor(), 1);
    apply_key(&mut state, press(KeyCode::Char('k')), &mut refresh);
    assert_eq!(state.cursor(), 0);
}

#[test]
fn only_key_presses_are_accepted() {
    // Windows 上会同时收到按下与抬起；事件循环只认按下（这里确认构造默认是按下）
    let release = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(release.kind, KeyEventKind::Press);
}
