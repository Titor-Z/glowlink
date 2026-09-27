//! 状态面板与帮助文本（纯字符串生成，便于单测）。
//!
//! 三段文本的定位不同，长度都被测试钉住：
//! - **[`status_panel`]**：敲 `glowlink` 回车看到的「第一屏」，只回答
//!   「现在是什么状态 + 我最常做的三件事」，**不超过 15 行**；
//! - **[`help_text`]**：`glowlink --help` 的四节总览，**不超过 32 行**，
//!   且**只回答「有哪些命令、各自干什么」**——选项、示例、端口细节都放单命令帮助，
//!   总览里堆选项会变成一堵墙，反而没人读；
//! - **[`command_help`]**：`glowlink help <命令>` / `glowlink <命令> --help` 的
//!   单命令说明，**不超过 30 行**。
//!
//! 命令的摘要、用法、参数、示例全部来自 [`COMMANDS`] 这一张表：
//! 总览与单命令帮助都从它生成，改一处不会漏另一处。
//!
//! 这里不打印、不查状态，只把内容拼成字符串——状态由调用方查好后传进来；
//! 着色也由调用方把 [`Style`] 传进来（依赖注入），因此**测试里用 `Style::plain()`
//! 就能断言纯文本**，不必去猜当前是不是终端。

use std::path::Path;

use crate::view::style::Style;

/// 守护进程状态摘要（由调用方查询后传入）。
pub struct DaemonStatus<'a> {
    pub pid: u32,
    pub receive_dir: &'a Path,
    /// 待确认的接收请求数；查不到时为 `None`。
    pub pending: Option<usize>,
}

/// 帮助里的分区；顺序即打印顺序。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// 后台服务：常驻进程相关。
    Service,
    /// 临时操作：用完即走，不需要后台服务。
    Adhoc,
    /// 安全与信任。
    Security,
    /// 配置与帮助。
    Config,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Service => "后台服务：",
            Section::Adhoc => "临时操作（不需要后台服务）：",
            Section::Security => "安全与信任：",
            Section::Config => "配置与帮助：",
        }
    }

    /// 全部小节，按打印顺序。
    pub const ALL: [Section; 4] =
        [Section::Service, Section::Adhoc, Section::Security, Section::Config];
}

/// 一条命令的全部说明。
pub struct Command {
    /// 命令名（用户敲的那个词）。
    pub name: &'static str,
    /// 属于哪一节。
    pub section: Section,
    /// 一行摘要（总览里用）。
    pub summary: &'static str,
    /// 用法行（单命令帮助里逐行列出）。
    pub usages: &'static [&'static str],
    /// 参数与开关说明（单命令帮助里逐行列出）。
    pub details: &'static [&'static str],
    /// 示例。
    pub examples: &'static [&'static str],
}

/// 全部命令。新增命令只改这里：总览、单命令帮助、测试都会跟着走。
pub const COMMANDS: &[Command] = &[
    Command {
        name: "on",
        section: Section::Service,
        summary: "在后台启动常驻服务（推荐）",
        usages: &["glowlink on [--dir 目录] [--yes] [--ipc-port 端口]"],
        details: &[
            "--dir 目录        收到的文件存哪（默认 ./received）",
            "--yes             仅对**已信任设备**免确认自动接收；陌生设备仍要人工确认",
            "--ipc-port 端口   本机命令行与后台服务通信的端口（默认自动顺延）",
            "重复执行是幂等的：已经在跑就直接返回。",
        ],
        examples: &["glowlink on --dir ~/Downloads", "glowlink on --yes   # 只对已配对设备免确认"],
    },
    Command {
        name: "off",
        section: Section::Service,
        summary: "停止后台服务",
        usages: &["glowlink off"],
        details: &["会通知后台服务收尾退出，并清理状态文件。"],
        examples: &["glowlink off"],
    },
    Command {
        name: "accept",
        section: Section::Service,
        summary: "确认接收待确认的传输",
        usages: &["glowlink accept [编号]"],
        details: &[
            "编号见 glowlink list；只有一条待确认时可以省略编号。",
            "首次与陌生设备互传，先与对方屏幕核对 6 位校验码再确认。",
            "确认后对方指纹进入信任列表（~/.glowlink/trusted.txt），下次不再核对。",
        ],
        examples: &["glowlink accept 001"],
    },
    Command {
        name: "reject",
        section: Section::Service,
        summary: "拒绝待确认的传输",
        usages: &["glowlink reject [编号]"],
        details: &[
            "对方会收到「传输被拒绝」；不会写入信任列表。",
            "不理会也会在超时后自动拒绝（默认 300 秒，见 pending_timeout_secs）。",
        ],
        examples: &["glowlink reject 001"],
    },
    Command {
        name: "ui",
        section: Section::Adhoc,
        summary: "交互式界面（默认命令，仅终端）",
        usages: &["glowlink ui [--discovery-port 端口]"],
        details: &[
            "↑↓ 选择 · 回车（设备：输入文件路径；待确认：接受）· d 拒绝 · r 刷新 · q 退出",
            "可直接把文件拖进终端，路径的引号与转义会自动还原。",
            "接受/拒绝通过后台服务执行，所以退出界面后它照常工作。",
        ],
        examples: &["glowlink            # 终端里直接就是它", "glowlink ui"],
    },
    Command {
        name: "list",
        section: Section::Adhoc,
        summary: "扫描附近设备",
        usages: &[
            "glowlink list [--watch] [--timeout 秒] [--discovery-port 端口]",
            "glowlink list [--discovery-port-span 个数]",
        ],
        details: &[
            "--watch                   原地刷新，Ctrl-C 退出（需要终端）",
            "--timeout 秒              扫描时长（默认 3 秒）",
            "--discovery-port 端口     发现端口（默认 9267）",
            "--discovery-port-span 个数  顺延跨度，同时也是查询覆盖的端口数（默认 21）",
            "后台服务在跑时结果直接来自它，不会再抢一次发现端口。",
        ],
        examples: &["glowlink list --watch"],
    },
    Command {
        name: "send",
        section: Section::Adhoc,
        summary: "发送文件给指定设备",
        usages: &[
            "glowlink send <设备名|设备ID> <文件>…",
            "glowlink send --peer ip:端口 <文件>…",
        ],
        details: &[
            "--peer ip:端口    直连发送，绕过发现层（跨网段或调试用）",
            "可一次发多个文件；传输进度显示百分比、速度与预计剩余时间。",
            "首次与陌生设备互传时，两端会显示同一个 6 位校验码，核对一致即可。",
        ],
        examples: &[
            "glowlink send 客厅的Mac 照片.jpg 报告.pdf",
            "glowlink send --peer 192.168.1.23:52878 文件.zip",
        ],
    },
    Command {
        name: "serve",
        section: Section::Adhoc,
        summary: "前台常驻接收（调试用）",
        usages: &["glowlink serve [--dir 目录] [--yes] [--port 端口]"],
        details: &[
            "--port 端口   固定 TCP 传输端口（默认由系统分配）",
            "与 on 的区别：不脱离终端、不写状态文件，适合临时收一个文件。",
        ],
        examples: &["glowlink serve --dir ./received"],
    },
    Command {
        name: "trust",
        section: Section::Security,
        summary: "查看或取消已信任设备",
        usages: &["glowlink trust", "glowlink trust forget <名称|指纹>"],
        details: &[
            "已信任设备不再核对校验码；配合 glowlink on --yes 才能免确认自动接收。",
            "打印本机设备指纹，可与对方核对。",
            "信任列表在 ~/.glowlink/trusted.txt，可以直接编辑。",
        ],
        examples: &["glowlink trust forget 甲机"],
    },
    Command {
        name: "config",
        section: Section::Config,
        summary: "显示生效配置与配置文件位置",
        usages: &["glowlink config"],
        details: &[
            "优先级：内置默认 < ./glowlink.toml < ~/.config/glowlink/config.toml < 命令行开关。",
            "配置文件是扁平的 key = value；写错键名会直接报错并指出行号。",
        ],
        examples: &["glowlink config"],
    },
    Command {
        name: "help",
        section: Section::Config,
        summary: "查看命令用法",
        usages: &["glowlink help [命令]", "glowlink <命令> --help"],
        details: &["不给命令名时打印四节总览。"],
        examples: &["glowlink help send"],
    },
];

/// 按名字找命令。
pub fn find_command(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.name == name)
}

/// 敲 `glowlink` 回车看到的默认输出。
pub fn status_panel(
    style: &Style,
    version: &str,
    name: &str,
    short_id: &str,
    daemon: Option<&DaemonStatus<'_>>,
) -> String {
    let mut lines = vec![
        style.heading(&format!("萤光 GlowLink {version} — 局域网近场文件传输（已加密）")),
        String::new(),
        format!("本机：{name}（{short_id}）"),
    ];

    match daemon {
        None => lines.push("状态：未启动".to_string()),
        Some(status) => {
            let mut state = format!(
                "状态：运行中（PID {}，接收目录 {}",
                status.pid,
                status.receive_dir.display()
            );
            match status.pending {
                Some(0) | None => state.push('）'),
                Some(count) => state.push_str(&format!("，待确认 {count}）")),
            }
            lines.push(state);
        }
    }

    lines.push(String::new());
    lines.push("常用命令：".to_string());
    if let Some(status) = daemon {
        if matches!(status.pending, Some(count) if count > 0) {
            lines.push("  glowlink list                   查看设备与待确认的传输".to_string());
            lines.push("  glowlink accept <编号>          确认接收".to_string());
        } else {
            lines.push("  glowlink list                   查看附近设备".to_string());
            lines.push("  glowlink send <设备> <文件…>    发送文件".to_string());
        }
    } else {
        lines.push("  glowlink on                     在后台启动服务（推荐）".to_string());
        lines.push("  glowlink list                   扫描附近设备".to_string());
        lines.push("  glowlink send <设备> <文件…>    发送文件".to_string());
    }
    lines.push("  glowlink --help                 完整用法".to_string());
    lines.join("\n")
}

/// `glowlink --help` 的四节总览（由 [`COMMANDS`] 生成，不另写一份）。
///
/// 总览只回答「有哪些命令、各自干什么」：选项与示例交给 `glowlink help <命令>`。
/// 把 `--dir` / `--watch` 这些堆进总览会让它变成一堵墙，反而没人读（git / docker 也不这么干）。
pub fn help_text(style: &Style, version: &str) -> String {
    let mut out = format!(
        "{}\n\n用法：glowlink [命令] [选项]    每条命令的详细说明：{}（也可 {}）\n",
        style.heading(&format!("萤光 GlowLink {version} — 局域网近场文件传输（已加密）")),
        style.strong("glowlink help <命令>"),
        style.strong("glowlink <命令> --help"),
    );

    // 命令名对齐到最长的那一个，读起来是一列而不是锯齿
    let width = COMMANDS.iter().map(|c| c.name.len()).max().unwrap_or(0);

    for section in Section::ALL {
        out.push('\n');
        out.push_str(&style.heading(section.title()));
        out.push('\n');
        for command in COMMANDS.iter().filter(|c| c.section == section) {
            out.push_str(&format!(
                "  {}  {}\n",
                style.strong(&format!("{:<width$}", command.name)),
                command.summary
            ));
        }
    }

    out.push_str(&format!(
        "\n{}首次与陌生设备互传时，两端显示同一个 6 位校验码，核对一致后再确认。\n\
         --yes 仅对已信任设备免确认，陌生设备一律人工确认。\n\
         退出码：0 成功 · 1 通用错误 · 2 用法或配置 · 3 网络不可达 · 4 加密或信任 · 5 被拒绝或超时\n",
        style.dim("安全："),
    ));
    out
}

/// 单个命令的详细说明；命令名不认识时返回 `None`。
pub fn command_help(style: &Style, name: &str) -> Option<String> {
    let command = find_command(name)?;
    let mut out = format!(
        "{} — {}\n\n{}\n",
        style.heading(&format!("glowlink {}", command.name)),
        command.summary,
        style.heading("用法：")
    );
    for usage in command.usages {
        out.push_str(&format!("  {}\n", style.strong(usage)));
    }
    if !command.details.is_empty() {
        out.push_str(&format!("\n{}\n", style.heading("说明：")));
        for detail in command.details {
            out.push_str(&format!("  {detail}\n"));
        }
    }
    if !command.examples.is_empty() {
        out.push_str(&format!("\n{}\n", style.heading("示例：")));
        for example in command.examples {
            out.push_str(&format!("  {}\n", style.dim(example)));
        }
    }
    out.push_str(&format!(
        "\n另见：{}（全部命令）\n",
        style.strong("glowlink --help")
    ));
    Some(out)
}
