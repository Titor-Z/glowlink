//! 协议文本的编解码：错误行、`OK` 载荷、`RESOLVE` 应答。
//!
//! 错误种类名（`usage` / `config` / `notfound` / `missing` / `crypto` /
//! `rejected` / `net` / `io` / `protocol`）**是协议的一部分**：改名等于改协议。
//! 它们存在的唯一理由是让退出码能跨进程原样传回——只回正文的话，
//! 同一条命令会随「有没有开守护进程」给出不同退出码。

use std::io::ErrorKind;
use std::net::SocketAddr;

use glowlink_core::model::error::{GlowError, Result};

/// 响应结束标记。
pub const END: &str = "END";

/// 把错误编码成 IPC 响应行：`ERR <种类> <正文>`。
pub fn error_reply(error: &GlowError) -> String {
    format!("ERR {} {}", error_kind(error), plain_message(error))
}

/// 错误种类名（IPC 协议的一部分）。
pub fn error_kind(error: &GlowError) -> &'static str {
    match error {
        GlowError::Usage(_) => "usage",
        GlowError::Config(_) => "config",
        GlowError::NotFound(_) => "notfound",
        GlowError::Missing(_) => "missing",
        GlowError::Crypto(_) => "crypto",
        GlowError::Rejected(_) => "rejected",
        // 网络类 IO 单独一种：只回 "io" 的话，客户端还原出的是「其它 IO 错误」，
        // 退出码会从 3（网络）掉成 1（通用）——脚本的重试策略就跟着失效了
        GlowError::Io(e) if is_network_io(e.kind()) => "net",
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

/// 网络类 IO 错误：连不上 / 连断了 / 超时 / 地址用不了。
///
/// **唯一来源**：IPC 的「错误种类名」与 CLI 的退出码都用这一套判定。
/// 两处各写一份的话，同一个错误会出现「种类叫 net、退出码算通用」的分裂。
///
/// 不在此列的（文件不存在、权限不足、磁盘满）都算通用错误——它们跟网络无关，
/// 混在一起会让脚本「重试三次」重试一个永远不会成功的操作。
pub fn is_network_io(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::NotConnected
            | ErrorKind::TimedOut
            | ErrorKind::BrokenPipe
            | ErrorKind::NetworkUnreachable
            | ErrorKind::HostUnreachable
            | ErrorKind::AddrNotAvailable
    )
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
