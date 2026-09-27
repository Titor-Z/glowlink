use glowlink_cli::view::cli::{human_duration, human_size, human_speed, session_banner, source_line};
use glowlink_cli::view::daemon::pending_hint;

#[test]
fn human_size_switches_units() {
    assert_eq!(human_size(0), "0 B");
    assert_eq!(human_size(999), "999 B");
    assert_eq!(human_size(1024), "1.0 KB");
    assert_eq!(human_size(1024 * 1024), "1.0 MB");
    assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
}

#[test]
fn human_speed_always_uses_megabytes() {
    assert_eq!(human_speed(1024.0 * 1024.0), "1.0 MB/s");
    assert_eq!(human_speed(0.0), "0.0 MB/s");
    // 千兆网线量级，避免出现 "0.1 GB/s" 这种不好比较的单位
    assert_eq!(human_speed(112.0 * 1024.0 * 1024.0), "112.0 MB/s");
}

#[test]
fn human_duration_picks_scale() {
    assert_eq!(human_duration(0), "0s");
    assert_eq!(human_duration(45), "45s");
    assert_eq!(human_duration(60), "1m00s");
    assert_eq!(human_duration(125), "2m05s");
    assert_eq!(human_duration(3600), "1h00m");
    assert_eq!(human_duration(3990), "1h06m");
}

/// 校验码**任何情况下**都要显示：本机信任对方 ≠ 对方信任本机。
///
/// 曾经的毛病：发送方在「已信任」时把校验码藏了起来，而接收方正因为不认识发送方
/// 要求核对——两边一对，接收方屏幕上那句「请与发送方屏幕核对」成了空话。
#[test]
fn session_banner_always_shows_the_code() {
    for trusted in [false, true] {
        let banner = session_banner("192.168.1.9:5000", "abcdef0123456789", "482913", trusted);
        assert!(banner.contains("482913"), "已信任={trusted} 时也必须给出校验码：{banner}");
        assert!(
            banner.contains("请与接收方屏幕上显示的核对"),
            "要告诉用户拿这个码去核对：{banner}"
        );
    }
    // 已信任时补一句说明，免得用户以为还要再核对一次
    assert!(session_banner("x", "fp", "482913", true).contains("已信任"));
    assert!(!session_banner("x", "fp", "482913", false).contains("已信任"));
}

#[test]
fn source_line_always_shows_the_code() {
    for trusted in [false, true] {
        let line = source_line("abcdef0123456789", "482913", trusted);
        assert!(line.contains("482913"), "已信任={trusted} 时也必须给出校验码：{line}");
        assert!(line.contains("abcdef0123456789"), "要能看出是谁：{line}");
    }
}

#[test]
fn pending_hint_always_shows_the_code() {
    for trusted in [false, true] {
        let hint = pending_hint("abcdef0123456789", "482913", trusted);
        assert!(hint.contains("482913"), "已信任={trusted} 时也必须给出校验码：{hint}");
    }
    // 已信任时要保留「免确认怎么开」的提示，别把原来的信息挤掉
    assert!(pending_hint("fp", "code", true).contains("--yes"));
    // 没有指纹（老版本对端）就不硬凑
    assert_eq!(pending_hint("", "482913", false), "");
}
