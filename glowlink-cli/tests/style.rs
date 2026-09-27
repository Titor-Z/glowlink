//! `glowlink-cli/src/view/style.rs` 的单元测试。
//!
//! 重点：**只在 TTY 且没有 NO_COLOR 时上色**，且**颜色不携带语义**
//! （去掉颜色后信息必须一字不少，否则脚本 grep 出来的东西会随环境变化）。

use glowlink_cli::view::style::Style;
use glowlink_cli::view::Level;

/// 去掉所有 ANSI 转义序列，得到「纯文本视图」。
fn strip(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // 跳过 "\x1b[....m"
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[test]
fn color_only_when_tty_and_no_color_is_absent() {
    assert!(Style::from_flags(true, false).colored(), "TTY + 未设 NO_COLOR → 上色");
    assert!(!Style::from_flags(true, true).colored(), "设了 NO_COLOR → 不上色");
    assert!(!Style::from_flags(false, false).colored(), "非 TTY（管道/重定向/CI）→ 不上色");
    assert!(!Style::from_flags(false, true).colored());
    assert!(!Style::plain().colored());
}

#[test]
fn color_never_changes_the_text_itself() {
    let colored = Style::from_flags(true, false);
    let plain = Style::plain();

    for text in ["错误：文件不存在", "端口 9267 被占用", "已发送：报告.pdf"] {
        assert_eq!(strip(&colored.error(text)), text);
        assert_eq!(strip(&colored.warn(text)), text);
        assert_eq!(strip(&colored.ok(text)), text);
        assert_eq!(strip(&colored.info(text)), text);
        assert_eq!(strip(&colored.strong(text)), text);
        assert_eq!(strip(&colored.dim(text)), text);
        // 不上色时应当**逐字节等于原文**，而不是留下空转义序列
        assert_eq!(plain.error(text), text);
        assert_eq!(plain.ok(text), text);
    }
}

#[test]
fn level_prefix_survives_without_color() {
    // 前缀是信息的一部分，任何环境都必须有；颜色才是可选的
    let plain = Style::plain();
    assert_eq!(plain.labeled(Level::Info, "已自动顺延到 9268"), "[信息] 已自动顺延到 9268");
    assert_eq!(plain.labeled(Level::Warn, "对端未加密"), "[警告] 对端未加密");
    assert_eq!(plain.labeled(Level::Notice, "先用 glowlink list"), "[提示] 先用 glowlink list");
    // 普通信息不带前缀
    assert_eq!(plain.labeled(Level::Plain, "已发送"), "已发送");

    let colored = Style::from_flags(true, false);
    assert_eq!(strip(&colored.labeled(Level::Info, "x")), "[信息] x");
    assert_eq!(strip(&colored.labeled(Level::Plain, "x")), "x");
}

#[test]
fn labeled_never_doubles_an_existing_prefix() {
    // 提示语是在别处拼好的（核心库也会产出），补前缀时必须先看有没有
    let plain = Style::plain();
    let line = "[信息] 端口 9267 被残留占用";
    assert_eq!(plain.labeled(Level::Info, line), line, "不能变成「[信息] [信息] …」");
    assert_eq!(plain.labeled(Level::Warn, "[警告] 对端未加密"), "[警告] 对端未加密");
    // 只上色那一个完全不碰文本
    assert_eq!(plain.colorize(Level::Info, line), line);
    let colored = Style::from_flags(true, false);
    assert_eq!(strip(&colored.colorize(Level::Info, line)), line);
}

#[test]
fn level_is_inferred_from_an_existing_prefix() {
    assert_eq!(Level::from_prefix("[信息] 端口顺延"), Level::Info);
    assert_eq!(Level::from_prefix("[警告] 功能受限"), Level::Warn);
    assert_eq!(Level::from_prefix("[提示] 先执行 list"), Level::Notice);
    assert_eq!(Level::from_prefix("已发送：报告.pdf"), Level::Plain);
}

#[test]
fn empty_text_is_left_alone() {
    // 空串不该被包成 "\x1b[31m\x1b[0m"（会污染对齐与行宽计算）
    let colored = Style::from_flags(true, false);
    assert_eq!(colored.error(""), "");
    assert_eq!(colored.strong(""), "");
}
