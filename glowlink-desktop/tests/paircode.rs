//! `glowlink-desktop/src/model/paircode.rs`：6 位配对码输入状态机。

use glowlink_desktop::model::paircode::{CodeInput, CODE_LEN};

#[test]
fn only_digits_are_accepted() {
    let mut input = CodeInput::new();
    assert!(input.push('7'));
    // 字母、空格、中文、符号一律不收（**明确忽略**，不报错）
    for ch in ['a', 'Z', ' ', '，', '-', '\n'] {
        assert!(!input.push(ch), "{ch:?} 不该被收下");
    }
    assert_eq!(input.digits(), "7");
}

#[test]
fn the_seventh_digit_does_not_displace_the_first() {
    // 满了就不许再吃：否则「多敲一下」会把第一位顶掉，用户看着 6 位却是错的
    let mut input = CodeInput::new();
    for ch in "123456".chars() {
        assert!(input.push(ch));
    }
    assert!(input.is_complete());
    assert!(!input.push('9'), "满 6 位后不该再收");
    assert_eq!(input.digits(), "123456");
    assert_eq!(input.digits().len(), CODE_LEN);
}

#[test]
fn backspace_only_removes_what_is_there() {
    let mut input = CodeInput::new();
    assert!(!input.backspace(), "空的时候退格不该有反应");
    input.push('4');
    input.push('2');
    assert!(input.backspace());
    assert_eq!(input.digits(), "4");
    assert!(input.backspace());
    assert!(input.digits().is_empty());
}

#[test]
fn cells_and_cursor_track_what_was_typed() {
    let mut input = CodeInput::new();
    assert_eq!(input.cursor(), 0);
    assert_eq!(input.cells(), ["", "", "", "", "", ""]);
    input.push('5');
    input.push('1');
    assert_eq!(input.cells(), ["5", "1", "", "", "", ""]);
    assert_eq!(input.cursor(), 2);
    let mut full = CodeInput::new();
    for ch in "518892".chars() {
        full.push(ch);
    }
    assert_eq!(full.cells(), ["5", "1", "8", "8", "9", "2"]);
    // 满了没有「正在输入的格」
    assert_eq!(full.cursor(), CODE_LEN);
}

#[test]
fn clear_resets_for_the_next_attempt() {
    let mut input = CodeInput::new();
    for ch in "123".chars() {
        input.push(ch);
    }
    input.clear();
    assert_eq!(input.digits(), "");
    assert!(!input.is_complete());
    assert_eq!(input.cells(), ["", "", "", "", "", ""]);
}
