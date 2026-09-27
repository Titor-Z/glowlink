//! 本地 IPC：回环 TCP 上的行协议。
//!
//! 为什么用文本行协议而不是 JSON：零依赖、可手敲（`nc`/telnet 就能调试）、
//! 日志里直接可读。每条响应都以 `END` 收尾，单行与多行响应在客户端
//! 走同一条读取路径，不必为多行指令写第二套解析。
//!
//! 指令一览：
//! - `PING`                    → `OK <版本> <PID> <传输端口> <发现端口>`
//! - `STATUS`                  → `OK <设备数> <待确认数> <接收目录>`
//! - `DEVICES`                 → `OK <n>` + n 行 `名称\tID\t地址\t存活毫秒`
//! - `RESOLVE <名称|ID>`       → `OK 名称\tID\t地址`
//! - `PENDING`                 → `OK <n>` + n 行 `编号\t来源\t文件名\t字节数\t指纹\t校验码`
//! - `ACCEPT [编号]` / `REJECT [编号]` → `OK <编号>`
//! - `STOP`                    → `OK …` 并关闭连接
//!
//! Step 3 的加密/配对将插在连接建立之后、指令分发之前，
//! 所以协议处理与守护进程本体（`daemon.rs`）刻意分开。

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use crate::pending::PendingOffer;
use crate::state::State;
use glowlink_core::model::error::{GlowError, Result};
use glowlink_core::Device;

/// 本地 IPC 首选端口。与发现端口（9267）无关：IPC 只监听回环，
/// 不参与局域网发现，单独占一个号段避免互相干扰。
pub const IPC_PORT: u16 = 10_000;

/// IPC 端口顺延范围。
pub const IPC_PORT_SPAN: u16 = 50;

/// 建立连接的等待时长：本机回环，超过这个时间说明守护进程已卡死或正在退出。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// 读一行响应的等待时长。
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// 响应结束标记。
const END: &str = "END";

/// IPC 指令处理所依赖的守护进程能力；抽成 trait 便于用测试替身覆盖协议分支。
pub trait IpcHost: Send + Sync {
    /// 状态快照（含实际端口与接收目录）。
    fn status(&self) -> State;

    /// 当前已知设备。
    fn devices(&self) -> Vec<Device>;

    /// 按名称或 ID 查设备。
    fn find(&self, key: &str) -> Option<Device>;

    /// 全部待确认请求：`(编号, 请求)`。
    fn pending(&self) -> Vec<(String, PendingOffer)>;

    /// 落定待确认请求（`token` 为空表示唯一那个），返回被处理的编号。
    fn decide(&self, token: Option<&str>, accept: bool) -> Result<String>;

    /// 请求守护进程退出。
    fn request_stop(&self);
}

/// 处理一行 IPC 指令，返回要写回的行；`true` 表示连接可以继续复用。
pub fn handle_line(host: &dyn IpcHost, line: &str) -> (Vec<String>, bool) {
    let line = line.trim();
    let (cmd, arg) = match line.split_once(' ') {
        Some((cmd, arg)) => (cmd, arg.trim()),
        None => (line, ""),
    };
    match cmd {
        "PING" => {
            let state = host.status();
            (
                vec![format!(
                    "OK {} {} {} {}",
                    state.version, state.pid, state.transfer_port, state.discovery_port
                )],
                true,
            )
        }
        "STATUS" => {
            let state = host.status();
            (
                vec![format!(
                    "OK {} {} {}",
                    host.devices().len(),
                    host.pending().len(),
                    state.receive_dir.display()
                )],
                true,
            )
        }
        "DEVICES" => {
            let devices = host.devices();
            let mut lines = vec![format!("OK {}", devices.len())];
            for device in &devices {
                lines.push(format!(
                    "{}\t{}\t{}\t{}",
                    device.name,
                    device.id.as_str(),
                    device.addr,
                    device.last_seen.elapsed().as_millis()
                ));
            }
            (lines, true)
        }
        "RESOLVE" => match host.find(arg) {
            Some(device) => (
                vec![format!(
                    "OK {}\t{}\t{}",
                    device.name,
                    device.id.as_str(),
                    device.addr
                )],
                true,
            ),
            // 走统一的错误编码：手写 ERR 行会漏掉种类，客户端只能当通用错误（退出码会不对）
            None => (
                vec![error_reply(&GlowError::NotFound(arg.to_string()))],
                true,
            ),
        },
        "PENDING" => {
            let pending = host.pending();
            let mut lines = vec![format!("OK {}", pending.len())];
            for (token, offer) in &pending {
                lines.push(format!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    token, offer.peer, offer.name, offer.size, offer.fingerprint, offer.code
                ));
            }
            (lines, true)
        }
        "ACCEPT" | "REJECT" => {
            let accept = cmd == "ACCEPT";
            let token = if arg.is_empty() { None } else { Some(arg) };
            match host.decide(token, accept) {
                Ok(token) => {
                    let verb = if accept { "已接受" } else { "已拒绝" };
                    (vec![format!("OK {verb} #{token}")], true)
                }
                // 回原文而不是 `e.to_string()`：否则客户端会叠成「协议错误：用法错误：…」
                // `ERR <种类> <正文>`：客户端据此还原错误种类，退出码才不会「守护进程在跑就变 1」
                Err(e) => (vec![error_reply(&e)], true),
            }
        }
        "STOP" => {
            host.request_stop();
            (vec!["OK 正在停止".to_string()], false)
        }
        "" => (
            vec![error_reply(&GlowError::Protocol("空指令".to_string()))],
            true,
        ),
        other => (
            vec![error_reply(&GlowError::Protocol(format!("未知指令「{other}」")))],
            true,
        ),
    }
}

/// 把错误编码成 IPC 响应行：`ERR <种类> <正文>`。
///
/// 为什么要带种类：退出码依赖错误种类（用法 2 / 网络 3 / 信任 4 / 拒绝 5），
/// 只回正文的话守护进程在跑时所有错误都会退化成通用错误 1，脚本行为会随
/// 「有没有开守护进程」而变——这种不一致最难排查。
pub fn error_reply(error: &GlowError) -> String {
    format!("ERR {} {}", error_kind(error), plain_message(error))
}

/// 错误种类名（IPC 协议的一部分，改名等于改协议）。
fn error_kind(error: &GlowError) -> &'static str {
    match error {
        GlowError::Usage(_) => "usage",
        GlowError::Config(_) => "config",
        GlowError::NotFound(_) => "notfound",
        GlowError::Missing(_) => "missing",
        GlowError::Crypto(_) => "crypto",
        GlowError::Rejected(_) => "rejected",
        // 网络类 IO 单独一种：只回 "io" 的话，客户端还原出的是「其它 IO 错误」，
        // 退出码会从 3（网络）掉成 1（通用）——脚本的重试策略就跟着失效了
        GlowError::Io(e) if crate::exit::is_network_io(e.kind()) => "net",
        GlowError::Io(_) => "io",
        GlowError::Protocol(_) => "protocol",
        GlowError::Remote(_) => "remote",
    }
}

/// `ERR <种类> <正文>` → 错误。种类不认识时退回 `Remote`（老守护进程仍能配合）。
pub fn error_from_line(line: &str) -> GlowError {
    let body = line.strip_prefix("ERR ").unwrap_or(line);
    let (kind, text) = match body.split_once(' ') {
        Some((kind, text)) => (kind, text.to_string()),
        None => ("", body.to_string()),
    };
    match kind {
        "usage" => GlowError::Usage(text),
        "config" => GlowError::Config(text),
        "notfound" => GlowError::NotFound(text),
        "missing" => GlowError::Missing(text),
        "crypto" => GlowError::Crypto(text),
        "rejected" => GlowError::Rejected(text),
        "net" => GlowError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            text,
        )),
        "io" => GlowError::Io(std::io::Error::other(text)),
        "protocol" => GlowError::Protocol(text),
        _ => GlowError::Remote(text),
    }
}

/// 取错误的正文，去掉本机错误类型的前缀。
fn plain_message(error: &GlowError) -> String {
    match error {
        GlowError::Io(e) => e.to_string(),
        GlowError::Protocol(m)
        | GlowError::Rejected(m)
        | GlowError::NotFound(m)
        | GlowError::Missing(m)
        | GlowError::Usage(m)
        | GlowError::Config(m)
        | GlowError::Remote(m)
        | GlowError::Crypto(m) => m.clone(),
    }
}

/// 把响应行写进流，并补上结束标记。
fn write_reply(stream: &mut TcpStream, lines: &[String]) -> std::io::Result<()> {
    for line in lines {
        writeln!(stream, "{line}")?;
    }
    writeln!(stream, "{END}")?;
    stream.flush()
}

/// 服务一次 IPC 连接，直到对端关闭或发出 `STOP`。
pub fn serve_connection(host: &dyn IpcHost, stream: TcpStream) {
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let (reply, keep_open) = handle_line(host, &line);
        if write_reply(&mut writer, &reply).is_err() {
            break;
        }
        if !keep_open {
            break;
        }
    }
}

/// 绑定回环上的 IPC 监听器；首选端口被占用时顺延。
///
/// `preferred` 为 0 表示「随便给一个空闲端口」（测试与临时用途）。
pub fn bind_ipc(preferred: u16) -> Result<(TcpListener, u16)> {
    if preferred == 0 {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        return Ok((listener, port));
    }
    let mut last_error = None;
    for offset in 0..IPC_PORT_SPAN {
        let port = preferred.saturating_add(offset);
        match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))) {
            Ok(listener) => return Ok((listener, port)),
            Err(e) => last_error = Some(e),
        }
    }
    Err(match last_error {
        Some(e) => GlowError::Io(e),
        None => GlowError::Usage("没有可用的 IPC 端口".into()),
    })
}

// ---------------------------------------------------------------- 客户端

/// 读取一行；超时或对端关闭都算失败。
fn read_line(reader: &mut BufReader<TcpStream>) -> Result<String> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => Err(GlowError::Protocol("守护进程已断开连接".into())),
        Ok(_) => Ok(line.trim_end().to_string()),
        Err(e) => Err(GlowError::Io(e)),
    }
}

pub struct IpcClient {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}

impl IpcClient {
    pub fn connect(state: &State) -> Result<Self> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, state.ipc_port));
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.set_write_timeout(Some(READ_TIMEOUT))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(IpcClient { writer: stream, reader })
    }

    /// 发一条指令并读取完整响应：`OK …` / `ERR …` 首行 + 数据行，直到 `END`。
    pub fn call(&mut self, command: &str) -> Result<Vec<String>> {
        writeln!(self.writer, "{command}")?;
        self.writer.flush()?;
        let mut lines = Vec::new();
        loop {
            let line = read_line(&mut self.reader)?;
            if line == END {
                break;
            }
            lines.push(line);
        }
        match lines.first() {
            Some(first) if first.starts_with("ERR ") => Err(error_from_line(first)),
            Some(_) => Ok(lines),
            None => Err(GlowError::Protocol("守护进程返回了空响应".into())),
        }
    }

    /// 只取首行（单行指令的常用形态）。
    pub fn call_first(&mut self, command: &str) -> Result<String> {
        Ok(self.call(command)?.remove(0))
    }
}

/// 取首行 `OK ` 之后的正文。
pub fn ok_payload(line: &str) -> &str {
    line.strip_prefix("OK ").unwrap_or(line).trim()
}

/// 把 `RESOLVE` 的响应解析成 `(名称, ID, 地址)`。
pub fn parse_resolve(line: &str) -> Result<(String, String, SocketAddr)> {
    let payload = ok_payload(line);
    let mut parts = payload.split('\t');
    let (name, id, addr) = (parts.next(), parts.next(), parts.next());
    match (name, id, addr.and_then(|a| a.parse::<SocketAddr>().ok())) {
        (Some(name), Some(id), Some(addr)) => Ok((name.to_string(), id.to_string(), addr)),
        _ => Err(GlowError::Protocol(format!(
            "无法解析守护进程应答：{payload}"
        ))),
    }
}

/// 把 `DEVICES` 的数据行解析成 `(名称, ID, 地址, 存活毫秒)`。
pub fn parse_device_line(line: &str) -> Option<(String, String, SocketAddr, u128)> {
    let mut parts = line.split('\t');
    let (name, id, addr, age) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    Some((
        name.to_string(),
        id.to_string(),
        addr.parse().ok()?,
        age.parse().ok()?,
    ))
}

/// 把 `PENDING` 的数据行解析成 `(编号, 来源, 文件名, 字节数, 指纹, 校验码)`。
pub fn parse_pending_line(line: &str) -> Option<(String, String, String, u64, String, String)> {
    let mut parts = line.split('\t');
    let token = parts.next()?;
    let peer = parts.next()?;
    let name = parts.next()?;
    let size: u64 = parts.next()?.parse().ok()?;
    let fingerprint = parts.next().unwrap_or("").to_string();
    let code = parts.next().unwrap_or("").to_string();
    Some((
        token.to_string(),
        peer.to_string(),
        name.to_string(),
        size,
        fingerprint,
        code,
    ))
}
