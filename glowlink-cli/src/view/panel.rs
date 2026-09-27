//! 状态面板与帮助文本（纯字符串生成，便于单测）。
//!
//! 两段文本的定位不同，长度也被测试钉住：
//! - **[`status_panel`]**：敲 `glowlink` 回车看到的「第一屏」，只回答
//!   「现在是什么状态 + 我最常做的三件事」，**不超过 15 行**；
//! - **[`help_text`]**：`glowlink --help` 的完整用法，按功能分四节，
//!   **不超过 45 行**。
//!
//! 这里不打印、不查状态，只把内容拼成字符串——状态由调用方查好后传进来。

use std::path::Path;

/// 守护进程状态摘要（由调用方查询后传入）。
pub struct DaemonStatus<'a> {
    pub pid: u32,
    pub receive_dir: &'a Path,
    /// 待确认的接收请求数；查不到时为 `None`。
    pub pending: Option<usize>,
}

/// 敲 `glowlink` 回车看到的默认输出。
pub fn status_panel(
    version: &str,
    name: &str,
    short_id: &str,
    daemon: Option<&DaemonStatus<'_>>,
) -> String {
    let mut lines = vec![
        format!("萤光 GlowLink {version}"),
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

/// `glowlink --help` 的完整用法，按功能分节。
pub fn help_text(version: &str, discovery_port: u16) -> String {
    format!(
        "萤光 GlowLink {version} — 局域网近场文件传输（传输已加密）\n\
         \n\
         用法：glowlink [命令] [选项]\n\
         \n\
         后台服务：\n\
         \x20 on [--dir 目录] [--yes] [--ipc-port 端口]\n\
         \x20     在后台启动常驻服务。收到文件时会等待人工确认，编号见 glowlink list。\n\
         \x20     --yes：仅对**已信任设备**免确认自动接收，陌生设备仍要人工确认。\n\
         \x20 off\n\
         \x20     停止后台服务。\n\
         \x20 accept [编号] / reject [编号]\n\
         \x20     确认或拒绝待接收的传输（有多个待确认时须给编号）。\n\
         \n\
         临时操作（不需要后台服务）：\n\
         \x20 ui                               交互式设备列表（默认命令，仅终端）\n\
         \x20 list [--watch] [--timeout 秒] [--discovery-port 端口]\n\
         \x20     扫描附近设备；--watch 原地刷新。后台服务在跑时结果直接来自它。\n\
         \x20 send <设备名|设备ID> <文件>…\n\
         \x20     发送文件（可多个）。后台服务在跑时自动向它解析地址。\n\
         \x20 send --peer ip:端口 <文件>…\n\
         \x20     直连发送，绕过发现层，适合跨网段或调试。\n\
         \x20 serve [--dir 目录] [--yes] [--port 端口]\n\
         \x20     前台常驻接收，在终端里直接确认；调试用。\n\
         \n\
         安全与信任：\n\
         \x20 trust                            查看已信任设备\n\
         \x20 trust forget <名称|指纹>           取消某台设备的免确认\n\
         \x20 首次与陌生设备互传时，两端会显示同一个 6 位校验码，核对一致后确认即可。\n\
         \x20 设备身份私钥与信任列表在 ~/.glowlink/ 下。\n\
         \n\
         配置：\n\
         \x20 config                           显示生效配置与配置文件位置\n\
         \x20 --help                           显示本帮助\n\
         \n\
         配置文件优先级：内置默认 < ./glowlink.toml < ~/.config/glowlink/config.toml\\\n\
         （可用 $GLOWLINK_CONFIG 指定）< 命令行开关。\n\
         发现端口默认 {discovery_port}，被占用时自动顺延到相邻端口。\n"
    )
}
