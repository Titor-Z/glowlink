use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use glowlink_core::controller::discovery::{
    DiscoveryController, DiscoveryEvent, DiscoveryTransport,
};
use glowlink_core::model::error::Result;
use glowlink_core::model::packet::Hello;
use glowlink_core::patterns::singleton::DeviceIdentity;

/// 发现通道的测试替身：记录发出的查询与应答，并返回预设的收包队列。
#[derive(Default)]
struct FakePeer {
    queries: Mutex<Vec<Hello>>,
    replies: Mutex<Vec<(Hello, SocketAddr)>>,
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
}

/// 适配器：把共享的测试替身接到控制器需要的接口上。
struct FakeTransport(Arc<FakePeer>);

impl DiscoveryTransport for FakeTransport {
    fn local_port(&self) -> Result<u16> {
        Ok(8888)
    }

    fn query(&self, hello: &Hello) -> Result<()> {
        self.0.queries.lock().unwrap().push(hello.clone());
        Ok(())
    }

    fn reply(&self, hello: &Hello, target: SocketAddr) -> Result<()> {
        self.0.replies.lock().unwrap().push((hello.clone(), target));
        Ok(())
    }

    fn poll(&self, _wait: Duration) -> Result<Vec<DiscoveryEvent>> {
        Ok(std::mem::take(&mut *self.0.inbox.lock().unwrap()))
    }
}

fn hello(id: &str, name: &str, port: u16) -> Hello {
    Hello { id: id.into(), name: name.into(), port }
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
fn replies_unicast_to_query_source_and_records_peer() {
    let peer = Arc::new(FakePeer::default());
    peer.push_inbox(hello("peer-1", "客厅的 Mac", 4000));

    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_secs(60));
    assert_eq!(discovery.tick().unwrap(), 1, "查询方应被登记为在线设备");

    let replies = peer.replies();
    assert_eq!(replies.len(), 1, "收到查询必须单播应答");
    assert_eq!(replies[0].0.id, "self-id");
    assert_eq!(replies[0].0.port, 51234);
    assert_eq!(replies[0].1, FakePeer::peer_addr(), "应答目标应是查询来源地址");
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
