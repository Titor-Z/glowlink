//! 状态文件与日志路径（`~/.glowlink`）。
//!
//! 状态文件是 CLI 找到守护进程的唯一入口，因此：
//! - **原子写入**：先写 `state.json.tmp` 再 `rename`，客户端永远读不到半截内容；
//! - **退出即删**：`off` 与守护进程自退都会删掉它，避免残留导致「以为还在跑」。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
/// 单条日志的最大字节数：超过就滚一份 `daemon.log.1`，避免日志把磁盘写满。
pub const LOG_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// 同一条消息连刷时，每隔这么久补一行「重复 N 次」的汇总。
const REPEAT_SUMMARY: Duration = Duration::from_secs(60);

/// 重复消息过滤器。
///
/// 网络中断这类环境问题一秒能产生上万条**完全相同**的日志：全写下来既没用
/// （信息量为零）又会写爆磁盘。这里只保留「第一条」与「每分钟一次的汇总」。
#[derive(Default)]
pub struct RepeatFilter {
    last: Option<(String, u64, Instant)>,
}

impl RepeatFilter {
    /// 返回应当写盘的内容：`None` 表示这条被抑制。
    pub fn admit(&mut self, line: &str, now: Instant) -> Option<String> {
        match &mut self.last {
            Some((prev, count, since)) if prev == line => {
                *count += 1;
                if now.duration_since(*since) >= REPEAT_SUMMARY {
                    let text = format!("{line}（同类消息已重复 {count} 次）");
                    *since = now;
                    *count = 0;
                    Some(text)
                } else {
                    None
                }
            }
            _ => {
                self.last = Some((line.to_string(), 0, now));
                Some(line.to_string())
            }
        }
    }
}

pub fn append_log(line: &str) {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // 进程内共用一个过滤器：日志是单例资源，用锁保证「第一条一定写下去」
    static FILTER: Mutex<Option<RepeatFilter>> = Mutex::new(None);
    let to_write = {
        let mut guard = FILTER.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get_or_insert_with(RepeatFilter::default)
            .admit(line, Instant::now())
    };
    // 被抑制（同一条消息的后续重复）时直接返回，不碰磁盘
    let Some(to_write) = to_write else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "[{stamp}] {to_write}");
    }
}

/// 日志超过上限就滚一份：旧内容改名为 `.1`（只留一代），新日志从空开始。
fn rotate_log(path: &Path) {
    let too_big = fs::metadata(path).map(|m| m.len() > LOG_MAX_BYTES).unwrap_or(false);
    if !too_big {
        return;
    }
    let old = path.with_extension("log.1");
    let _ = fs::remove_file(&old);
    let _ = fs::rename(path, &old);
}

/// 打开日志文件，用于守护进程子进程的 stdout/stderr 重定向。
pub fn open_log() -> Result<File> {
    let path = log_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    rotate_log(&path);
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
