//! 状态文件与日志路径（`~/.glowlink`）。
//!
//! 状态文件是 CLI 找到守护进程的唯一入口，因此：
//! - **原子写入**：先写 `state.json.tmp` 再 `rename`，客户端永远读不到半截内容；
//! - **退出即删**：`off` 与守护进程自退都会删掉它，避免残留导致「以为还在跑」。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use glowlink_core::model::error::{GlowError, Result};

/// 状态目录名（位于用户主目录下）。
const STATE_DIR: &str = ".glowlink";

/// 状态文件名。
const STATE_FILE: &str = "state.json";

/// 日志文件名。
const LOG_FILE: &str = "daemon.log";

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// 状态目录 `~/.glowlink`。
pub fn state_dir() -> PathBuf {
    match home_dir() {
        Some(home) => home.join(STATE_DIR),
        // 拿不到主目录就退到当前目录，至少不 panic
        None => PathBuf::from(STATE_DIR),
    }
}

/// 状态文件 `~/.glowlink/state.json`。
pub fn state_path() -> PathBuf {
    state_dir().join(STATE_FILE)
}

/// 守护进程日志 `~/.glowlink/daemon.log`。
pub fn log_path() -> PathBuf {
    state_dir().join(LOG_FILE)
}

/// 追加一行守护进程日志；写失败时静默——日志本身不该拖垮主流程。
pub fn append_log(line: &str) {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "[{stamp}] {line}");
    }
}

/// 打开日志文件，用于守护进程子进程的 stdout/stderr 重定向。
pub fn open_log() -> Result<File> {
    let path = log_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(OpenOptions::new().create(true).append(true).open(path)?)
}

/// 守护进程写入的状态记录。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub version: String,
    pub pid: u32,
    pub ipc_port: u16,
    pub transfer_port: u16,
    pub discovery_port: u16,
    pub receive_dir: PathBuf,
    pub started_at: u64,
}

impl State {
    pub fn to_json(&self) -> String {
        format!(
            "{{\n  \"version\": \"{}\",\n  \"pid\": {},\n  \"ipc_port\": {},\n  \
             \"transfer_port\": {},\n  \"discovery_port\": {},\n  \"receive_dir\": \"{}\",\n  \
             \"started_at\": {}\n}}\n",
            escape(&self.version),
            self.pid,
            self.ipc_port,
            self.transfer_port,
            self.discovery_port,
            escape(&self.receive_dir.to_string_lossy()),
            self.started_at
        )
    }

    /// 解析自己写出的状态文件；缺字段或字段类型不对都算损坏。
    pub fn parse(text: &str) -> Result<Self> {
        let bad = |field: &str| GlowError::Protocol(format!("状态文件缺少字段「{field}」"));
        Ok(State {
            version: json_string(text, "version").ok_or_else(|| bad("version"))?,
            pid: json_u64(text, "pid").ok_or_else(|| bad("pid"))? as u32,
            ipc_port: json_u64(text, "ipc_port").ok_or_else(|| bad("ipc_port"))? as u16,
            transfer_port: json_u64(text, "transfer_port")
                .ok_or_else(|| bad("transfer_port"))? as u16,
            discovery_port: json_u64(text, "discovery_port")
                .ok_or_else(|| bad("discovery_port"))? as u16,
            receive_dir: PathBuf::from(
                json_string(text, "receive_dir").ok_or_else(|| bad("receive_dir"))?,
            ),
            started_at: json_u64(text, "started_at").unwrap_or(0),
        })
    }
}

/// 读取状态文件；不存在返回 `None`。
pub fn read_state() -> Result<Option<State>> {
    let path = state_path();
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)?;
    Ok(Some(State::parse(&text)?))
}

/// 原子写入状态文件：临时文件 + `rename`，避免客户端读到半截内容。
pub fn write_state(state: &State) -> Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{STATE_FILE}.tmp"));
    fs::write(&tmp, state.to_json())?;
    fs::rename(&tmp, state_path())?;
    Ok(())
}

/// 删除状态文件；不存在也算成功。
pub fn remove_state() -> Result<()> {
    let path = state_path();
    if path.is_file() {
        fs::remove_file(path)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- JSON 小工具
//
// 自己写状态文件、自己读回来，用一个只认字面量的小解析器就够了；
// 为此引入 serde 不值当（本项目零第三方依赖）。

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// 定位 `"key"` 之后冒号右侧的原文。
fn value_after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let start = text.find(&needle)? + needle.len();
    let rest = text[start..].trim_start();
    Some(rest.strip_prefix(':')?.trim_start())
}

fn json_string(text: &str, key: &str) -> Option<String> {
    let mut chars = value_after(text, key)?.strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    let code = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(code)?);
                }
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

fn json_u64(text: &str, key: &str) -> Option<u64> {
    let rest = value_after(text, key)?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}
