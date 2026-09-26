use glowlink_core::model::ports::FALLBACK_SPAN;
use glowlink_core::model::probe::{decide_port, PortOwner};

/// 找出一个在当前机器上确实“绑不上也无人应答”的端口（内核残留绑定）。
///
/// 若本机没有这种残留，则跳过相关断言——它是环境相关的，
/// 不能因为测试机干净就让用例失败。
fn find_stale_port() -> Option<u16> {
    use std::net::UdpSocket;
    for port in 19000..19999u16 {
        // 能绑上就不是残留
        if UdpSocket::bind(("0.0.0.0", port)).is_ok() {
            continue;
        }
        // 绑不上且探测无应答 → 残留
        if matches!(
            glowlink_core::model::probe::identify(port),
            Ok(PortOwner::Stale)
        ) {
            return Some(port);
        }
    }
    None
}

#[test]
fn decide_port_uses_preferred_when_free() {
    // 100 个连续空闲端口里必定有一个可用
    let mut chosen = None;
    for port in 30000..30099u16 {
        if let Ok(decision) = decide_port(port) {
            chosen = Some(decision);
            break;
        }
    }
    let decision = chosen.expect("应能在候选区段绑到一个端口");
    assert!(!decision.shifted || decision.owner != PortOwner::Free);
    // 真正持有的端口必须与报告的一致
    assert_eq!(decision.socket.local_addr().unwrap().port(), decision.port);
}

#[test]
fn decision_keeps_announced_port_separate_from_actual() {
    use glowlink_core::patterns::factory::{DiscoveryTransportFactory, NativeTransportFactory};
    use std::net::UdpSocket;

    // 占住首选端口，制造“必须顺延”的场景
    let base = {
        let probe = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        probe.local_addr().unwrap().port()
    };
    let holder = UdpSocket::bind(("0.0.0.0", base)).unwrap();

    let decision = decide_port(base).unwrap();
    assert!(decision.shifted, "首选被占时必须顺延");
    assert_eq!(decision.preferred, base);
    assert_ne!(decision.port, base);
    assert!(decision.notice().is_some(), "顺延必须给出提示语");

    // 适配器对外仍然广播到约定的首选端口
    let transport = NativeTransportFactory
        .from_bound_socket(decision)
        .expect("已决策好的端口不应再失败");
    transport
        .query(&glowlink_core::model::packet::Hello {
            id: "t".into(),
            name: "t".into(),
            port: 1,
            reply_port: 0,
        })
        .expect("顺延后的节点仍应能发出查询");
    drop(holder);
}

#[test]
fn stale_port_is_not_reported_as_foreign_program() {
    let Some(port) = find_stale_port() else {
        // 本机没有残留绑定，跳过（该断言依赖真实环境）
        eprintln!("本机没有残留绑定端口，跳过该用例");
        return;
    };
    let owner = glowlink_core::model::probe::identify(port).unwrap();
    assert!(owner.is_stale(), "无人应答的占用必须是 Stale，实际：{owner:?}");
    assert!(!owner.is_foreign(), "无人应答不得说成“被其它程序占用”");
    assert!(!owner.is_ours());

    // 顺延时给出的说法必须提到“残留”，而不是“其他程序”
    let decision = decide_port(port).unwrap();
    let notice = decision.notice().expect("残留占用必须解释一句");
    assert!(notice.contains("残留"), "提示语应说明残留：{notice}");
    assert!(!notice.contains("其他程序"), "不得误报为其他程序：{notice}");
}

#[test]
fn foreign_program_is_reported_as_stranger() {
    use std::net::UdpSocket;
    use std::thread;
    use std::time::Duration;

    // 找一个空闲端口，然后由一个“别的程序”占住并乱答
    let port = {
        let probe = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        probe.local_addr().unwrap().port()
    };
    let squatter = match UdpSocket::bind(("0.0.0.0", port)) {
        Ok(socket) => socket,
        Err(_) => return,
    };
    squatter
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let responder = thread::spawn(move || {
        let mut buf = vec![0u8; 1024];
        // 收到探测后回一个非协议内容（模拟别的程序）
        if let Ok((_, from)) = squatter.recv_from(&mut buf) {
            let _ = squatter.send_to(b"HTTP/1.1 400 Bad Request\r\n\r\n", from);
        }
        // 保持端口被占，直到断言完成
        thread::sleep(Duration::from_millis(500));
        squatter
    });

    let owner = glowlink_core::model::probe::identify(port).unwrap();
    assert!(owner.is_foreign(), "非协议应答必须判为别的程序，实际：{owner:?}");
    assert!(!owner.is_stale());
    responder.join().unwrap();
}

#[test]
fn fallback_span_is_reasonable() {
    // 顺延范围不能太小（容不下端口冲突），也不能太大（避免扫到敏感端口）
    assert!(FALLBACK_SPAN >= 10 && FALLBACK_SPAN <= 1000, "顺延范围：{FALLBACK_SPAN}");
}
