use std::fs;
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use glowlink_core::controller::discovery::DiscoveryTransport;
use glowlink_core::controller::transfer::{TransferController, TransferListener, TransferView};
use glowlink_core::model::packet::{Frame, Hello};
use glowlink_core::patterns::adapter::{TcpListenerAdapter, TcpTransportAdapter, UdpDiscoveryAdapter};

struct AutoView;

impl TransferView for AutoView {
    fn confirm_receive(&self, _peer: &str, _name: &str, _size: u64) -> bool {
        true
    }

    fn progress(&self, _label: &str, _done: u64, _total: u64) {}
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "glowlink-adapter-{tag}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// 借系统分配一个空闲端口后立刻释放，供适配器绑定。
fn free_udp_port() -> u16 {
    let probe = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    probe.local_addr().unwrap().port()
}

#[test]
fn tcp_adapters_transfer_over_real_socket() {
    let listener = TcpListenerAdapter::bind(0).unwrap();
    let port = listener.local_port().unwrap();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    let src_dir = temp_dir("src");
    let dst_dir = temp_dir("dst");
    let source = src_dir.join("payload.bin");
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    fs::write(&source, &data).unwrap();

    let controller = TransferController::new(64 * 1024, 1 << 20);
    let target = dst_dir.clone();
    let server = thread::spawn(move || {
        let (mut stream, _peer) = listener.accept().unwrap();
        controller
            .receive_file(stream.as_mut(), &target, "127.0.0.1", &AutoView)
            .unwrap()
    });

    let mut client = TcpTransportAdapter::connect(addr, Duration::from_secs(5)).unwrap();
    assert_eq!(controller.send_file(&mut client, &source, &AutoView).unwrap(), 300_000);

    let received = server.join().unwrap();
    assert_eq!(received.file_name().unwrap(), "payload.bin");
    assert_eq!(fs::read(&received).unwrap(), data);

    fs::remove_dir_all(&src_dir).unwrap();
    fs::remove_dir_all(&dst_dir).unwrap();
}

#[test]
fn udp_adapter_receives_hello_datagram() {
    let port = free_udp_port();
    let adapter = UdpDiscoveryAdapter::bind(port).unwrap();

    let hello = Hello { id: "peer-1".into(), name: "书房 PC".into(), port: 4000 };
    let bytes = Frame::Hello(hello.clone()).encode().unwrap();
    let sender = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    sender.send_to(&bytes, ("127.0.0.1", port)).unwrap();

    let received = adapter.poll(Duration::from_millis(500)).unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].0, hello);
    assert_eq!(received[0].1.to_string(), "127.0.0.1");
}

#[test]
fn udp_adapter_ignores_foreign_datagrams() {
    let port = free_udp_port();
    let adapter = UdpDiscoveryAdapter::bind(port).unwrap();

    let sender = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    sender.send_to(b"not-a-glowlink-frame", ("127.0.0.1", port)).unwrap();
    sender.send_to(&[0xFFu8; 32], ("127.0.0.1", port)).unwrap();

    let received = adapter.poll(Duration::from_millis(300)).unwrap();
    assert!(received.is_empty(), "非协议报文必须被忽略而不是报错");
}

#[test]
fn udp_adapter_announces_broadcast() {
    let port = free_udp_port();
    let adapter = UdpDiscoveryAdapter::bind(port).unwrap();
    let hello = Hello { id: "self".into(), name: "本机".into(), port: 4000 };
    adapter.announce(&hello).expect("广播应当发送成功");
}
