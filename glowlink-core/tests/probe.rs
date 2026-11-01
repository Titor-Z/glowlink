use std::net::UdpSocket;
use std::thread;
use std::time::{Duration, Instant};

use glowlink_core::model::packet::{Frame, Probe};
use glowlink_core::model::probe::{answer, bind_or_identify, compare_versions, PortOwner, VersionVerdict};

/// 找一个**确实可绑定**的端口。
///
/// 不能用「系统分配一个临时端口再释放」：那个端口可能恰好落在这台机器上
/// 已存在的 UDP 残留绑定上（macOS 进程退出后偶发不释放），导致用例本身
/// 与被测逻辑无关地失败。
fn free_udp_port() -> u16 {
    for _ in 0..200 {
        let probe = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        // 立刻再绑一次确认真的可用
        if UdpSocket::bind(("0.0.0.0", port)).is_ok() {
            return port;
        }
    }
    panic!("找不到可用端口");
}

/// 端口的分配是「先问系统要一个、立刻释放」，本质是 TOCTOU：同一进程里并行跑的
/// 用例会拿到**同一个**端口，于是 A 占着的端口被 B 抢绑（CI 上实际撞到过
/// `AddrInUse`）。凡是要绑固定端口的用例，先取这把锁。
fn port_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn probe_roundtrip_carries_version_and_pid() {
    let _guard = port_guard();
    let port = free_udp_port();
    let listener = UdpSocket::bind(("0.0.0.0", port)).expect("刚验证过的端口应可绑定");
    listener.set_read_timeout(Some(Duration::from_secs(2))).unwrap();

    // 扮演“已在运行的节点”：收到 PROBE 就回 PONG
    let responder = thread::spawn(move || {
        let mut buf = vec![0u8; 1024];
        let (len, from) = listener.recv_from(&mut buf).unwrap();
        assert!(matches!(Frame::decode(&buf[..len]).unwrap(), Frame::Probe(_)));
        answer(&listener, from).unwrap();
        // 让端口保持被占，直到应答发出
        thread::sleep(Duration::from_millis(50));
        listener
    });

    let owner = glowlink_core::model::probe::identify(port).unwrap();
    match owner {
        PortOwner::Ours(p) => {
            assert_eq!(p.pid, std::process::id());
            // 版本用软件版本字符串（用于判断“运行中的是不是旧版本”），而非协议字节
            assert_eq!(p.version, glowlink_core::model::packet::VERSION_STR);
            assert_eq!(p.reply_port, port, "PONG 应带上监听端口");
        }
        other => panic!("应识别为我们自己的节点，实际：{other:?}"),
    }
    responder.join().unwrap();
}

#[test]
fn identify_reports_stale_when_nobody_answers() {
    let _guard = port_guard();
    let port = free_udp_port();
    // 占住端口但完全不收包 → 无人应答，应判为 Stale（残留绑定），不是“别的程序”
    let _holder = UdpSocket::bind(("0.0.0.0", port)).unwrap();

    let started = Instant::now();
    let owner = glowlink_core::model::probe::identify(port).unwrap();
    assert_eq!(owner, PortOwner::Stale);
    assert!(!owner.is_ours());
    assert!(!owner.is_foreign(), "无人应答不得说成被其它程序占用");
    // 不能长时间卡住：回环探测必须在几百毫秒内出结论
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "探测耗时过长：{:?}",
        started.elapsed()
    );
}

#[test]
fn identify_reports_stranger_for_foreign_response() {
    let _guard = port_guard();
    let port = free_udp_port();
    let responder = UdpSocket::bind(("0.0.0.0", port)).unwrap();
    responder.set_read_timeout(Some(Duration::from_secs(2))).unwrap();

    // 回一个不是 GLWK 的垃圾数据
    let noise = thread::spawn(move || {
        let mut buf = vec![0u8; 1024];
        let (_, from) = responder.recv_from(&mut buf).unwrap();
        responder.send_to(b"HTTP/1.0 200 OK\r\n\r\n", from).unwrap();
        thread::sleep(Duration::from_millis(50));
        responder
    });

    assert_eq!(
        glowlink_core::model::probe::identify(port).unwrap(),
        PortOwner::Stranger,
        "非法应答必须归类为别的程序"
    );
    noise.join().unwrap();
}

#[test]
fn bind_or_identify_returns_free_socket_when_available() {
    let _guard = port_guard();
    let port = free_udp_port();
    let (socket, owner) = bind_or_identify(port).unwrap();
    assert_eq!(owner, PortOwner::Free);
    let socket = socket.expect("空闲端口应返回可用套接字");
    assert_eq!(socket.local_addr().unwrap().port(), port);
}

#[test]
fn bind_or_identify_detects_own_running_node() {
    let _guard = port_guard();
    let port = free_udp_port();
    let held = UdpSocket::bind(("0.0.0.0", port)).unwrap();
    held.set_read_timeout(Some(Duration::from_secs(2))).unwrap();

    let responder = thread::spawn(move || {
        let mut buf = vec![0u8; 1024];
        let (len, from) = held.recv_from(&mut buf).unwrap();
        assert!(Frame::decode(&buf[..len]).is_ok());
        answer(&held, from).unwrap();
        thread::sleep(Duration::from_millis(50));
        held
    });

    let (socket, owner) = bind_or_identify(port).unwrap();
    assert!(socket.is_none(), "端口被占时不应返回套接字");
    assert!(owner.is_ours(), "应识别出这是我们自己的节点：{owner:?}");
    responder.join().unwrap();
}

#[test]
fn version_comparison_drives_upgrade_decision() {
    assert_eq!(compare_versions("0.1.5", "0.1.5"), VersionVerdict::Same);
    assert_eq!(compare_versions("0.2.0", "0.1.9"), VersionVerdict::RunningIsOlder);
    assert_eq!(compare_versions("0.1.9", "0.2.0"), VersionVerdict::RunningIsNewer);
    // 简写版本号按缺省 0 补齐
    assert_eq!(compare_versions("1.0", "1.0.0"), VersionVerdict::Same);
    assert_eq!(compare_versions("1", "1.0.0"), VersionVerdict::Same);
    // 解析不出的版本号保守视作相同，避免误触顶替
    assert_eq!(compare_versions("dev-build", "0.1.5"), VersionVerdict::Same);
    assert_eq!(compare_versions("", "0.1.5"), VersionVerdict::Same);
}

#[test]
fn probe_frame_roundtrip() {
    let probe = Probe {
        id: "abc123".into(),
        version: "1.2.3".into(),
        pid: 4242,
        reply_port: 19999,
    };
    for frame in [Frame::Probe(probe.clone()), Frame::Pong(probe.clone())] {
        let bytes = frame.encode().unwrap();
        assert_eq!(Frame::decode(&bytes).unwrap(), frame);
    }
}
