use std::fs;
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use glowlink_core::controller::discovery::{DiscoveryEvent, DiscoveryTransport};
use glowlink_core::controller::transfer::{TransferController, TransferListener, TransferView};
use glowlink_core::model::packet::{Frame, Hello};
use glowlink_core::patterns::adapter::{
    reply_target, TcpListenerAdapter, TcpTransportAdapter, UdpDiscoveryAdapter,
};

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

fn hello() -> Hello {
    Hello { id: "peer-1".into(), name: "书房 PC".into(), port: 4000, reply_port: 9267 }
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
fn receives_query_and_replies_unicast() {
    let port = free_udp_port();
    let adapter = UdpDiscoveryAdapter::bind(port).unwrap();
    assert_eq!(adapter.bound_port().unwrap(), port);
    assert!(!adapter.degraded());

    // 模拟另一个节点从随机端口发出查询。
    let querier = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    querier.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    let query = Frame::Query(hello()).encode().unwrap();
    querier.send_to(&query, ("127.0.0.1", port)).unwrap();

    let events = adapter.poll(Duration::from_millis(500)).unwrap();
    assert_eq!(events.len(), 1);
    let DiscoveryEvent::Query { hello: received, from } = &events[0] else {
        panic!("应识别为查询事件，实际：{:?}", events[0]);
    };
    assert_eq!(received, &hello());
    assert_eq!(from.ip().to_string(), "127.0.0.1");

    // 单播应答回到查询来源地址。
    adapter.reply(&Hello { id: "self".into(), name: "本机".into(), port: 51234, reply_port: 9267 }, *from).unwrap();
    let mut buf = vec![0u8; 1024];
    let (len, _) = querier.recv_from(&mut buf).unwrap();
    assert_eq!(
        Frame::decode(&buf[..len]).unwrap(),
        Frame::Hello(Hello { id: "self".into(), name: "本机".into(), port: 51234, reply_port: 9267 })
    );
}

#[test]
fn receives_hello_reply() {
    let port = free_udp_port();
    let adapter = UdpDiscoveryAdapter::bind(port).unwrap();

    let responder = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    let reply = Frame::Hello(hello()).encode().unwrap();
    responder.send_to(&reply, ("127.0.0.1", port)).unwrap();

    let events = adapter.poll(Duration::from_millis(500)).unwrap();
    assert_eq!(events.len(), 1);
    assert!(
        matches!(&events[0], DiscoveryEvent::Reply { hello: h, .. } if h == &hello()),
        "HELLO 应被识别为应答事件，实际：{:?}",
        events[0]
    );
}

#[test]
fn ignores_foreign_datagrams() {
    let port = free_udp_port();
    let adapter = UdpDiscoveryAdapter::bind(port).unwrap();

    let sender = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    sender.send_to(b"not-a-glowlink-frame", ("127.0.0.1", port)).unwrap();
    sender.send_to(&[0xFFu8; 32], ("127.0.0.1", port)).unwrap();

    let events = adapter.poll(Duration::from_millis(300)).unwrap();
    assert!(events.is_empty(), "非协议报文必须被忽略而不是报错");
}

#[test]
fn degrades_to_send_only_when_port_is_taken() {
    let port = free_udp_port();
    let holder = UdpDiscoveryAdapter::bind(port).unwrap();
    assert!(!holder.degraded());
    assert_eq!(holder.bound_port(), Some(port));

    let degraded = UdpDiscoveryAdapter::bind(port).unwrap();
    assert!(degraded.degraded(), "端口被占用时应降级");
    // 关键语义：降级节点不绑定固定的发现端口（否则会把端口永远占死，
    // 让后来启动的节点再也无法成为应答方），但仍持有临时端口用于收应答。
    assert_eq!(degraded.bound_port(), None);
    assert!(degraded.poll(Duration::from_millis(10)).unwrap().is_empty());
    degraded.query(&hello()).expect("降级节点仍应能发送查询");
}

#[test]
fn reply_target_prefers_declared_reply_port() {
    // 广播时系统分配的源端口是临时的，回包必须用报文里声明的接收端口
    let from: SocketAddr = "192.168.1.9:54321".parse().unwrap();
    let declared = Hello { reply_port: 9267, ..hello() };
    assert_eq!(reply_target(&declared, from).to_string(), "192.168.1.9:9267");

    // 对方也是降级节点（未声明端口）时，才回退到报文源地址
    let undeclared = Hello { reply_port: 0, ..hello() };
    assert_eq!(reply_target(&undeclared, from), from);
}

#[test]
fn query_sends_broadcast_without_error() {
    let port = free_udp_port();
    let adapter = UdpDiscoveryAdapter::bind(port).unwrap();
    adapter.query(&hello()).expect("广播查询应当发送成功");
}
