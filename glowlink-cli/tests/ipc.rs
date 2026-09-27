//! `glowlink-cli/src/ipc.rs` 的单元测试：协议分支 + 回环真实收发。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use glowlink_cli::ipc::{
    bind_ipc, handle_line, serve_connection, IpcClient, IpcHost, parse_device_line,
    parse_pending_line, parse_resolve,
};
use glowlink_cli::pending::PendingOffer;
use glowlink_cli::state::State;
use glowlink_core::model::error::{GlowError, Result};
use glowlink_core::{Device, DeviceId};

fn sample_state() -> State {
    State {
        version: "0.3.0".to_string(),
        pid: 4242,
        ipc_port: 10_000,
        transfer_port: 51_234,
        discovery_port: 9267,
        receive_dir: "/tmp/接收 目录".into(),
        started_at: 1_700_000_000,
    }
}

fn a_device(name: &str, addr: SocketAddr) -> Device {
    Device {
        id: DeviceId::new("abc123def4567890"),
        name: name.to_string(),
        addr,
        last_seen: Instant::now(),
    }
}

/// 测试替身：把守护进程状态塞进内存，覆盖协议各分支。
#[derive(Default)]
struct FakeHost {
    devices: Mutex<Vec<Device>>,
    pending: Mutex<Vec<(String, PendingOffer)>>,
    decided: Mutex<Vec<(String, bool)>>,
    stopped: AtomicBool,
}

impl IpcHost for FakeHost {
    fn status(&self) -> State {
        sample_state()
    }

    fn devices(&self) -> Vec<Device> {
        self.devices.lock().unwrap().clone()
    }

    fn find(&self, key: &str) -> Option<Device> {
        self.devices
            .lock()
            .unwrap()
            .iter()
            .find(|d| d.name == key || d.id.as_str() == key)
            .cloned()
    }

    fn pending(&self) -> Vec<(String, PendingOffer)> {
        self.pending.lock().unwrap().clone()
    }

    fn decide(&self, token: Option<&str>, accept: bool) -> Result<String> {
        let mut pending = self.pending.lock().unwrap();
        let token = match token {
            Some(token) => token.to_string(),
            None if pending.len() == 1 => pending[0].0.clone(),
            None if pending.is_empty() => {
                return Err(GlowError::Usage("当前没有待确认的接收请求".into()))
            }
            None => {
                return Err(GlowError::Usage(format!(
                    "有 {} 个待确认请求，请指定编号",
                    pending.len()
                )))
            }
        };
        if !pending.iter().any(|(t, _)| *t == token) {
            return Err(GlowError::Usage(format!("没有编号为 {token} 的待确认请求")));
        }
        self.decided.lock().unwrap().push((token.clone(), accept));
        pending.retain(|(t, _)| *t != token);
        Ok(token)
    }

    fn request_stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

fn pending_offer(peer: &str, name: &str, size: u64) -> PendingOffer {
    PendingOffer {
        peer: peer.into(),
        name: name.into(),
        size,
        fingerprint: "abcdef0123456789".into(),
        code: "482913".into(),
    }
}

// ---------------------------------------------------------------- 协议分支

#[test]
fn ping_reports_version_pid_and_ports() {
    let host = FakeHost::default();
    let (lines, keep_open) = handle_line(&host, "PING");
    assert!(lines[0].starts_with("OK 0.3.0 4242 51234 9267"), "实际：{}", lines[0]);
    assert!(keep_open, "PING 之后连接应可复用");
}

#[test]
fn status_ends_with_pending_count() {
    let host = FakeHost::default();
    let (lines, _) = handle_line(&host, "STATUS");
    assert!(lines[0].ends_with("/tmp/接收 目录"), "最后一段仍是接收目录：{}", lines[0]);
    assert!(lines[0].starts_with("OK 0 0 "), "设备数 0、待确认数 0：{}", lines[0]);

    host.pending
        .lock()
        .unwrap()
        .push(("001".into(), pending_offer("10.0.0.1", "a.bin", 1)));
    let (lines, _) = handle_line(&host, "STATUS");
    assert!(lines[0].starts_with("OK 0 1 "), "待确认数应出现在字段里：{}", lines[0]);
}

#[test]
fn devices_lists_rows_with_tabs() {
    let addr: SocketAddr = "192.168.31.37:53500".parse().unwrap();
    let host = FakeHost::default();
    host.devices.lock().unwrap().push(a_device("甲机", addr));

    let (lines, _) = handle_line(&host, "DEVICES");
    assert_eq!(lines[0], "OK 1");
    let (name, _, parsed, _) = parse_device_line(&lines[1]).expect("数据行要能被客户端解析");
    assert_eq!(name, "甲机");
    assert_eq!(parsed, addr);
}

#[test]
fn devices_without_matches_is_empty_not_error() {
    let host = FakeHost::default();
    let (lines, _) = handle_line(&host, "DEVICES");
    assert_eq!(lines, vec!["OK 0".to_string()]);
}

#[test]
fn resolve_finds_by_name_and_id_rejects_unknown() {
    let addr: SocketAddr = "127.0.0.1:53000".parse().unwrap();
    let host = FakeHost::default();
    host.devices.lock().unwrap().push(a_device("乙机", addr));

    let (ok, _) = handle_line(&host, "RESOLVE 乙机");
    let (name, id, parsed) = parse_resolve(&ok[0]).unwrap();
    assert_eq!((name.as_str(), id.as_str(), parsed), ("乙机", "abc123def4567890", addr));

    let (by_id, _) = handle_line(&host, "RESOLVE abc123def4567890");
    assert!(parse_resolve(&by_id[0]).is_ok());

    let (err, _) = handle_line(&host, "RESOLVE 幽灵机");
    assert!(err[0].starts_with("ERR "), "实际：{}", err[0]);
}

#[test]
fn pending_returns_a_list_with_tokens() {
    let host = FakeHost::default();
    *host.pending.lock().unwrap() = vec![
        ("001".into(), pending_offer("192.168.31.9", "甲.bin", 2048)),
        ("002".into(), pending_offer("192.168.31.8", "乙.bin", 4096)),
    ];

    let (lines, _) = handle_line(&host, "PENDING");
    assert_eq!(lines[0], "OK 2", "首行是数量");
    assert_eq!(lines.len(), 3, "数量 + 两行数据");
    let (token, peer, name, size, fingerprint, code) = parse_pending_line(&lines[1]).unwrap();
    assert_eq!(
        (token.as_str(), peer.as_str(), name.as_str(), size),
        ("001", "192.168.31.9", "甲.bin", 2048)
    );
    // 未配对设备必须把指纹与校验码带到客户端，否则用户无从核对
    assert_eq!(fingerprint, "abcdef0123456789");
    assert_eq!(code, "482913");
    assert_eq!(parse_pending_line(&lines[2]).unwrap().0, "002");
}

#[test]
fn pending_empty_is_count_zero() {
    let host = FakeHost::default();
    let (lines, _) = handle_line(&host, "PENDING");
    assert_eq!(lines, vec!["OK 0".to_string()]);
}

#[test]
fn accept_with_token_decides_exactly_that_request() {
    let host = FakeHost::default();
    *host.pending.lock().unwrap() = vec![
        ("001".into(), pending_offer("10.0.0.1", "a.bin", 1)),
        ("002".into(), pending_offer("10.0.0.2", "b.bin", 2)),
    ];

    let (ok, _) = handle_line(&host, "ACCEPT 002");
    assert_eq!(ok[0], "OK 已接受 #002", "响应要回显编号，用户能确认点对了");
    assert_eq!(*host.decided.lock().unwrap(), vec![("002".to_string(), true)]);
    assert_eq!(host.pending().len(), 1, "只应消掉 002");
    assert_eq!(host.pending()[0].0, "001");
}

#[test]
fn reject_with_token_is_recorded() {
    let host = FakeHost::default();
    host.pending
        .lock()
        .unwrap()
        .push(("007".into(), pending_offer("10.0.0.9", "c.bin", 3)));

    let (ok, _) = handle_line(&host, "REJECT 007");
    assert!(ok[0].starts_with("OK 已拒绝 #007"), "实际：{}", ok[0]);
    assert_eq!(*host.decided.lock().unwrap(), vec![("007".to_string(), false)]);
}

#[test]
fn accept_without_token_refuses_when_multiple_pending() {
    let host = FakeHost::default();
    *host.pending.lock().unwrap() = vec![
        ("001".into(), pending_offer("10.0.0.1", "a.bin", 1)),
        ("002".into(), pending_offer("10.0.0.2", "b.bin", 2)),
    ];
    let (lines, _) = handle_line(&host, "ACCEPT");
    assert!(lines[0].starts_with("ERR "), "多个待确认时必须要求编号：{}", lines[0]);
    assert!(host.decided.lock().unwrap().is_empty());
}

#[test]
fn accept_with_unknown_token_is_an_error() {
    let host = FakeHost::default();
    host.pending
        .lock()
        .unwrap()
        .push(("001".into(), pending_offer("10.0.0.1", "a.bin", 1)));
    let (lines, _) = handle_line(&host, "ACCEPT 042");
    assert!(lines[0].starts_with("ERR "), "实际：{}", lines[0]);
    assert_eq!(host.pending().len(), 1, "错编号不该动到任何槽位");
}

#[test]
fn decide_without_pending_is_refused() {
    let host = FakeHost::default();
    let (lines, _) = handle_line(&host, "ACCEPT");
    assert!(lines[0].starts_with("ERR "), "没有待确认请求时不能假装成功");
}

#[test]
fn stop_closes_connection() {
    let host = FakeHost::default();
    let (lines, keep_open) = handle_line(&host, "STOP");
    assert!(lines[0].starts_with("OK "));
    assert!(!keep_open, "STOP 之后应关闭连接");
    assert!(host.stopped.load(Ordering::SeqCst));
}

#[test]
fn unknown_command_is_an_error_but_keeps_connection() {
    let host = FakeHost::default();
    let (lines, keep_open) = handle_line(&host, "FLY");
    assert!(lines[0].starts_with("ERR "));
    assert!(keep_open);
}

// ---------------------------------------------------------------- 回环真实收发

#[test]
fn client_and_server_talk_over_loopback() {
    let host = Arc::new(FakeHost::default());
    host.devices
        .lock()
        .unwrap()
        .push(a_device("甲机", "192.168.31.37:53500".parse().unwrap()));
    host.pending
        .lock()
        .unwrap()
        .push(("001".into(), pending_offer("192.168.31.9", "报告.pdf", 2048)));

    let (listener, port) = bind_ipc(0).expect("应能绑到一个回环端口");
    let server_host = Arc::clone(&host);
    let server = std::thread::spawn(move || {
        for stream in listener.incoming().take(1) {
            serve_connection(server_host.as_ref(), stream.unwrap());
        }
    });

    let mut state = sample_state();
    state.ipc_port = port;
    let mut client = IpcClient::connect(&state).expect("应能连上守护进程");
    // 一条连接连发多条指令：验证响应边界（END）没串行错位
    assert!(client.call_first("PING").unwrap().starts_with("OK 0.3.0"));
    assert_eq!(client.call("STATUS").unwrap()[0].split(' ').nth(2), Some("1"));
    assert_eq!(client.call("DEVICES").unwrap().len(), 2);
    assert_eq!(client.call("PENDING").unwrap().len(), 2);

    let reply = client.call_first("ACCEPT 001").unwrap();
    assert!(reply.contains("#001"), "实际：{reply}");
    assert_eq!(host.decided.lock().unwrap().len(), 1);

    // 错误响应要能被客户端识别成错误
    let err = client.call_first("RESOLVE 幽灵机").unwrap_err().to_string();
    assert!(err.contains("幽灵机"), "实际：{err}");

    // 必须先关客户端：服务端是「读到对端关闭才退出」的循环，
    // 否则 join 会永远等下去（CI 上就是这么挂住 30 分钟的）。
    drop(client);
    server.join().unwrap();
}

#[test]
fn bind_ipc_shifts_when_port_is_taken() {
    let (first, port) = bind_ipc(0).unwrap();
    // 再绑同一个端口必然失败，应自动顺延（并行测试可能已占住 port+1，所以只要求更大）
    let (_second, next) = bind_ipc(port).unwrap();
    assert!(next > port, "应顺延到更大的端口，实际 {port} → {next}");
    drop(first);
}

#[test]
fn client_fails_fast_when_nothing_listens() {
    // 找一个没人听的端口
    let (listener, port) = bind_ipc(0).unwrap();
    drop(listener);
    let mut state = sample_state();
    state.ipc_port = port;
    assert!(IpcClient::connect(&state).is_err(), "没人监听时必须报错而不是挂住");
}
