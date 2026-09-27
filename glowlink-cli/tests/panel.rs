//! `glowlink-cli/src/view/panel.rs` 的单元测试。
//!
//! 面板与帮助文本的长度是被钉住的：面板是「第一屏」，帮助是「完整用法」，
//! 一旦有人往里堆东西，测试会先叫。

use std::path::Path;

use glowlink_cli::view::panel::{help_text, status_panel, DaemonStatus};

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

#[test]
fn panel_shows_stopped_state_and_top_commands() {
    let panel = status_panel("0.1.4", "甲机", "5ad4e682", None);

    assert!(panel.starts_with("萤光 GlowLink 0.1.4"), "实际：{panel}");
    assert!(panel.contains("本机：甲机（5ad4e682）"), "要能认出是哪台机器");
    assert!(panel.contains("状态：未启动"));
    // 没启动时最该推荐的就是 on
    assert!(panel.contains("glowlink on"), "应引导用户启动后台服务");
    assert!(panel.contains("glowlink --help"), "完整用法要指路");
    // 不启动时不该出现 accept（还没有待确认的东西）
    assert!(!panel.contains("accept"), "未启动时不该提示确认接收");
}

#[test]
fn panel_reports_running_daemon_and_pending() {
    let dir = Path::new("./received");
    let panel = status_panel(
        "0.1.4",
        "甲机",
        "5ad4e682",
        Some(&DaemonStatus { pid: 8990, receive_dir: dir, pending: Some(2) }),
    );

    assert!(panel.contains("状态：运行中"), "实际：{panel}");
    assert!(panel.contains("PID 8990"));
    assert!(panel.contains("./received"));
    assert!(panel.contains("待确认 2"), "有待确认时要一眼看到数量");
    // 有待确认时，第一件事是去看列表并确认
    assert!(panel.contains("glowlink accept <编号>"), "应直接给出确认命令");
}

#[test]
fn panel_hides_pending_count_when_unknown() {
    let dir = Path::new("/tmp/x");
    let panel = status_panel(
        "0.1.4",
        "甲机",
        "5ad4e682",
        Some(&DaemonStatus { pid: 1, receive_dir: dir, pending: None }),
    );
    // 查不到待确认数（IPC 不通）也不该报错，更不该瞎写个 0
    assert!(panel.contains("状态：运行中（PID 1，接收目录 /tmp/x）"), "实际：{panel}");
    assert!(!panel.contains("待确认"));
}

#[test]
fn panel_stays_within_first_screen() {
    let dir = Path::new("./received");
    for daemon in [
        None,
        Some(DaemonStatus { pid: 1, receive_dir: dir, pending: Some(0) }),
        Some(DaemonStatus { pid: 1, receive_dir: dir, pending: Some(7) }),
    ] {
        let panel = status_panel("0.1.4", "甲机", "5ad4e682", daemon.as_ref());
        assert!(
            lines(&panel).len() <= 15,
            "默认输出是「第一屏」，不该超过 15 行：\n{panel}"
        );
    }
}

#[test]
fn help_is_grouped_into_four_sections() {
    let help = help_text("0.1.4", 9267);

    for section in ["后台服务：", "临时操作", "安全与信任：", "配置："] {
        assert!(help.contains(section), "帮助缺少「{section}」这一节：\n{help}");
    }
    // 每个命令都要出现，别漏
    for command in ["on", "off", "accept", "reject", "list", "send", "serve", "trust", "config"] {
        assert!(help.contains(&format!(" {command}")), "帮助里没有 {command}");
    }
    assert!(help.contains("9267"), "要写明默认发现端口");
    assert!(help.contains("--help"));
}

#[test]
fn help_explains_that_yes_only_covers_trusted_devices() {
    let help = help_text("0.1.4", 9267);
    // 这句是安全模型的关键，不能省：--yes 不等于谁都收
    assert!(
        help.contains("仅对已信任设备") || help.contains("仅对**已信任设备**"),
        "帮助必须说清 --yes 的边界：\n{help}"
    );
    assert!(help.contains("6 位校验码"), "帮助要说清首次配对怎么核对");
}

#[test]
fn help_stays_readable() {
    let help = help_text("0.1.4", 9267);
    assert!(lines(&help).len() <= 45, "完整帮助不该超过 45 行");
    // 行宽别超出常见终端，否则一换行就散架
    let widest = lines(&help).iter().map(|l| l.chars().count()).max().unwrap_or(0);
    assert!(widest <= 90, "最宽一行 {widest} 列，太宽了");
}
