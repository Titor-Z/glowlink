use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use glowlink_core::controller::discovery::{
    DiscoveryController, DiscoveryEvent, DiscoveryTransport,
};
use glowlink_core::model::error::Result;
use glowlink_core::model::packet::Hello;
use glowlink_core::model::packet::DeviceKind;
use glowlink_core::model::ports::DISCOVERY_PORT_SPAN;
use glowlink_core::model::probe::{PortDecision, PortOwner};
use glowlink_core::patterns::adapter::UdpDiscoveryAdapter;
use glowlink_core::patterns::singleton::DeviceIdentity;

/// 发现通道的测试替身：记录发出的查询与应答，并返回预设的收包队列。
#[derive(Default)]
struct FakePeer {
    queries: Mutex<Vec<Hello>>,
    replies: Mutex<Vec<(Hello, SocketAddr)>>,
    probe_replies: Mutex<Vec<(glowlink_core::Probe, SocketAddr)>>,
    inbox: Mutex<Vec<DiscoveryEvent>>,
}

impl FakePeer {
    fn peer_addr() -> SocketAddr {
        "192.168.1.9:4000".parse().unwrap()
    }

    fn push_inbox(&self, hello: Hello) {
        self.inbox
            .lock()
            .unwrap()
            .push(DiscoveryEvent::Query { hello, from: Self::peer_addr() });
    }

    fn push_reply(&self, hello: Hello) {
        self.inbox
            .lock()
            .unwrap()
            .push(DiscoveryEvent::Reply { hello, from: Self::peer_addr() });
    }

    fn queries(&self) -> Vec<Hello> {
        self.queries.lock().unwrap().clone()
    }

    fn replies(&self) -> Vec<(Hello, SocketAddr)> {
        self.replies.lock().unwrap().clone()
    }

    fn probe_replies(&self) -> Vec<(glowlink_core::Probe, SocketAddr)> {
        self.probe_replies.lock().unwrap().clone()
    }
}

/// 适配器：把共享的测试替身接到控制器需要的接口上。
struct FakeTransport(Arc<FakePeer>);

impl DiscoveryTransport for FakeTransport {
    fn local_port(&self) -> Option<u16> {
        Some(9267)
    }

    fn query(&self, hello: &Hello) -> Result<()> {
        self.0.queries.lock().unwrap().push(hello.clone());
        Ok(())
    }

    fn reply(&self, hello: &Hello, target: SocketAddr) -> Result<()> {
        self.0.replies.lock().unwrap().push((hello.clone(), target));
        Ok(())
    }

    fn reply_probe(&self, probe: &glowlink_core::Probe, target: SocketAddr) -> Result<()> {
        self.0.probe_replies.lock().unwrap().push((probe.clone(), target));
        Ok(())
    }

    fn poll(&self, _wait: Duration) -> Result<Vec<DiscoveryEvent>> {
        Ok(std::mem::take(&mut *self.0.inbox.lock().unwrap()))
    }
}

fn hello(id: &str, name: &str, port: u16) -> Hello {
    Hello {
        id: id.into(),
        name: name.into(),
        port,
        reply_port: 9267,
        kind: DeviceKind::Desktop,
    }
}

fn controller(peer: Arc<FakePeer>, id: &str, query_every: Duration) -> DiscoveryController {
    DiscoveryController::new(
        Box::new(FakeTransport(peer)),
        DeviceIdentity::new(id, "本机"),
        51234,
        Duration::from_millis(60),
        query_every,
    )
}

#[test]
fn queries_own_identity_with_transfer_port() {
    let peer = Arc::new(FakePeer::default());
    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_secs(60));

    discovery.tick().unwrap();
    discovery.tick().unwrap();

    let queries = peer.queries();
    assert_eq!(queries.len(), 1, "查询周期未到不应重复查询");
    assert_eq!(queries[0].id, "self-id");
    assert_eq!(queries[0].name, "本机");
    assert_eq!(queries[0].port, 51234, "查询必须携带传输端口");
}

#[test]
fn queries_on_every_tick_when_period_is_zero() {
    let peer = Arc::new(FakePeer::default());
    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_millis(0));

    discovery.tick().unwrap();
    discovery.tick().unwrap();

    assert_eq!(peer.queries().len(), 2);
}

#[test]
fn replies_unicast_to_declared_reply_port() {
    let peer = Arc::new(FakePeer::default());
    peer.push_inbox(hello("peer-1", "客厅的 Mac", 4000));

    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_secs(60));
    assert_eq!(discovery.tick().unwrap(), 1, "查询方应被登记为在线设备");

    let replies = peer.replies();
    assert_eq!(replies.len(), 1, "收到查询必须单播应答");
    assert_eq!(replies[0].0.id, "self-id");
    assert_eq!(replies[0].0.port, 51234);
    // 关键：目标是报文里声明的接收端口（9267），而不是广播时的临时源端口（4000）
    assert_eq!(
        replies[0].1,
        "192.168.1.9:9267".parse().unwrap(),
        "应答应发往对方声明的接收端口"
    );
    // 本机应答也要带上自己的接收端口，供对方回包
    assert_eq!(replies[0].0.reply_port, 9267);
}

#[test]
fn reply_falls_back_to_source_when_peer_has_no_listen_port() {
    let peer = Arc::new(FakePeer::default());
    // reply_port = 0 表示对方也是降级节点，只能回到报文的真实源地址
    let degraded_peer = Hello { reply_port: 0, ..hello("peer-2", "降级机", 4000) };
    peer.push_inbox(degraded_peer);

    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_secs(60));
    discovery.tick().unwrap();

    let replies = peer.replies();
    assert_eq!(replies[0].1, FakePeer::peer_addr(), "对方未声明端口时回退到源地址");
}

#[test]
fn reply_is_recorded_without_answering_back() {
    let peer = Arc::new(FakePeer::default());
    peer.push_reply(hello("peer-1", "书房 PC", 4001));

    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_secs(60));
    assert_eq!(discovery.tick().unwrap(), 1);
    assert!(peer.replies().is_empty(), "收到应答不应再回应答");
    assert_eq!(discovery.devices()[0].addr.port(), 4001);
}

#[test]
fn own_query_is_ignored_and_not_answered() {
    let peer = Arc::new(FakePeer::default());
    peer.push_inbox(hello("self-id", "本机", 51234));

    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_secs(60));
    assert_eq!(discovery.tick().unwrap(), 0, "自身广播不应被登记");
    assert!(peer.replies().is_empty(), "不应给自己应答");
}

#[test]
fn merges_peers_by_id() {
    let peer = Arc::new(FakePeer::default());
    peer.push_inbox(hello("peer-1", "客厅的 Mac", 4000));
    peer.push_reply(hello("peer-1", "客厅的 MacBook", 4001));
    peer.push_reply(hello("peer-2", "书房 PC", 4002));

    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_secs(60));
    assert_eq!(discovery.tick().unwrap(), 2);

    let devices = discovery.devices();
    let mac = devices.iter().find(|d| d.id.as_str() == "peer-1").unwrap();
    assert_eq!(mac.name, "客厅的 MacBook");
    assert_eq!(mac.addr.port(), 4001);
    assert_eq!(mac.addr.ip().to_string(), "192.168.1.9");
    assert!(discovery.find("书房").is_some());
}

#[test]
fn prunes_devices_that_stop_answering() {
    let peer = Arc::new(FakePeer::default());
    peer.push_reply(hello("peer-1", "客厅的 Mac", 4000));

    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_secs(60));
    assert_eq!(discovery.tick().unwrap(), 1);

    thread::sleep(Duration::from_millis(90));
    assert_eq!(discovery.tick().unwrap(), 0, "超时设备应自动下线");
    assert!(discovery.devices().is_empty());
}

#[test]
fn answers_probe_so_port_selfcheck_can_identify_us() {
    // 端口自检依赖这一点：已经在监听的节点必须能回应 PROBE，
    // 否则后来者会把自己人误判成“残留占用”。
    let peer = Arc::new(FakePeer::default());
    peer.inbox
        .lock()
        .unwrap()
        .push(DiscoveryEvent::Probe { from: FakePeer::peer_addr() });

    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_secs(60));
    discovery.tick().unwrap();

    let answers = peer.probe_replies();
    assert_eq!(answers.len(), 1, "收到 PROBE 必须回 PONG");
    assert_eq!(answers[0].0.pid, std::process::id());
    assert_eq!(answers[0].0.version, glowlink_core::model::packet::VERSION_STR);
    assert_eq!(answers[0].0.reply_port, 9267, "PONG 应带上自己的监听端口");
    assert_eq!(answers[0].1, FakePeer::peer_addr());
    // 探测不应被当成设备记录
    assert!(discovery.devices().is_empty());
}

/// 借一组「基准端口 + 两个紧邻的空闲端口」，用来模拟「约定端口被占、两个节点都顺延」。
///
/// 不用 9267 这类固定端口：本机可能真被残留进程占着，用固定值会让用例时灵时不灵。
fn shifted_pair() -> (u16, UdpSocket, UdpSocket) {
    for _ in 0..200 {
        let probe = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let base = probe.local_addr().unwrap().port();
        if base + 2 > u16::MAX {
            continue;
        }
        let Ok(first) = UdpSocket::bind(("127.0.0.1", base + 1)) else { continue };
        let Ok(second) = UdpSocket::bind(("127.0.0.1", base + 2)) else { continue };
        drop(probe);
        return (base, first, second);
    }
    panic!("找不到可用的基准端口");
}

fn shifted_adapter(base: u16, socket: UdpSocket, span: u16) -> Box<dyn DiscoveryTransport> {
    let _ = socket.set_broadcast(true);
    Box::new(
        UdpDiscoveryAdapter::from_decision(PortDecision {
            socket,
            // 实际落点已经顺延，但约定端口仍是 base
            port: 0,
            preferred: base,
            owner: PortOwner::Stale,
            shifted: true,
        })
        .unwrap()
        .with_span(span),
    )
}

/// 真实场景：约定端口被残留占用，两个节点分别顺延到 base+1 / base+2。
///
/// 修复前查询只发往约定端口 base，两个都顺延了的节点互相看不见——
/// 表现为「两台机器都开着，就是搜不到对方」。
#[test]
fn shifted_nodes_still_find_each_other() {
    let (base, socket_a, socket_b) = shifted_pair();
    let span = DISCOVERY_PORT_SPAN;
    let mut a = DiscoveryController::new(
        shifted_adapter(base, socket_a, span),
        DeviceIdentity::new("node-a", "甲机"),
        0,
        Duration::from_secs(5),
        Duration::from_millis(200),
    );
    let mut b = DiscoveryController::new(
        shifted_adapter(base, socket_b, span),
        DeviceIdentity::new("node-b", "乙机"),
        0,
        Duration::from_secs(5),
        Duration::from_millis(200),
    );

    // 交替推进：a 发查询 → b 收到并单播应答 → a 收到应答
    for _ in 0..3 {
        a.tick().unwrap();
        b.tick().unwrap();
    }

    assert!(
        a.devices().iter().any(|d| d.id.as_str() == "node-b"),
        "顺延后的节点必须仍能发现对方（甲机看不到乙机）"
    );
    assert!(
        b.devices().iter().any(|d| d.id.as_str() == "node-a"),
        "顺延后的节点必须仍能发现对方（乙机看不到甲机）"
    );
}

/// 对照实验：跨度只有 1（即「只发约定端口」）时，这套场景**必须发现不了**。
///
/// 没有这条，上面那条用例可能在别的原因下通过（例如碰巧两边落在同一个端口上）。
#[test]
fn without_the_span_shifted_nodes_miss_each_other() {
    let (base, socket_a, socket_b) = shifted_pair();
    let mut a = DiscoveryController::new(
        shifted_adapter(base, socket_a, 1),
        DeviceIdentity::new("node-a", "甲机"),
        0,
        Duration::from_secs(5),
        Duration::from_millis(200),
    );
    let mut b = DiscoveryController::new(
        shifted_adapter(base, socket_b, 1),
        DeviceIdentity::new("node-b", "乙机"),
        0,
        Duration::from_secs(5),
        Duration::from_millis(200),
    );
    for _ in 0..3 {
        a.tick().unwrap();
        b.tick().unwrap();
    }
    assert!(
        a.devices().iter().all(|d| d.id.as_str() != "node-b"),
        "跨度 1 时不该发现（这正是修复前的现象）"
    );
}
