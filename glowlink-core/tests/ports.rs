//! `model/ports.rs` 的测试：端口可用性判定与候选端口序列。

use std::net::UdpSocket;

use glowlink_core::model::ports::{
    candidate_at, candidate_ports, is_port_conflict, probe_udp, BindOutcome, DISCOVERY_PORT_SPAN,
};

#[test]
fn probe_reports_bound_for_free_port() {
    // 必须占用 **0.0.0.0** 而不是 127.0.0.1：`probe_udp` 绑的是 0.0.0.0，
    // Windows 允许「0.0.0.0:P」与「127.0.0.1:P」共存，拿回环地址当占位会得出「端口空闲」的假结论。
    let socket = UdpSocket::bind(("0.0.0.0", 0)).unwrap();
    let port = socket.local_addr().unwrap().port();
    // 该端口此刻被自己持有，探测必须报「被占」（说明 probe 用的是真 bind 结果）
    assert_eq!(probe_udp(port), BindOutcome::Taken);
    drop(socket);
}

#[test]
fn probe_reports_bound_after_release() {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    // 释放后应当可用；万一被别的进程抢走则跳过（不制造假故障）
    match probe_udp(port) {
        BindOutcome::Bound(p) => assert_eq!(p, port),
        BindOutcome::Taken => eprintln!("端口 {port} 已被其它进程占用，跳过本次断言"),
    }
}

#[test]
fn port_conflicts_are_recognised() {
    let error = std::io::Error::from(std::io::ErrorKind::AddrInUse);
    assert!(is_port_conflict(&error));
    let other = std::io::Error::from(std::io::ErrorKind::UnexpectedEof);
    assert!(!is_port_conflict(&other));
}

/// 候选端口必须从约定端口开始、连续覆盖整个跨度。
#[test]
fn candidates_cover_the_whole_span() {
    let ports = candidate_ports(9267, 21);
    assert_eq!(ports.len(), 21);
    assert_eq!(ports.first(), Some(&9267));
    assert_eq!(ports.last(), Some(&9287));
    assert_eq!(candidate_at(9267, 20), Some(9287));
    assert_eq!(candidate_at(9267, 21), Some(9288));
}

/// 越界不回绕：宁可少几个候选，也不要跳回低端口区。
#[test]
fn candidates_do_not_wrap_around_the_port_space() {
    let ports = candidate_ports(65530, 21);
    assert_eq!(ports, vec![65530, 65531, 65532, 65533, 65534, 65535]);
    assert_eq!(candidate_at(65535, 1), None);
    // 跨度 0 视为 1，保证至少探测约定端口本身
    assert_eq!(candidate_ports(9267, 0), vec![9267]);
    assert_eq!(candidate_ports(0, 2), vec![1, 2], "端口 0 不可用，从 1 起算");
}

/// 协议常量本身必须是合理值：太小会导致顺延即失联，太大等于刷屏。
#[test]
fn span_constant_is_sane() {
    assert!(
        (10..=64).contains(&DISCOVERY_PORT_SPAN),
        "跨度 {DISCOVERY_PORT_SPAN} 超出合理范围（广播流量与容错能力的折中）"
    );
}
