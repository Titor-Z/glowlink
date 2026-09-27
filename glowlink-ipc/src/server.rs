//! IPC 服务端骨架：绑定回环监听、按行处理指令、逐连接服务。
//!
//! 守护进程的**能力**经 [`IpcHost`] 注入，所以协议分支可以用测试替身覆盖，
//! 不必真的起一个守护进程。真正的落地（发现循环、待确认队列、传输）
//! 仍在 CLI 里——本 crate 只负责协议。

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};

use glowlink_core::model::error::{GlowError, Result};
use glowlink_core::Device;

use crate::codec::{error_reply, END};
use crate::row::{DeviceRow, PairingRow, PendingRow};
use crate::state::State;

/// 本地 IPC 首选端口。与发现端口（9267）无关：IPC 只监听回环，
/// 不参与局域网发现，单独占一个号段避免互相干扰。
pub const IPC_PORT: u16 = 10_000;

/// IPC 端口顺延范围。
pub const IPC_PORT_SPAN: u16 = 50;

/// IPC 指令处理所依赖的守护进程能力；抽成 trait 便于用测试替身覆盖协议分支。
pub trait IpcHost: Send + Sync {
    /// 状态快照（含实际端口与接收目录）。
    fn status(&self) -> State;

    /// 当前已知设备。
    fn devices(&self) -> Vec<Device>;

    /// 按名称或 ID 查设备。
    fn find(&self, key: &str) -> Option<Device>;

    /// 全部待确认请求（编号在 [`PendingRow::token`] 里）。
    fn pending(&self) -> Vec<PendingRow>;

    /// 落定待确认请求（`token` 为空表示唯一那个），返回被处理的编号。
    fn decide(&self, token: Option<&str>, accept: bool) -> Result<String>;

    /// 发起一次配对：连上对端并完成握手，返回 `(编号, 6 位校验码)`。
    ///
    /// 返回后配对**还在进行**（等对方输码），结果靠 [`Self::pairings`] 查。
    fn pair(&self, key: &str) -> Result<(String, String)>;

    /// 本机发起的配对槽位（含已结束的，供界面显示成败）。
    fn pairings(&self) -> Vec<PairingRow>;

    /// 对端发起、等本机用户决定的配对请求。
    fn pair_pending(&self) -> Vec<PairingRow>;

    /// 落定一个待决定的配对请求：`code` 为 `Some` 表示允许并提交用户输入的码。
    fn pair_decide(&self, token: &str, code: Option<String>) -> Result<String>;

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
                lines.push(DeviceRow::from_device(device).to_line());
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
            for row in &pending {
                lines.push(row.to_line());
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
        "PAIR" => match host.pair(arg) {
            // 码一并回给调用方：界面要立刻就把它显示出来给人核对，
            // 不能等配对结束再回（那时候人早忘了自己看到的是什么）。
            Ok((token, code)) => (vec![format!("OK {token} {code}")], true),
            Err(e) => (vec![error_reply(&e)], true),
        },
        "PAIRINGS" => {
            let rows = host.pairings();
            let mut lines = vec![format!("OK {}", rows.len())];
            for row in &rows {
                lines.push(row.to_line());
            }
            (lines, true)
        }
        "PAIRPENDING" => {
            let rows = host.pair_pending();
            let mut lines = vec![format!("OK {}", rows.len())];
            for row in &rows {
                lines.push(row.to_line());
            }
            (lines, true)
        }
        // `PAIR_ACCEPT <编号> <6 位码>`：码是**用户敲进去的**，不是从别处抄来的
        "PAIR_ACCEPT" => {
            let mut parts = arg.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some(token), Some(code)) => match host.pair_decide(token, Some(code.to_string())) {
                    Ok(token) => (vec![format!("OK 已接受 #{token}")], true),
                    Err(e) => (vec![error_reply(&e)], true),
                },
                _ => (
                    vec![error_reply(&GlowError::Usage(
                        "用法：PAIR_ACCEPT <编号> <6位码>".into(),
                    ))],
                    true,
                ),
            }
        }
        "PAIR_REJECT" => match host.pair_decide(arg, None) {
            Ok(token) => (vec![format!("OK 已拒绝 #{token}")], true),
            Err(e) => (vec![error_reply(&e)], true),
        },
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
