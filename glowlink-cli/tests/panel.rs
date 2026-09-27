//! `glowlink-cli/src/view/panel.rs` 的单元测试。
//!
//! 面板与帮助文本的长度是被钉住的：面板是「第一屏」，帮助是「完整用法」，
//! 一旦有人往里堆东西，测试会先叫。

use std::path::Path;

use glowlink_cli::view::panel::{
    command_help, find_command, help_text, status_panel, DaemonStatus, Section, COMMANDS,
};
use glowlink_cli::view::Style;

/// 测试里一律用「不上色」的样式：断言的是信息本身，不是转义序列。
fn plain() -> Style {
    Style::plain()
}

/// 去掉 ANSI 转义序列，用来验证「颜色不改变文本」。
fn strip(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            // 跳过 CSI 直到终止字母
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

#[test]
fn panel_shows_stopped_state_and_top_commands() {
    let panel = status_panel(&plain(), "0.1.4", "甲机", "5ad4e682", None);

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
        &plain(),
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
        &plain(),
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
        let panel = status_panel(&plain(), "0.1.4", "甲机", "5ad4e682", daemon.as_ref());
        assert!(
            lines(&panel).len() <= 15,
            "默认输出是「第一屏」，不该超过 15 行：\n{panel}"
        );
    }
}

#[test]
fn help_is_grouped_into_four_sections() {
    let help = help_text(&plain(), "0.1.4");

    // 分节标题直接用同一份来源，避免测试里再抄一遍标题
    for section in Section::ALL {
        assert!(
            help.contains(section.title()),
            "帮助缺少「{}」这一节：\n{help}",
            section.title()
        );
    }
    // 每个命令都要出现，别漏
    for command in ["on", "off", "accept", "reject", "list", "send", "serve", "trust", "config"] {
        assert!(help.contains(&format!(" {command}")), "帮助里没有 {command}");
    }
    // 端口这类细节挪到了 help list，不再堆在总览里
    assert!(
        command_help(&plain(), "list").unwrap().contains("9267"),
        "发现端口的说明应当在 help list 里"
    );
    assert!(help.contains("--help"));
}

#[test]
fn help_explains_that_yes_only_covers_trusted_devices() {
    let help = help_text(&plain(), "0.1.4");
    // 这句是安全模型的关键，不能省：--yes 不等于谁都收
    assert!(
        help.contains("仅对已信任设备") || help.contains("仅对**已信任设备**"),
        "帮助必须说清 --yes 的边界：\n{help}"
    );
    assert!(help.contains("6 位校验码"), "帮助要说清首次配对怎么核对");
}

#[test]
fn help_stays_readable() {
    let help = help_text(&plain(), "0.1.4");
    assert!(
        lines(&help).len() <= 32,
        "总览不该超过 32 行（它是索引，不是手册）：\n{help}"
    );
    // 行宽别超出常见终端，否则一换行就散架
    let widest = lines(&help).iter().map(|l| l.chars().count()).max().unwrap_or(0);
    assert!(widest <= 90, "最宽一行 {widest} 列，太宽了");
}

/// 总览必须列出表里的**每一条**命令：新增命令忘了写总览，这条会先红。
#[test]
fn overview_lists_every_command_in_the_table() {
    let help = help_text(&plain(), "0.1.4");
    for command in COMMANDS {
        assert!(
            help.contains(&format!("  {}  ", command.name)),
            "总览里没有 {} 这一行：\n{help}",
            command.name
        );
        assert!(
            help.contains(command.summary),
            "总览里没有 {} 的摘要：\n{help}",
            command.name
        );
    }
    // 表里的名字不能重复（find_command 只返回第一个）
    let mut names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
    names.sort_unstable();
    let unique = names.len();
    names.dedup();
    assert_eq!(names.len(), unique, "命令名有重复");
}

/// 每一个命令都要有详细说明，且该说到的都说到了。
#[test]
fn every_command_has_a_usable_help_page() {
    for command in COMMANDS {
        let text = command_help(&plain(), command.name).unwrap_or_else(|| panic!("{} 没有帮助页", command.name));
        assert!(text.starts_with(&format!("glowlink {} — {}", command.name, command.summary)));
        assert!(text.contains("用法："), "{} 的帮助缺用法", command.name);
        for usage in command.usages {
            assert!(text.contains(usage), "{} 的帮助缺用法行「{usage}」", command.name);
            assert!(usage.starts_with("glowlink "), "用法行要以 glowlink 开头：{usage}");
        }
        for example in command.examples {
            assert!(text.contains(example), "{} 的帮助缺示例「{example}」", command.name);
        }
        let count = text.lines().count();
        assert!(count <= 30, "{} 的帮助 {count} 行，太长了", command.name);
        let widest = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
        assert!(widest <= 90, "{} 的帮助有 {widest} 列宽的行", command.name);
        // 每条命令都要指路总览，用户才知道还能看什么
        assert!(text.contains("glowlink --help"), "{} 的帮助没指路总览", command.name);
    }
}

#[test]
fn unknown_command_has_no_help_page() {
    assert!(command_help(&plain(), "nope").is_none());
    assert!(find_command("nope").is_none());
    assert!(find_command("send").is_some());
}

/// 表与真正的命令分派必须对得上：真跑二进制问一遍。
///
/// 只看表不看分派会漏掉「表里写了、分派里拼错」这种漂移——那种情况下
/// 帮助说得好好的，敲下去却是「未知命令」。
#[test]
fn the_binary_accepts_every_command_in_the_table() {
    let bin = env!("CARGO_BIN_EXE_glowlink");
    for command in COMMANDS {
        let output = std::process::Command::new(bin)
            .args(["help", command.name])
            .output()
            .expect("应当能启动 glowlink");
        assert!(
            output.status.success(),
            "`glowlink help {}` 失败：{}",
            command.name,
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(command.summary),
            "`glowlink help {}` 没打出这条命令的说明：{stdout}",
            command.name
        );
    }
}

/// 未知命令必须是用法错误（退出码 2），并指路总览。
#[test]
fn the_binary_rejects_unknown_commands_with_usage_exit_code() {
    let bin = env!("CARGO_BIN_EXE_glowlink");
    let output = std::process::Command::new(bin)
        .args(["help", "nope"])
        .output()
        .expect("应当能启动 glowlink");
    assert_eq!(output.status.code(), Some(2), "用法错误应当是退出码 2");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("glowlink --help"), "要指路总览：{stderr}");
}

/// 颜色只是装饰：去掉转义序列后必须与纯文本版本**逐字节相同**。
#[test]
fn colors_never_change_the_text() {
    let colored = Style::from_flags(true, false);
    assert_ne!(colored.strong("x"), "x", "前提：这个样式确实会上色");
    for (with, without) in [
        (help_text(&colored, "0.1.4"), help_text(&plain(), "0.1.4")),
        (
            command_help(&colored, "send").unwrap(),
            command_help(&plain(), "send").unwrap(),
        ),
        (
            status_panel(&colored, "0.1.4", "甲机", "5ad4e682", None),
            status_panel(&plain(), "0.1.4", "甲机", "5ad4e682", None),
        ),
    ] {
        assert_eq!(strip(&with), without, "去掉颜色后文本必须一字不差");
    }
}

/// TTY 下总览要真有色：否则「支持彩色」只是文档上的一句空话。
#[test]
fn help_is_colored_in_a_terminal() {
    let text = help_text(&Style::from_flags(true, false), "0.1.4");
    assert!(text.contains("\u{1b}["), "TTY 下应当出现转义序列");
    // 小节标题与命令名都要上色（标题行、四个小节、每条命令）
    for section in Section::ALL {
        let heading = section.title();
        assert!(
            text.contains(&format!("\u{1b}[36;1m{heading}")),
            "「{heading}」应当是彩色小节标题"
        );
    }
    assert!(text.contains("\u{1b}[1mon"), "命令名应当加粗");
    // 非 TTY：一个转义序列都不许有
    assert!(!help_text(&plain(), "0.1.4").contains('\u{1b}'));
}
