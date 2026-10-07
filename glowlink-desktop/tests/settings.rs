//! `model/settings.rs` 的测试：桌面设置（设置面板背后的那份数据）。
//!
//! 全是纯解析与取值，所以能单测。这里的重点是**不静默**：
//! 未知键、非法值、缺「=」都要报出来——静默忽略会让用户以为改了却没生效。

use glowlink_desktop::model::settings::{apply_line, parse, serialize, Settings, ThemeChoice};

#[test]
fn defaults_match_design() {
    let settings = Settings::default();
    assert_eq!(settings.theme, ThemeChoice::System);
    // 空态显示本机名字：用户拍板「暂时显示」（设置页留开关）
    assert!(settings.self_name_in_empty);
    // 空态结论报完就收起（常驻是可选项，不是默认）
    assert!(!settings.note_persistent);
    assert_eq!(settings.receive_dir, None);
}

#[test]
fn parses_all_keys() {
    let text = "theme = dark\nself_name_in_empty = false\nnote_persistent = true\n";
    let (settings, problems) = parse(text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(settings.theme, ThemeChoice::Dark);
    assert!(!settings.self_name_in_empty);
    assert!(settings.note_persistent);
}

#[test]
fn quotes_and_comments_are_stripped() {
    let (settings, problems) = parse("# 注释\ntheme = \"light\" # 行尾注释\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(settings.theme, ThemeChoice::Light);
}

#[test]
fn unknown_key_is_reported_not_swallowed() {
    let (settings, problems) = parse("themes = dark\n");
    assert_eq!(problems.len(), 1, "不认识的键必须报出来");
    assert_eq!(settings, Settings::default());
}

#[test]
fn bad_value_is_reported_and_key_keeps_default() {
    let (settings, problems) = parse("theme = blue\n");
    assert_eq!(problems.len(), 1);
    assert_eq!(settings.theme, ThemeChoice::System);
}

#[test]
fn apply_line_rejects_bad_bool() {
    let mut settings = Settings::default();
    assert!(apply_line(&mut settings, "note_persistent", "yes").is_err());
    assert!(!settings.note_persistent, "坏值不许改掉默认");
}

#[test]
fn missing_line_without_equals_is_reported() {
    let (_, problems) = parse("theme dark\n");
    assert_eq!(problems.len(), 1);
    assert!(problems[0].contains("第 1 行"));
}

#[test]
fn roundtrip_preserves_every_field() {
    let mut settings = Settings::default();
    settings.theme = ThemeChoice::Light;
    settings.self_name_in_empty = false;
    settings.note_persistent = true;
    settings.receive_dir = Some("/tmp/收到 的文件".into());
    let (parsed, problems) = parse(&serialize(&settings));
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(parsed, settings);
}

#[test]
fn empty_receive_dir_clears_override() {
    let (settings, problems) = parse("receive_dir = \"\"\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(settings.receive_dir, None);
}

#[test]
fn theme_tokens_roundtrip() {
    for choice in [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light] {
        assert_eq!(ThemeChoice::from_token(choice.as_token()), Some(choice));
    }
    assert_eq!(ThemeChoice::from_token("blue"), None);
}

#[test]
fn theme_index_matches_seg_order() {
    // 分段控件的滑块位置按这个档位算，顺序错一格就会停在隔壁
    assert_eq!(ThemeChoice::System.index(), 0);
    assert_eq!(ThemeChoice::Dark.index(), 1);
    assert_eq!(ThemeChoice::Light.index(), 2);
}
