use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use glowlink_core::controller::discovery::{DiscoveryController, DiscoveryTransport};
use glowlink_core::model::error::Result;
use glowlink_core::model::packet::Hello;
use glowlink_core::patterns::singleton::DeviceIdentity;

/// 发现通道的测试替身：记录广播内容，并返回预设的收包队列。
#[derive(Default)]
struct FakePeer {
    announced: Mutex<Vec<Hello>>,
    inbox: Mutex<Vec<(Hello, IpAddr)>>,
}

impl FakePeer {
    fn push_inbox(&self, hello: Hello) {
        self.inbox.lock().unwrap().push((hello, "192.168.1.9".parse().unwrap()));
    }

    fn announced(&self) -> Vec<Hello> {
        self.announced.lock().unwrap().clone()
    }
}

/// 适配器：把共享的测试替身接到控制器需要的接口上。
struct FakeTransport(Arc<FakePeer>);

impl DiscoveryTransport for FakeTransport {
    fn announce(&self, hello: &Hello) -> Result<()> {
        self.0.announced.lock().unwrap().push(hello.clone());
        Ok(())
    }

    fn poll(&self, _wait: Duration) -> Result<Vec<(Hello, IpAddr)>> {
        Ok(std::mem::take(&mut *self.0.inbox.lock().unwrap()))
    }
}

fn hello(id: &str, name: &str, port: u16) -> Hello {
    Hello { id: id.into(), name: name.into(), port }
}

fn identity(id: &str) -> DeviceIdentity {
    DeviceIdentity::new(id, "本机")
}

fn controller(
    peer: Arc<FakePeer>,
    id: &str,
    announce: Duration,
) -> DiscoveryController {
    DiscoveryController::new(
        Box::new(FakeTransport(peer)),
        identity(id),
        51234,
        Duration::from_millis(60),
        announce,
    )
}

#[test]
fn announces_own_identity_with_transfer_port() {
    let peer = Arc::new(FakePeer::default());
    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_secs(60));

    discovery.tick().unwrap();
    discovery.tick().unwrap();

    let announced = peer.announced();
    assert_eq!(announced.len(), 1, "广播周期未到不应重复广播");
    assert_eq!(announced[0].id, "self-id");
    assert_eq!(announced[0].name, "本机");
    assert_eq!(announced[0].port, 51234);
}

#[test]
fn announces_on_every_tick_when_period_is_zero() {
    let peer = Arc::new(FakePeer::default());
    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_millis(0));

    discovery.tick().unwrap();
    discovery.tick().unwrap();

    assert_eq!(peer.announced().len(), 2);
}

#[test]
fn merges_peers_and_ignores_self() {
    let peer = Arc::new(FakePeer::default());
    peer.push_inbox(hello("self-id", "本机", 51234));
    peer.push_inbox(hello("peer-1", "客厅的 Mac", 4000));
    peer.push_inbox(hello("peer-1", "客厅的 MacBook", 4001));
    peer.push_inbox(hello("peer-2", "书房 PC", 4002));

    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_secs(60));
    assert_eq!(discovery.tick().unwrap(), 2, "自身广播应被过滤，同 id 应合并");

    let devices = discovery.devices();
    let mac = devices.iter().find(|d| d.id.as_str() == "peer-1").unwrap();
    assert_eq!(mac.name, "客厅的 MacBook");
    assert_eq!(mac.addr.port(), 4001);
    assert_eq!(mac.addr.ip().to_string(), "192.168.1.9");
    assert!(discovery.find("书房").is_some());
}

#[test]
fn prunes_devices_that_stop_broadcasting() {
    let peer = Arc::new(FakePeer::default());
    peer.push_inbox(hello("peer-1", "客厅的 Mac", 4000));

    let mut discovery = controller(Arc::clone(&peer), "self-id", Duration::from_secs(60));
    assert_eq!(discovery.tick().unwrap(), 1);

    thread::sleep(Duration::from_millis(90));
    assert_eq!(discovery.tick().unwrap(), 0, "超时设备应自动下线");
    assert!(discovery.devices().is_empty());
}
