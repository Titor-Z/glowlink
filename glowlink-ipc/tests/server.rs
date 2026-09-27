//! `glowlink-ipc/src/server.rs` 的单元测试：协议分支 + 监听端口顺延。

mod common;

use common::{a_device, pairing_row, pending_row, FakeHost};

use glowlink_ipc::row::PairingState;

use glowlink_ipc::server::{bind_ipc, handle_line};

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
        .push(pending_row("001", "10.0.0.1", "a.bin", 1));
    let (lines, _) = handle_line(&host, "STATUS");
    assert!(lines[0].starts_with("OK 0 1 "), "待确认数应出现在字段里：{}", lines[0]);
}

#[test]
fn devices_lists_rows_with_tabs() {
    let addr = "192.168.31.37:53500".parse().unwrap();
    let host = FakeHost::default();
    host.devices.lock().unwrap().push(a_device("甲机", addr));

    let (lines, _) = handle_line(&host, "DEVICES");
    assert_eq!(lines[0], "OK 1");
    let row = glowlink_ipc::DeviceRow::from_line(&lines[1]).expect("数据行要能被客户端解析");
    assert_eq!(row.name, "甲机");
    assert_eq!(row.addr, addr);
    assert!(!row.trusted, "替身没把它列入信任表，那一列应是 0：{}", lines[1]);
}

#[test]
fn devices_marks_trusted_ids_from_the_daemon() {
    // 信任列表住在守护进程里（配对成功就是它写的一笔），
    // 界面不该自己去读那个文件——两处各读一份就会不一致。
    let host = FakeHost::default();
    host.devices.lock().unwrap().push(a_device("甲机", "127.0.0.1:53500".parse().unwrap()));
    host.trusted.lock().unwrap().push("abc123def4567890".to_string());

    let (lines, _) = handle_line(&host, "DEVICES");
    let row = glowlink_ipc::DeviceRow::from_line(&lines[1]).unwrap();
    assert!(row.trusted, "信任列表里的 ID 要标出来：{}", lines[1]);
}

#[test]
fn devices_without_matches_is_empty_not_error() {
    let host = FakeHost::default();
    let (lines, _) = handle_line(&host, "DEVICES");
    assert_eq!(lines, vec!["OK 0".to_string()]);
}

#[test]
fn resolve_finds_by_name_and_id_rejects_unknown() {
    let host = FakeHost::default();
    host.devices
        .lock()
        .unwrap()
        .push(a_device("乙机", "127.0.0.1:53000".parse().unwrap()));

    let (ok, _) = handle_line(&host, "RESOLVE 乙机");
    assert_eq!(ok[0], "OK 乙机\tabc123def4567890\t127.0.0.1:53000");

    let (by_id, _) = handle_line(&host, "RESOLVE abc123def4567890");
    assert!(by_id[0].starts_with("OK 乙机"), "按 ID 也要能查：{}", by_id[0]);

    let (err, _) = handle_line(&host, "RESOLVE 幽灵机");
    assert!(err[0].starts_with("ERR "), "实际：{}", err[0]);
}

#[test]
fn pending_returns_a_list_with_tokens() {
    let host = FakeHost::default();
    *host.pending.lock().unwrap() = vec![
        pending_row("001", "192.168.31.9", "甲.bin", 2048),
        pending_row("002", "192.168.31.8", "乙.bin", 4096),
    ];

    let (lines, _) = handle_line(&host, "PENDING");
    assert_eq!(lines[0], "OK 2", "首行是数量");
    assert_eq!(lines.len(), 3, "数量 + 两行数据");
    let row = glowlink_ipc::PendingRow::from_line(&lines[1]).unwrap();
    assert_eq!(
        (row.token.as_str(), row.peer.as_str(), row.name.as_str(), row.size),
        ("001", "192.168.31.9", "甲.bin", 2048)
    );
    // 未配对设备必须把指纹与校验码带到客户端，否则用户无从核对
    assert_eq!(row.fingerprint, "abcdef0123456789");
    assert_eq!(row.code, "482913");
    assert_eq!(glowlink_ipc::PendingRow::from_line(&lines[2]).unwrap().token, "002");
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
        pending_row("001", "10.0.0.1", "a.bin", 1),
        pending_row("002", "10.0.0.2", "b.bin", 2),
    ];

    let (ok, _) = handle_line(&host, "ACCEPT 002");
    assert_eq!(ok[0], "OK 已接受 #002", "响应要回显编号，用户能确认点对了");
    assert_eq!(*host.decided.lock().unwrap(), vec![("002".to_string(), true)]);
    assert_eq!(host.pending.lock().unwrap().len(), 1, "只应消掉 002");
    assert_eq!(host.pending.lock().unwrap()[0].token, "001");
}

#[test]
fn reject_with_token_is_recorded() {
    let host = FakeHost::default();
    host.pending
        .lock()
        .unwrap()
        .push(pending_row("007", "10.0.0.9", "c.bin", 3));

    let (ok, _) = handle_line(&host, "REJECT 007");
    assert!(ok[0].starts_with("OK 已拒绝 #007"), "实际：{}", ok[0]);
    assert_eq!(*host.decided.lock().unwrap(), vec![("007".to_string(), false)]);
}

#[test]
fn accept_without_token_refuses_when_multiple_pending() {
    let host = FakeHost::default();
    *host.pending.lock().unwrap() = vec![
        pending_row("001", "10.0.0.1", "a.bin", 1),
        pending_row("002", "10.0.0.2", "b.bin", 2),
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
        .push(pending_row("001", "10.0.0.1", "a.bin", 1));
    let (lines, _) = handle_line(&host, "ACCEPT 042");
    assert!(lines[0].starts_with("ERR "), "实际：{}", lines[0]);
    assert_eq!(host.pending.lock().unwrap().len(), 1, "错编号不该动到任何槽位");
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
    assert!(host.stopped.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn unknown_command_is_an_error_but_keeps_connection() {
    let host = FakeHost::default();
    let (lines, keep_open) = handle_line(&host, "FLY");
    assert!(lines[0].starts_with("ERR "));
    assert!(keep_open);
}

#[test]
fn bind_ipc_shifts_when_port_is_taken() {
    let (first, port) = bind_ipc(0).unwrap();
    // 再绑同一个端口必然失败，应自动顺延（并行测试可能已占住 port+1，所以只要求更大）
    let (_second, next) = bind_ipc(port).unwrap();
    assert!(next > port, "应顺延到更大的端口，实际 {port} → {next}");
    drop(first);
}

// ---------------------------------------------------------------- 配对

#[test]
fn pair_starts_a_session_and_returns_the_code_at_once() {
    // 码必须**立刻**回给调用方：界面要在第一时间把它显示给人核对，
    // 不能等配对结束才回——那时人早忘了自己看到的是什么。
    let host = FakeHost::default();
    let (lines, keep_open) = handle_line(&host, "PAIR 甲机");
    assert_eq!(lines[0], "OK 001 482913");
    assert!(keep_open);
}

#[test]
fn pair_with_unknown_device_returns_a_typed_error() {
    let host = FakeHost::default();
    let (lines, _) = handle_line(&host, "PAIR 幽灵机");
    assert!(lines[0].starts_with("ERR notfound "), "真码：{}", lines[0]);
}

#[test]
fn pairings_lists_rows_with_state() {
    let host = FakeHost::default();
    *host.pairings.lock().unwrap() = vec![
        pairing_row("001", "甲机", PairingState::WaitingPeer, 0),
        pairing_row("002", "乙机", PairingState::Paired, 0),
    ];
    let (lines, _) = handle_line(&host, "PAIRINGS");
    assert_eq!(lines[0], "OK 2");
    let row = glowlink_ipc::PairingRow::from_line(&lines[1]).unwrap();
    assert_eq!((row.peer.as_str(), row.state, row.code.as_str()), ("甲机", PairingState::WaitingPeer, "482913"));
    assert_eq!(glowlink_ipc::PairingRow::from_line(&lines[2]).unwrap().state, PairingState::Paired);
}

#[test]
fn pair_pending_lists_only_waiting_requests() {
    let host = FakeHost::default();
    *host.pair_pending.lock().unwrap() = vec![pairing_row("003", "丙机", PairingState::WaitingUser, 3)];
    let (lines, _) = handle_line(&host, "PAIRPENDING");
    assert_eq!(lines[0], "OK 1");
    let row = glowlink_ipc::PairingRow::from_line(&lines[1]).unwrap();
    assert_eq!(row.state, PairingState::WaitingUser);
    assert_eq!(row.remaining, 3, "界面要能显示「还剩几次」");
}

#[test]
fn pair_accept_requires_both_token_and_code() {
    let host = FakeHost::default();
    *host.pair_pending.lock().unwrap() = vec![pairing_row("003", "丙机", PairingState::WaitingUser, 3)];

    // 光给编号不行：码必须由人输进来
    let (lines, _) = handle_line(&host, "PAIR_ACCEPT 003");
    assert!(lines[0].starts_with("ERR usage "), "实际：{}", lines[0]);
    assert!(host.pair_decisions.lock().unwrap().is_empty(), "缺码时不能落决定");

    let (ok, _) = handle_line(&host, "PAIR_ACCEPT 003 482913");
    assert_eq!(ok[0], "OK 已接受 #003");
    assert_eq!(
        *host.pair_decisions.lock().unwrap(),
        vec![("003".to_string(), Some("482913".to_string()))]
    );
}

#[test]
fn pair_reject_carries_no_code() {
    let host = FakeHost::default();
    *host.pair_pending.lock().unwrap() = vec![pairing_row("003", "丙机", PairingState::WaitingUser, 3)];
    let (ok, _) = handle_line(&host, "PAIR_REJECT 003");
    assert!(ok[0].starts_with("OK 已拒绝 #003"), "实际：{}", ok[0]);
    assert_eq!(*host.pair_decisions.lock().unwrap(), vec![("003".to_string(), None)]);
}

#[test]
fn pair_decide_with_unknown_token_is_refused() {
    let host = FakeHost::default();
    let (lines, _) = handle_line(&host, "PAIR_ACCEPT 999 000000");
    assert!(lines[0].starts_with("ERR usage "), "实际：{}", lines[0]);
    assert!(host.pair_decisions.lock().unwrap().is_empty());
}
