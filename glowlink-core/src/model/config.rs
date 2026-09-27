//! 工程配置：纯解析 + 默认值，不做任何文件 IO。
//!
//! 配置来源优先级的合并发生在 `Config::apply`：以「已有配置」为底，
//! 只覆盖文件中显式出现的键，因此可以逐层叠加（内置默认 → 工程 → 用户 → 命令行）。
//!
//! 文件格式（扁平 `key = value`，`#` 起注释，字符串可加双引号）：
//! ```text
//! discovery_port = 9267
//! receive_dir = "./received"
//! ```

use std::path::PathBuf;

use crate::model::error::{GlowError, Result};
use crate::model::packet::DISCOVERY_PORT;
use crate::model::ports::DISCOVERY_PORT_SPAN;

/// 全部受支持的配置项，用于拼装「未知配置项」提示。
pub const KEYS: [&str; 11] = [
    "discovery_port",
    "discovery_port_span",
    "transfer_port",
    "receive_dir",
    "query_interval_secs",
    "device_timeout_secs",
    "scan_timeout_secs",
    "connect_timeout_secs",
    "pending_timeout_secs",
    "chunk_size_kib",
    "ring_capacity_kib",
];

const DEFAULT_DISCOVERY_PORT: u16 = DISCOVERY_PORT;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// 发现端口：局域网内所有节点应保持一致。
    pub discovery_port: u16,
    /// 查询要覆盖的候选端口个数（约定端口加顺延位）。
    ///
    /// 同时决定「本机最多顺延到哪」与「去哪里找别人」，所以**局域网内应保持一致**：
    /// 本机调小、对端顺延得更远时，就会出现「它开着但我找不到」。
    pub discovery_port_span: u16,
    /// TCP 传输端口，0 表示由系统自动分配。
    pub transfer_port: u16,
    /// 接收文件的目录。
    pub receive_dir: PathBuf,
    /// 主动查询周期（秒）。
    pub query_interval_secs: u64,
    /// 多久收不到对方消息就判定离线（秒）。
    pub device_timeout_secs: u64,
    /// `list` / `send` 的默认搜索时长（秒）。
    pub scan_timeout_secs: u64,
    /// 建连超时（秒）。
    pub connect_timeout_secs: u64,
    /// 守护进程等待人工确认接收的上限（秒），超时按拒绝处理。
    ///
    /// 给得宽一些：人可能离开一会儿，60 秒那种「回来发现被自动拒了」很恼人。
    pub pending_timeout_secs: u64,
    /// 分块大小（KiB）。
    pub chunk_size_kib: usize,
    /// 环形缓冲容量（KiB），背压控制的上限。
    pub ring_capacity_kib: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            discovery_port: DEFAULT_DISCOVERY_PORT,
            discovery_port_span: DISCOVERY_PORT_SPAN,
            transfer_port: 0,
            receive_dir: PathBuf::from("./received"),
            query_interval_secs: 1,
            device_timeout_secs: 6,
            scan_timeout_secs: 3,
            connect_timeout_secs: 10,
            pending_timeout_secs: 300,
            chunk_size_kib: 64,
            ring_capacity_kib: 1024,
        }
    }
}

impl Config {
    /// 在内置默认之上解析一份配置文本。
    pub fn parse(text: &str) -> Result<Config> {
        let mut config = Config::default();
        config.apply(text)?;
        Ok(config)
    }

    /// 只覆盖文本中显式出现的键，其余保持当前值。
    pub fn apply(&mut self, text: &str) -> Result<()> {
        for (index, raw) in text.lines().enumerate() {
            let line_no = index + 1;
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                return Err(bad(line_no, "不支持 [分节]，请使用扁平的 key = value"));
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| bad(line_no, "缺少 `=`，应为 key = value"))?;
            self.set(line_no, key.trim(), unquote(value.trim()))?;
        }
        Ok(())
    }

    pub fn chunk_size(&self) -> usize {
        self.chunk_size_kib.max(1) * 1024
    }

    pub fn ring_capacity(&self) -> usize {
        self.ring_capacity_kib.max(1) * 1024
    }

    fn set(&mut self, line_no: usize, key: &str, value: &str) -> Result<()> {
        match key {
            "discovery_port" => self.discovery_port = parse_u16(line_no, key, value)?,
            "discovery_port_span" => {
                self.discovery_port_span = parse_span(line_no, key, value)?
            }
            "transfer_port" => self.transfer_port = parse_u16(line_no, key, value)?,
            "receive_dir" => self.receive_dir = PathBuf::from(value),
            "query_interval_secs" => self.query_interval_secs = parse_u64(line_no, key, value)?,
            "device_timeout_secs" => self.device_timeout_secs = parse_u64(line_no, key, value)?,
            "scan_timeout_secs" => self.scan_timeout_secs = parse_u64(line_no, key, value)?,
            "connect_timeout_secs" => self.connect_timeout_secs = parse_u64(line_no, key, value)?,
            "pending_timeout_secs" => {
                self.pending_timeout_secs = parse_u64(line_no, key, value)?
            }
            "chunk_size_kib" => self.chunk_size_kib = parse_usize(line_no, key, value)?,
            "ring_capacity_kib" => self.ring_capacity_kib = parse_usize(line_no, key, value)?,
            other => {
                return Err(bad(
                    line_no,
                    &format!("未知配置项 `{other}`（可用：{}）", KEYS.join("、")),
                ))
            }
        }
        Ok(())
    }
}

fn bad(line_no: usize, message: &str) -> GlowError {
    GlowError::Config(format!("第 {line_no} 行：{message}"))
}

/// 候选端口个数：太小容易「顺延即失联」，太大等于每秒刷几十个广播包。
fn parse_span(line_no: usize, key: &str, value: &str) -> Result<u16> {
    let span = parse_u16(line_no, key, value)?;
    if !(1..=MAX_PORT_SPAN).contains(&span) {
        return Err(bad(
            line_no,
            &format!("`{key}` 需在 1..={MAX_PORT_SPAN} 之间，收到「{value}」"),
        ));
    }
    Ok(span)
}

/// 候选端口个数的上限（广播流量与容错能力的折中）。
pub const MAX_PORT_SPAN: u16 = 64;

fn parse_u16(line_no: usize, key: &str, value: &str) -> Result<u16> {
    value
        .parse()
        .map_err(|_| bad(line_no, &format!("`{key}` 需要 0-65535 的整数，收到「{value}」")))
}

fn parse_u64(line_no: usize, key: &str, value: &str) -> Result<u64> {
    value
        .parse()
        .map_err(|_| bad(line_no, &format!("`{key}` 需要非负整数，收到「{value}」")))
}

fn parse_usize(line_no: usize, key: &str, value: &str) -> Result<usize> {
    value
        .parse()
        .map_err(|_| bad(line_no, &format!("`{key}` 需要非负整数，收到「{value}」")))
}

/// 去掉行内注释，双引号内的 `#` 不算注释。
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

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value)
}
