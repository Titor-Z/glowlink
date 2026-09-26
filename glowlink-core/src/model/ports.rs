//! 端口分配策略：先试首选端口，被占用则向外顺延，全失败才算错。
//!
//! 不使用 `lsof` / `netstat` 判断占用：它们会漏掉沙盒进程与内核层占用
//! （实测本机某端口被一个 `lsof` 完全看不见的系统进程锁死），
//! 唯一可靠的判据是能否真正 `bind`。

use std::net::UdpSocket;

use crate::model::error::{GlowError, Result};

/// 从首选端口开始最多向外探测多少个端口。
pub const FALLBACK_SPAN: u16 = 100;

/// 一次端口尝试的结果，供调用方区分“被占”与“真出错”。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindOutcome {
    /// 绑定成功，返回实际可用端口。
    Bound(u16),
    /// 端口被占用（或权限不允许），可以换一个再试。
    Taken,
}

/// 试探某个 UDP 端口是否可用（绑定后立即释放）。
///
/// 这是判断端口占用与否的唯一可靠方式：系统工具会漏掉不可见的占用者，
/// 而 `bind` 是内核给出的最终裁决。
pub fn probe_udp(port: u16) -> BindOutcome {
    match UdpSocket::bind(("0.0.0.0", port)) {
        Ok(_) => BindOutcome::Bound(port),
        Err(e) if is_port_conflict(&e) => BindOutcome::Taken,
        // 其它错误（例如端口 0 或权限问题）同样认为该端口不可用
        Err(_) => BindOutcome::Taken,
    }
}

/// 在 `[preferred, preferred + FALLBACK_SPAN)` 范围内找到第一个可用端口。
///
/// 返回 `(实际端口, 是否为降级结果)`：首选端口可用时第二个值为 `false`，
/// 顺延成功时为 `true`，全部失败则报错并提示用户显式指定端口。
pub fn resolve_udp_port(preferred: u16) -> Result<(u16, bool)> {
    let start = preferred.max(1);
    for offset in 0..FALLBACK_SPAN {
        let candidate = match start.checked_add(offset) {
            Some(port) if port > 0 => port,
            // 顺延越界（例如首选接近 65535）：回绕到高位区间继续试
            _ => start.wrapping_add(offset),
        };
        if candidate == 0 {
            continue;
        }
        if probe_udp(candidate) == BindOutcome::Bound(candidate) {
            return Ok((candidate, offset != 0));
        }
    }
    Err(GlowError::Config(format!(
        "端口 {start} 起的 {FALLBACK_SPAN} 个端口全部被占用，请用 --discovery-port 指定其它端口"
    )))
}

/// 绑定 UDP 端口，失败时自动顺延；返回套接字、实际端口与是否降级。
///
/// 与 [`resolve_udp_port`] 的区别：本函数直接返回**真正持有**的套接字，
/// 避免“试探成功但随后被别的进程抢走”的竞态。
pub fn bind_udp_with_fallback(preferred: u16) -> Result<(UdpSocket, u16, bool)> {
    let start = preferred.max(1);
    for offset in 0..FALLBACK_SPAN {
        let candidate = match start.checked_add(offset) {
            Some(port) if port > 0 => port,
            _ => start.wrapping_add(offset),
        };
        if candidate == 0 {
            continue;
        }
        match UdpSocket::bind(("0.0.0.0", candidate)) {
            Ok(socket) => {
                socket.set_broadcast(true)?;
                return Ok((socket, candidate, offset != 0));
            }
            Err(e) if is_port_conflict(&e) => continue,
            Err(e) => return Err(GlowError::Io(e)),
        }
    }
    Err(GlowError::Config(format!(
        "端口 {start} 起的 {FALLBACK_SPAN} 个端口全部被占用，请用 --discovery-port 指定其它端口"
    )))
}

/// 端口冲突类错误：换一个端口就能解决。
pub fn is_port_conflict(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
    )
}