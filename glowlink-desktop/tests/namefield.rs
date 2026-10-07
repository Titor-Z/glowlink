//! `model/namefield.rs` 的测试：单行名字输入（配对改名）。
//!
//! 重点三类：基本编辑（插入 / 退格 / 光标）、**UTF-16 换算**（中文与 emoji 的
//! 下标不是一回事，IME 给的是 UTF-16）、长度上限（设计稿 maxlength 20）。

use glowlink_desktop::model::namefield::{
    char_index_from_utf16, utf16_index_from_char, NameField, NAME_MAX,
};

#[test]
fn starts_with_initial_and_caret_at_end() {
    let field = NameField::new("Pixel 9");
    assert_eq!(field.text(), "Pixel 9");
    assert_eq!(field.caret(), 7);
    assert!(!field.is_composing());
}

#[test]
fn insert_goes_to_caret_and_moves_it() {
    let mut field = NameField::new("Pixel");
    field.move_caret(-5);
    field.insert("新的 ");
    assert_eq!(field.text(), "新的 Pixel");
    assert_eq!(field.caret(), 3);
}

#[test]
fn backspace_deletes_before_caret() {
    let mut field = NameField::new("Mac mini");
    field.backspace();
    assert_eq!(field.text(), "Mac min");
    assert_eq!(field.caret(), 7);
    // 空文本上退格不动
    field.clear();
    field.backspace();
    assert_eq!(field.text(), "");
}

#[test]
fn backspace_during_composition_clears_the_marked_text() {
    // IME 组合态里按删除：删的是整段组文，不是组文前的字符
    let mut field = NameField::new("我的 ");
    field.replace_and_mark_utf16(None, "macmini");
    assert!(field.is_composing());
    field.backspace();
    assert_eq!(field.text(), "我的 ");
    assert!(!field.is_composing());
}

#[test]
fn composition_replaces_previous_marked_text_each_round() {
    // 拼音一节一节上屏：每轮整段替换上一轮的组文
    let mut field = NameField::new("");
    field.replace_and_mark_utf16(None, "mac");
    field.replace_and_mark_utf16(None, "macmini");
    assert_eq!(field.text(), "macmini");
    field.unmark();
    assert_eq!(field.text(), "macmini");
}

#[test]
fn max_length_truncates_what_does_not_fit() {
    let mut field = NameField::new(&"字".repeat(NAME_MAX));
    field.insert("多出来的");
    assert_eq!(field.text().chars().count(), NAME_MAX, "只截断放不下的部分");
    assert_eq!(field.text(), "字".repeat(NAME_MAX));
}

#[test]
fn initial_text_is_clamped_too() {
    let field = NameField::new(&"名".repeat(NAME_MAX + 5));
    assert_eq!(field.text().chars().count(), NAME_MAX);
}

#[test]
fn caret_moves_within_bounds() {
    let mut field = NameField::new("ab");
    assert!(field.move_caret(-99), "从末尾往左是有效的移动");
    assert_eq!(field.caret(), 0);
    assert!(!field.move_caret(-1), "已经顶到头，再往左不该有动作");
    assert!(field.move_caret(99), "从开头往右是有效的移动");
    assert_eq!(field.caret(), 2);
    assert!(!field.move_caret(99), "已经到末尾，再往右不该有动作");
    assert!(field.move_caret(-1));
    assert_eq!(field.caret(), 1);
}

#[test]
fn arrows_do_nothing_while_composing() {
    // 组文期间方向键属于 IME：状态机不去动光标
    let mut field = NameField::new("ab");
    field.replace_and_mark_utf16(None, "拼音");
    assert!(!field.move_caret(-1));
    assert_eq!(field.caret(), 4);
}

#[test]
fn utf16_range_replace_works_for_cjk() {
    // IME 提交带的是 UTF-16 区间：中文每字 1 个 UTF-16 单元
    let mut field = NameField::new("甲机");
    field.replace_utf16_range(Some(0..1), "乙");
    assert_eq!(field.text(), "乙机");
}

#[test]
fn utf16_index_roundtrips_through_emoji() {
    // emoji 占 2 个 UTF-16 单元、1 个字符：两套下标必须各转各的
    let text = "📱甲机";
    assert_eq!(utf16_index_from_char(text, 1), 2);
    assert_eq!(utf16_index_from_char(text, 2), 3);
    assert_eq!(char_index_from_utf16(text, 2), 1);
    assert_eq!(char_index_from_utf16(text, 3), 2);
    // 越界钳到末端，不 panic
    assert_eq!(char_index_from_utf16(text, 999), 3);
    assert_eq!(utf16_index_from_char(text, 999), 4);
}

#[test]
fn replace_utf16_range_inside_emoji_pair_is_clamped() {
    // 平台如果给出发到代理对中间的下标，按包含它的字符边界处理，不许崩
    let mut field = NameField::new("📱机");
    field.replace_utf16_range(Some(1..2), "x");
    assert!(!field.text().is_empty());
}
