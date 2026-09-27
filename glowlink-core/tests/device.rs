use std::net::IpAddr;
use std::thread;
use std::time::Duration;

use glowlink_core::model::device::DeviceBook;
use glowlink_core::model::packet::Hello;
use glowlink_core::model::packet::DeviceKind;

fn hello(id: &str, name: &str, port: u16) -> Hello {
    Hello {
        id: id.into(),
        name: name.into(),
        port,
        reply_port: 9267,
        kind: DeviceKind::Desktop,
    }
}

#[test]
fn kind_is_refreshed_on_every_observation() {
    // 同一 ID 换了设备、或对端升级后开始上报类型：每次广播都要跟着刷新，
    // 否则界面会一直拿第一次看到的值画图标。
    let mut book = DeviceBook::new(Duration::from_secs(30));
    book.observe(&hello("peer-1", "甲机", 4000), "10.0.0.1".parse().unwrap());
    assert_eq!(book.find("peer-1").unwrap().kind, DeviceKind::Desktop);

    let mut upgraded = hello("peer-1", "甲机", 4000);
    upgraded.kind = DeviceKind::Unknown;
    book.observe(&upgraded, "10.0.0.1".parse().unwrap());
    assert_eq!(book.find("peer-1").unwrap().kind, DeviceKind::Unknown);
    assert_eq!(book.len(), 1, "刷新类型不该多出一台设备");
}

fn ip(last: u8) -> IpAddr {
    format!("192.168.1.{last}").parse().unwrap()
}

#[test]
fn observe_adds_and_refreshes() {
    let mut book = DeviceBook::new(Duration::from_secs(5));
    book.observe(&hello("d1", "Mac", 4000), ip(10));
    assert_eq!(book.len(), 1);
    assert_eq!(book.devices()[0].name, "Mac");

    // 同一设备改名/改端口后仍是同一台，不应新增。
    book.observe(&hello("d1", "MacBook", 5000), ip(11));
    assert_eq!(book.len(), 1);
    assert_eq!(book.devices()[0].name, "MacBook");
    assert_eq!(book.devices()[0].addr.port(), 5000);
    assert_eq!(book.devices()[0].addr.ip(), ip(11));
}

#[test]
fn prune_removes_stale_devices() {
    let mut book = DeviceBook::new(Duration::from_millis(40));
    book.observe(&hello("d1", "Mac", 4000), ip(10));
    assert_eq!(book.prune(), 0, "未超时不应下线");
    thread::sleep(Duration::from_millis(80));
    assert_eq!(book.prune(), 1);
    assert!(book.is_empty());
}

#[test]
fn find_matches_name_and_id() {
    let mut book = DeviceBook::new(Duration::from_secs(5));
    book.observe(&hello("aabbccdd11223344", "客厅的 Mac", 4000), ip(10));
    book.observe(&hello("1122334455667788", "书房 PC", 4001), ip(11));

    assert_eq!(book.find("客厅的 Mac").unwrap().id.as_str(), "aabbccdd11223344");
    assert_eq!(book.find("客厅的 mac").unwrap().id.as_str(), "aabbccdd11223344");
    assert_eq!(book.find("11223344").unwrap().name, "书房 PC");
    assert_eq!(book.find("书房").unwrap().id.as_str(), "1122334455667788");
    assert!(book.find("不存在的设备").is_none());
}

#[test]
fn short_id_is_truncated() {
    let mut book = DeviceBook::new(Duration::from_secs(5));
    let device = book.observe(&hello("aabbccdd11223344", "Mac", 4000), ip(10));
    assert_eq!(device.short_id(), "aabbccdd");
}
