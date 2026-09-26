//! `glowlink-cli/src/daemon.rs` 的单元测试：状态文件、IPC 协议、待确认槽。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glowlink_cli::daemon::{
    handle_line, parse_device_line, parse_resolve, PendingOffer, PendingSlot, State, IpcHost,
};
use glowlink_core::{Device, DeviceId};

// ---------------------------------------------------------------- 状态文件

fn sample_state() -> State {
    State {
        version: "0.2.0".to_string(),
        pid: 4242,
        ipc_port: 10_000,
        transfer_port: 51_234,
        discovery_port: 9267,
        receive_dir: "/tmp/接收 目录".into(),
        started_at: 1_700_000_000,
    }
}

#[test]
fn state_survives_json_roundtrip() {
    let state = sample_state();
    let parsed = State::parse(&state.to_json()).unwrap();
    assert_eq!(parsed, state, "状态文件写出去要能原样读回来");
}

#[test]
fn state_escapes_quotes_and_backslashes() {
    let mut state = sample_state();
    state.receive_dir = r#"/tmp/带"引号"\与反斜杠"#.into();
    let parsed = State::parse(&state.to_json()).unwrap();
    assert_eq!(parsed.receive_dir, state.receive_dir);
}

#[test]
fn state_parse_rejects_corrupt_file() {
    // 缺字段：必须是错误，不能默默补默认值让客户端连到错误的端口
    let missing = State::parse(r#"{"version":"0.2.0","pid":1}"#);
    assert!(missing.is_err(), "缺字段应当报错");
    assert!(State::parse("完全不是 JSON").is_err());
}

// ---------------------------------------------------------------- IPC 协议

/// 测试替身：把守护进程状态塞进内存，覆盖协议各个分支。
#[derive(Default)]
struct FakeHost {
    devices: Mutex<Vec<Device>>,
    pending: Mutex<Option<PendingOffer>>,
    decided: Mutex<Vec<bool>>,
    stopped: AtomicBool,
}

impl FakeHost {
    fn with_device(peer: &str, addr: SocketAddr) -> Self {
        let host = FakeHost::default();
        host.devices.lock().unwrap().push(Device {
            id: DeviceId::new("abc123def4567890"),
            name: peer.to_string(),
            addr,
            last_seen: Instant::now(),
        });
        host
    }
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

    fn pending(&self) -> Option<PendingOffer> {
        self.pending.lock().unwrap().clone()
    }

    fn decide(&self, accept: bool) -> Result<(), glowlink_core::GlowError> {
        if self.pending.lock().unwrap().is_none() {
            return Err(glowlink_core::GlowError::Usage("当前没有待确认的接收请求".into()));
        }
        self.decided.lock().unwrap().push(accept);
        *self.pending.lock().unwrap() = None;
        Ok(())
    }

    fn request_stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

#[test]
fn ping_reports_version_pid_and_ports() {
    let host = FakeHost::default();
    let (lines, keep_open) = handle_line(&host, "PING");
    assert!(lines[0].starts_with("OK 0.2.0 4242 51234 9267"), "实际：{}", lines[0]);
    assert!(keep_open, "PING 之后连接应可复用");
}

#[test]
fn devices_lists_rows_with_tabs() {
    let addr: SocketAddr = "192.168.31.37:53500".parse().unwrap();
    let host = FakeHost::with_device("甲机", addr);
    let (lines, _) = handle_line(&host, "DEVICES");
    assert_eq!(lines[0], "OK 1");
    let parsed = parse_device_line(&lines[1]).expect("数据行要能被客户端解析");
    assert_eq!(parsed.0, "甲机");
    assert_eq!(parsed.2, addr);
}

#[test]
fn devices_without_matches_is_empty_not_error() {
    let host = FakeHost::default();
    let (lines, _) = handle_line(&host, "DEVICES");
    assert_eq!(lines, vec!["OK 0".to_string()]);
}

#[test]
fn resolve_finds_by_name_and_rejects_unknown() {
    let addr: SocketAddr = "127.0.0.1:53000".parse().unwrap();
    let host = FakeHost::with_device("乙机", addr);

    let (ok, _) = handle_line(&host, "RESOLVE 乙机");
    let (name, id, parsed) = parse_resolve(&ok[0]).unwrap();
    assert_eq!((name.as_str(), id.as_str(), parsed), ("乙机", "abc123def4567890", addr));

    // 按 ID 也要能找到
    let (by_id, _) = handle_line(&host, "RESOLVE abc123def4567890");
    assert!(parse_resolve(&by_id[0]).is_ok());

    let (err, _) = handle_line(&host, "RESOLVE 不存在的设备");
    assert!(err[0].starts_with("ERR "), "未知设备应回 ERR，实际：{}", err[0]);
}

#[test]
fn decide_without_pending_is_refused() {
    let host = FakeHost::default();
    let (lines, _) = handle_line(&host, "ACCEPT");
    assert!(lines[0].starts_with("ERR "), "没有待确认请求时不能假装成功");
    assert!(host.decided.lock().unwrap().is_empty());
}

#[test]
fn pending_then_accept_delivers_decision() {
    let host = FakeHost::default();
    *host.pending.lock().unwrap() = Some(PendingOffer {
        peer: "192.168.31.9".into(),
        name: "报告.pdf".into(),
        size: 2048,
    });

    let (lines, _) = handle_line(&host, "PENDING");
    assert_eq!(lines[0], "OK 1");
    assert_eq!(lines[1], "192.168.31.9\t报告.pdf\t2048");

    let (ok, _) = handle_line(&host, "ACCEPT");
    assert!(ok[0].starts_with("OK "), "实际：{}", ok[0]);
    assert_eq!(*host.decided.lock().unwrap(), vec![true]);
    assert!(host.pending().is_none(), "落决定后应清空待确认槽");
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
fn unknown_command_is_an_error_not_a_silent_success() {
    let host = FakeHost::default();
    let (lines, keep_open) = handle_line(&host, "FLY");
    assert!(lines[0].starts_with("ERR "));
    assert!(keep_open, "未知指令不该顺带断开连接");
}

// ---------------------------------------------------------------- 待确认槽

#[test]
fn ask_blocks_until_decision_arrives() {
    let slot = Arc::new(PendingSlot::new());
    let waiter = Arc::clone(&slot);
    let handle = std::thread::spawn(move || {
        waiter.ask(
            PendingOffer { peer: "10.0.0.1".into(), name: "a.bin".into(), size: 7 },
            Duration::from_secs(5),
        )
    });

    // 等请求登记上来，再从另一个线程落决定
    let deadline = Instant::now() + Duration::from_secs(2);
    while slot.current().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(slot.current().is_some(), "ask 应当登记待确认请求");
    slot.decide(true).unwrap();

    assert!(handle.join().unwrap(), "接受后 ask 应返回 true");
    assert!(slot.current().is_none(), "结束后槽位应清空");
}

#[test]
fn ask_times_out_as_reject() {
    let slot = PendingSlot::new();
    let accepted = slot.ask(
        PendingOffer { peer: "10.0.0.1".into(), name: "a.bin".into(), size: 7 },
        Duration::from_millis(80),
    );
    // 没人管的时候宁可什么都不收
    assert!(!accepted, "超时必须按拒绝处理");
    assert!(slot.current().is_none());
}

#[test]
fn second_ask_is_not_answered_by_a_stale_decision() {
    let slot = Arc::new(PendingSlot::new());
    let offer = PendingOffer { peer: "10.0.0.1".into(), name: "a.bin".into(), size: 7 };

    // 第一次：超时拒绝
    assert!(!slot.ask(offer.clone(), Duration::from_millis(50)));
    // 第二次：必须重新等待，而不是复用上一轮的结论
    assert!(!slot.ask(offer, Duration::from_millis(50)));
    assert!(slot.current().is_none());
}
