//! `view/tui.rs` 的状态机测试。
//!
//! 界面渲染依赖真实终端，没法在测试里跑；但「上下选择、刷新后保持选中、
//! 输入路径、回车确认」这些是纯逻辑，必须在这里钉死。

use std::net::SocketAddr;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use glowlink_cli::view::tui::{apply_key, parse_path_input, DeviceList, TuiAction};
use glowlink_core::model::device::{Device, DeviceId};

fn device(id: &str, name: &str, port: u16) -> Device {
    Device {
        id: DeviceId::new(id),
        name: name.to_string(),
        addr: format!("127.0.0.1:{port}").parse::<SocketAddr>().unwrap(),
        last_seen: Instant::now(),
    }
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn three() -> DeviceList {
    let mut list = DeviceList::new();
    list.set_devices(vec![
        device("aaaa1111", "甲机", 9001),
        device("bbbb2222", "乙机", 9002),
        device("cccc3333", "丙机", 9003),
    ]);
    list
}

fn no_refresh() -> impl FnMut() -> Vec<Device> {
    Vec::new
}

#[test]
fn selection_moves_and_stops_at_the_ends() {
    let mut list = three();
    assert_eq!(list.selected_index(), 0);

    // 到顶再往上不越界、不到尾再往下不越界（不循环）
    list.select_previous();
    assert_eq!(list.selected_index(), 0);
    list.select_next();
    list.select_next();
    assert_eq!(list.selected_index(), 2);
    list.select_next();
    assert_eq!(list.selected_index(), 2);
    assert_eq!(list.selected().unwrap().name, "丙机");
}

#[test]
fn empty_list_has_no_selection() {
    let list = DeviceList::new();
    assert!(list.selected().is_none());
    assert_eq!(list.devices().len(), 0);
    // 空列表也能安全回车（不该进入输入模式，更不该 panic）
    let mut list = list;
    assert!(!list.begin_input());
    assert!(!list.input_active());
}

#[test]
fn refresh_keeps_you_on_the_same_device() {
    let mut list = three();
    list.select_next();
    list.select_next();
    assert_eq!(list.selected().unwrap().name, "丙机");

    // 中途有设备消失、顺序也变了：选中的那台还在，就必须还停在它身上
    list.set_devices(vec![
        device("cccc3333", "丙机", 9003),
        device("aaaa1111", "甲机", 9001),
    ]);
    assert_eq!(list.selected().unwrap().name, "丙机", "刷新后选中项不该跳回第一行");

    // 选中的那台没了：下标夹回有效范围
    list.set_devices(vec![device("aaaa1111", "甲机", 9001)]);
    assert_eq!(list.selected_index(), 0);
    assert_eq!(list.selected().unwrap().name, "甲机");
}

#[test]
fn input_mode_collects_a_path_and_confirms() {
    let mut list = three();
    assert!(list.begin_input());
    assert!(list.input_active());
    for ch in "/tmp/报告.pdf".chars() {
        list.push_char(ch);
    }
    assert_eq!(list.input(), "/tmp/报告.pdf");
    // 退格删掉最后一个字符，后面的期望值要跟着少一个
    list.pop_char();

    let action = list.confirm_input().unwrap();
    match action {
        TuiAction::Send { addr, name, path } => {
            assert_eq!(name, "甲机");
            assert_eq!(addr.port(), 9001);
            assert_eq!(path.to_string_lossy(), "/tmp/报告.pd", "退格应删掉末尾的 f");
        }
        other => panic!("应当是发送动作，实际 {other:?}"),
    }
    // 确认后回到浏览模式，输入被清空
    assert!(!list.input_active());
    assert_eq!(list.input(), "");
}

/// 拖拽进终端得到的是带引号/转义的文本，必须还原成真正的路径。
#[test]
fn dragged_in_paths_are_unquoted_and_unescaped() {
    // 普通输入（含首尾空白）
    assert_eq!(parse_path_input("  /tmp/a.bin  ").to_string_lossy(), "/tmp/a.bin");
    // macOS / iTerm 拖拽：单引号包裹
    assert_eq!(parse_path_input("'/tmp/有 空格.pdf'").to_string_lossy(), "/tmp/有 空格.pdf");
    // 双引号包裹
    assert_eq!(parse_path_input("\"/tmp/有 空格.pdf\"").to_string_lossy(), "/tmp/有 空格.pdf");
    // 反斜杠转义空格
    assert_eq!(parse_path_input("/tmp/有\\ 空格.pdf").to_string_lossy(), "/tmp/有 空格.pdf");
    // Windows 路径里的反斜杠不能被当成转义
    assert_eq!(
        parse_path_input("C:\\Users\\titor\\报告.pdf").to_string_lossy(),
        "C:\\Users\\titor\\报告.pdf"
    );
    // 半截引号不动它；空输入仍是空（由调用方当取消处理）
    assert_eq!(parse_path_input("\"/tmp/a'").to_string_lossy(), "\"/tmp/a'");
    assert_eq!(parse_path_input("   ").to_string_lossy(), "");
}

#[test]
fn enter_with_empty_path_cancels_instead_of_sending() {
    let mut list = three();
    assert!(list.begin_input());
    assert!(list.confirm_input().is_none(), "空路径不能变成一次发送");
}

#[test]
fn escape_quits_from_browse_and_cancels_from_input() {
    let mut list = three();
    // 输入模式下 Esc 只取消输入，不能顺手把界面也关了
    list.begin_input();
    let mut refresh = no_refresh();
    assert_eq!(apply_key(&mut list, press(KeyCode::Esc), &mut refresh), None);
    assert!(!list.input_active());
    // 浏览模式下 Esc 才是退出
    assert_eq!(
        apply_key(&mut list, press(KeyCode::Esc), &mut refresh),
        Some(TuiAction::Quit)
    );
}

#[test]
fn only_key_presses_are_accepted() {
    // Windows 上会同时收到按下与抬起；只认按下，否则一次按键算两次
    let mut list = three();
    let release = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(release.kind, KeyEventKind::Press, "构造默认是按下，用它测语义");
    let mut refresh = no_refresh();
    apply_key(&mut list, press(KeyCode::Down), &mut refresh);
    assert_eq!(list.selected_index(), 1);
}

#[test]
fn enter_on_a_device_enters_input_then_sends() {
    let mut list = three();
    let mut refresh = no_refresh();
    // 选中第二台，回车进输入模式，输入路径，再回车 → 定向到乙机
    apply_key(&mut list, press(KeyCode::Down), &mut refresh);
    assert_eq!(
        apply_key(&mut list, press(KeyCode::Enter), &mut refresh),
        None,
        "第一次回车只是进入输入模式"
    );
    assert!(list.input_active());
    for ch in "/tmp/a.bin".chars() {
        apply_key(&mut list, press(KeyCode::Char(ch)), &mut refresh);
    }
    let action = apply_key(&mut list, press(KeyCode::Enter), &mut refresh).unwrap();
    match action {
        TuiAction::Send { name, path, .. } => {
            assert_eq!(name, "乙机");
            assert_eq!(path.to_string_lossy(), "/tmp/a.bin");
        }
        other => panic!("应当是发送动作，实际 {other:?}"),
    }
}

#[test]
fn refresh_key_pulls_new_devices_into_the_list() {
    let mut list = three();
    let mut refresh = || vec![device("dddd4444", "丁机", 9004)];
    apply_key(&mut list, press(KeyCode::Char('r')), &mut refresh);
    assert_eq!(list.devices().len(), 1);
    assert_eq!(list.devices()[0].name, "丁机");
}

#[test]
fn jk_are_aliases_for_arrows() {
    let mut list = three();
    let mut refresh = no_refresh();
    apply_key(&mut list, press(KeyCode::Char('j')), &mut refresh);
    assert_eq!(list.selected_index(), 1);
    apply_key(&mut list, press(KeyCode::Char('k')), &mut refresh);
    assert_eq!(list.selected_index(), 0);
}
