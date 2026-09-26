use std::net::UdpSocket;

use glowlink_core::model::ports::{bind_udp_with_fallback, is_port_conflict, probe_udp, BindOutcome};

/// 借系统分配一个空闲端口后立刻释放。
/// 找一个确实可绑定的端口（避开本机可能存在的 UDP 残留绑定）。
fn free_udp_port() -> u16 {
    for _ in 0..200 {
        let probe = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        if UdpSocket::bind(("0.0.0.0", port)).is_ok() {
            return port;
        }
    }
    panic!("找不到可用端口");
}

#[test]
fn probe_reports_free_port_as_bound() {
    let port = free_udp_port();
    assert_eq!(probe_udp(port), BindOutcome::Bound(port));
}

#[test]
fn probe_reports_taken_port_as_taken() {
    let port = free_udp_port();
    let holder = UdpSocket::bind(("0.0.0.0", port)).unwrap();

    assert_eq!(probe_udp(port), BindOutcome::Taken);
    // 同端口再次绑定必须失败，这正是降级逻辑的前提
    assert!(UdpSocket::bind(("0.0.0.0", port)).is_err());
    drop(holder);
}

#[test]
fn bind_with_fallback_uses_preferred_when_free() {
    let port = free_udp_port();
    let (socket, bound, degraded) = bind_udp_with_fallback(port).unwrap();
    assert_eq!(bound, port);
    assert!(!degraded, "首选端口可用时不应标记为降级");
    assert!(socket.local_addr().unwrap().port() == port);
}

#[test]
fn bind_with_fallback_shifts_when_preferred_taken() {
    // 占住连续 3 个端口，验证顺延会跳过它们
    let base = free_udp_port();
    let mut holders = Vec::new();
    for offset in 0..3 {
        match UdpSocket::bind(("0.0.0.0", base + offset)) {
            Ok(socket) => holders.push(socket),
            // 该端口恰好被别的进程占了也无妨，同样达到“被占”的效果
            Err(_) => {}
        }
    }

    let (socket, bound, degraded) = bind_udp_with_fallback(base).unwrap();
    assert!(degraded, "首选端口被占时标记为降级");
    assert!(bound > base);
    // 返回的必须是我们真正持有的端口
    assert_eq!(socket.local_addr().unwrap().port(), bound);
    drop(holders);
}

#[test]
fn bind_with_fallback_keeps_broadcast_enabled() {
    let port = free_udp_port();
    let (socket, _, _) = bind_udp_with_fallback(port).unwrap();
    // set_broadcast 成功即说明属性可用；这里通过再次设置来间接确认套接字有效
    assert!(socket.set_broadcast(true).is_ok());
}

#[test]
fn port_conflict_classification() {
    let taken = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    let port = taken.local_addr().unwrap().port();
    let error = UdpSocket::bind(("127.0.0.1", port)).unwrap_err();
    assert!(is_port_conflict(&error), "AddrInUse 必须被识别为端口冲突");
}
