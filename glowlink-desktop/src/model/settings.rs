//! Model：桌面版设置（设置面板背后的那份数据）。
//!
//! 解析与取值全是纯函数（零 IO、零 GPUI，可单测）；文件的定位与读写集中在
//! [`load`] / [`save`]。持久化位置是**用户层** `~/.config/glowlink/settings.toml`：
//! 这里存的是「这台机器上用户挑的」，与守护进程的工程配置（`glowlink.toml`，
//! 编译期常量）不是一个东西，职责别混。
//!
//! 格式与核心配置同一套扁平 `key = value`：`#` 起注释，值可加双引号。
//! **未知键与非法值不静默吞**：记日志指出行号——静默忽略会让用户以为改了却没生效。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 主题三选一（设计稿的分段控件）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    /// 跟随系统（深色/浅色由 OS 外观决定，每次渲染现问窗口）。
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeChoice {
    pub fn as_token(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Dark => "dark",
            ThemeChoice::Light => "light",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "system" => Some(ThemeChoice::System),
            "dark" => Some(ThemeChoice::Dark),
            "light" => Some(ThemeChoice::Light),
            _ => None,
        }
    }

    /// 在分段控件里的档位（跟随系统 / 深色 / 浅色）——滑块按它算坐标。
    pub fn index(self) -> usize {
        match self {
            ThemeChoice::System => 0,
            ThemeChoice::Dark => 1,
            ThemeChoice::Light => 2,
        }
    }
}

/// 一份完整的桌面设置。**字段都有默认值**：新键是后加的，旧文件缺了就按默认走。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// 空态（附近没有设备时）显示不显示本机名字。
    pub self_name_in_empty: bool,
    /// 空态结论（「附近没有设备」）是否常驻（默认报完 6 秒就收起）。
    pub note_persistent: bool,
    /// 接收目录（`None` = 用内置默认 `~/.glowlink/received`）。
    ///
    /// 只影响**桌面版拉起的**守护进程（拉起时带 `--dir`）：用户自己 `glowlink on`
    /// 起的服务，目录仍归 CLI 的配置管——桌面版不越权改别人的配置文件。
    pub receive_dir: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeChoice::System,
            // 用户拍板：暂时显示（设置页留开关，见 AGENTS.md 待做清单）
            self_name_in_empty: true,
            note_persistent: false,
            receive_dir: None,
        }
    }
}

/// 一行设置怎么落到 [`Settings`] 上。返回 `Err` = 值不合法（调用方记日志）。
pub fn apply_line(settings: &mut Settings, key: &str, value: &str) -> Result<(), String> {
    match key {
        "theme" => {
            settings.theme = ThemeChoice::from_token(value)
                .ok_or_else(|| format!("theme 的值不认识：{value}（可用 system/dark/light）"))?;
        }
        "self_name_in_empty" => settings.self_name_in_empty = parse_bool(key, value)?,
        "note_persistent" => settings.note_persistent = parse_bool(key, value)?,
        "receive_dir" => {
            let trimmed = value.trim();
            settings.receive_dir = (!trimmed.is_empty()).then(|| trimmed.to_string());
        }
        // 未知键报错而不是跳过：写错了用户要知道，否则以为改了
        other => return Err(format!("不认识的设置项：{other}")),
    }
    Ok(())
}

fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => Err(format!("{key} 要 true/false，收到 {other}")),
    }
}

/// 解析整份文本。返回（设置，问题列表）：坏行**跳过并报告**，
/// 不让一行笔误把整份设置作废——设置面板还有界面兜底，不至于瞎。
pub fn parse(text: &str) -> (Settings, Vec<String>) {
    let mut settings = Settings::default();
    let mut problems = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            problems.push(format!("第 {} 行缺「=」：{}", index + 1, raw.trim()));
            continue;
        };
        let value = unquote(value.trim());
        if let Err(problem) = apply_line(&mut settings, key.trim(), &value) {
            problems.push(format!("第 {} 行：{}", index + 1, problem));
        }
    }
    (settings, problems)
}

/// `#` 起注释（双引号内的 `#` 不算）——与核心配置同一套口径。
fn strip_comment(line: &str) -> &str {
    let mut in_quotes = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '#' if !in_quotes => return &line[..index],
            _ => {}
        }
    }
    line
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// 落成文件文本（写回时全量生成，注释不保留——机器管理的文件不假装是手写的）。
pub fn serialize(settings: &Settings) -> String {
    let mut body = String::from("# 萤光 GlowLink 桌面版设置（改完保存，重开窗口生效）\n");
    body.push_str(&format!("theme = {}\n", settings.theme.as_token()));
    body.push_str(&format!(
        "self_name_in_empty = {}\n",
        settings.self_name_in_empty
    ));
    body.push_str(&format!("note_persistent = {}\n", settings.note_persistent));
    if let Some(dir) = &settings.receive_dir {
        body.push_str(&format!("receive_dir = {dir}\n"));
    }
    body
}

/// 接收目录的「当前值」：守护进程在跑就以它为准（那是事实），
/// 否则退回本机设置——服务没在跑时守护进程报不出目录，
/// 不这么兜底就会拿 `None` 把用户刚选的目录冲掉（界面看着像「选了不变」）。
pub fn effective_receive_dir(
    from_daemon: Option<String>,
    configured: Option<&str>,
) -> Option<String> {
    from_daemon.or_else(|| configured.map(str::to_string))
}

/// 设置文件位置：`~/.config/glowlink/settings.toml`（与用户配置同一层）。
pub fn settings_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("glowlink").join("settings.toml")
}

/// 读设置。文件不存在就是默认值（第一次用很正常，不算错误）。
/// 解析出的问题记日志（`log` 已由 main 初始化），不让坏行静默消失。
pub fn load() -> Settings {
    match std::fs::read_to_string(settings_path()) {
        Ok(text) => {
            let (settings, problems) = parse(&text);
            for problem in problems {
                log::info!("设置文件：{problem}");
            }
            settings
        }
        Err(_) => Settings::default(),
    }
}

/// 保存设置：临时文件 + rename 原子写（与状态文件、信任列表同一套做法，
/// 写一半断电也不会留半个文件在那儿）。
pub fn save(settings: &Settings) -> std::io::Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("toml.{}.tmp", now_secs()));
    std::fs::write(&tmp, serialize(settings))?;
    std::fs::rename(&tmp, &path)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
