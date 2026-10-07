//! Model：单行名字输入（配对改名的输入框）。
//!
//! 纯状态机：文本 + 光标（字符下标）+ 组文区间（IME 画布上的那截「标记文本」）。
//! 零 IO、零 GPUI，全部可单测；与平台 IME 的对接在 `view::app` 的
//! `EntityInputHandler` 实现里，这里只提供它要的每一步操作。
//!
//! 为什么必须正经做输入处理：改名的对象就是「titor-z的Mac mini」这种名字，
//! 只吃 ASCII 等于让中文用户没法用。gpui 的文本区间一律按 **UTF-16 下标**，
//! 而我们内部按字符存——两套换算在这里收口，配对码那种「不碰 IME」的省法
//! 在这里省不得（见文件底部的换算测试）。

/// 名字最长字符数（设计稿 `maxlength="20"`）。
pub const NAME_MAX: usize = 20;

/// 一份单行输入状态。光标只是一个字符下标（不做选区）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameField {
    text: String,
    /// 光标（**字符下标**，0..=字符数）。
    caret: usize,
    /// IME 组文区间（字符下标；`None` = 没有在组文）。
    marked: Option<(usize, usize)>,
}

impl NameField {
    pub fn new(initial: &str) -> Self {
        let mut field = NameField { text: String::new(), caret: 0, marked: None };
        field.set_text(initial);
        field
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn marked(&self) -> Option<(usize, usize)> {
        self.marked
    }

    pub fn is_composing(&self) -> bool {
        self.marked.is_some()
    }

    /// 重置成一段文本（光标落到末尾）——进入编辑态时用它装当前名字。
    pub fn set_text(&mut self, text: &str) {
        let clamped: String = text.chars().take(NAME_MAX).collect();
        self.caret = clamped.chars().count();
        self.text = clamped;
        self.marked = None;
    }

    pub fn clear(&mut self) {
        self.set_text("");
    }

    /// 在光标处插入文本（超长的部分**截断**，设计稿 maxlength 的语义）。
    /// 组文进行中不许走这条路：直接插入会让组文区间指错位置。
    pub fn insert(&mut self, text: &str) {
        self.marked = None;
        self.replace_range(self.caret, self.caret, text);
    }

    /// 退格：组文进行中删掉整段组文（IME 的惯例），否则删光标前一个字符。
    pub fn backspace(&mut self) {
        if let Some((start, end)) = self.marked.take() {
            self.replace_range(start, end, "");
            return;
        }
        if self.caret > 0 {
            self.replace_range(self.caret - 1, self.caret, "");
        }
    }

    /// 左右移动光标（组文进行中不动：方向键属于 IME，不会走到这里）。
    pub fn move_caret(&mut self, delta: isize) -> bool {
        if self.marked.is_some() {
            return false;
        }
        let len = self.text.chars().count() as isize;
        let next = (self.caret as isize + delta).clamp(0, len);
        if next as usize == self.caret {
            return false;
        }
        self.caret = next as usize;
        true
    }

    /// 输入协议的统一入口：替换一段（UTF-16 下标的）区间。
    ///
    /// `range = None` 表示「组文区间，没有组文就光标处」——平台在
    /// 普通输入与 IME 提交时都这样调。
    pub fn replace_utf16_range(&mut self, range_utf16: Option<std::ops::Range<usize>>, text: &str) {
        let (start, end) = match range_utf16 {
            Some(range) => (
                char_index_from_utf16(&self.text, range.start),
                char_index_from_utf16(&self.text, range.end),
            ),
            None => match self.marked {
                Some((start, end)) => (start, end),
                None => (self.caret, self.caret),
            },
        };
        self.replace_range(start, end, text);
    }

    /// 插入后把「刚插入的那段」标记为组文（IME 组合态的标准节奏：
    /// 每次拼音上屏都整段替换上一轮的组文）。
    pub fn replace_and_mark_utf16(
        &mut self,
        range_utf16: Option<std::ops::Range<usize>>,
        text: &str,
    ) {
        self.replace_utf16_range(range_utf16, text);
        let end = self.caret;
        let start = end - text.chars().count();
        self.marked = Some((start.max(0), end));
    }

    /// IME 提交（或用户按了别的键打断）：组文转正。
    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// 底层替换：`start..end`（字符下标）换成 `text`，光标落在插入文本之后。
    fn replace_range(&mut self, start: usize, end: usize, text: &str) {
        let chars: Vec<char> = self.text.chars().collect();
        let start = start.min(chars.len());
        let end = end.clamp(start, chars.len());
        let budget = NAME_MAX.saturating_sub(chars.len() - (end - start));
        let mut next: String = chars[..start].iter().collect();
        let inserted = text.chars().take(budget);
        let inserted_count = inserted.clone().count();
        next.extend(inserted);
        next.extend(chars[end..].iter());
        self.text = next;
        self.caret = start + inserted_count;
    }
}

/// UTF-16 下标 → 字符下标（越界按末端钳住）。
pub fn char_index_from_utf16(text: &str, pos_utf16: usize) -> usize {
    let mut seen_utf16 = 0usize;
    for (index, ch) in text.chars().enumerate() {
        if seen_utf16 >= pos_utf16 {
            return index;
        }
        seen_utf16 += ch.len_utf16();
    }
    text.chars().count()
}

/// 字符下标 → UTF-16 下标。
pub fn utf16_index_from_char(text: &str, pos_char: usize) -> usize {
    text.chars()
        .take(pos_char)
        .map(char::len_utf16)
        .sum()
}
