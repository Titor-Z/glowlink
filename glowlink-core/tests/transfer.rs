use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use glowlink_core::controller::transfer::{TransferController, TransferTransport, TransferView};
use glowlink_core::model::crc::crc32;
use glowlink_core::model::error::{GlowError, Result};
use glowlink_core::model::packet::Frame;

/// 内存传输通道：由两条 mpsc 通道构成一条双向连接。
struct FakeTransport {
    tx: mpsc::Sender<Frame>,
    rx: mpsc::Receiver<Frame>,
}

impl TransferTransport for FakeTransport {
    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        self.tx
            .send(frame.clone())
            .map_err(|_| GlowError::Protocol("对端已断开".into()))
    }

    fn recv_frame(&mut self) -> Result<Frame> {
        self.rx.recv().map_err(|_| GlowError::Protocol("对端已断开".into()))
    }
}

fn pair() -> (FakeTransport, FakeTransport) {
    let (tx_a, rx_a) = mpsc::channel();
    let (tx_b, rx_b) = mpsc::channel();
    (FakeTransport { tx: tx_a, rx: rx_b }, FakeTransport { tx: tx_b, rx: rx_a })
}

/// 适配器：发送时破坏分块内容，用于验证分块校验。
struct CorruptingTransport(FakeTransport);

impl TransferTransport for CorruptingTransport {
    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        match frame {
            Frame::Chunk { offset, data, crc } => {
                let mut data = data.clone();
                data[0] ^= 0xFF;
                self.0.send_frame(&Frame::Chunk { offset: *offset, data, crc: *crc })
            }
            other => self.0.send_frame(other),
        }
    }

    fn recv_frame(&mut self) -> Result<Frame> {
        self.0.recv_frame()
    }
}

struct AutoView;

impl TransferView for AutoView {
    fn confirm_receive(&self, _peer: &str, _name: &str, _size: u64) -> bool {
        true
    }

    fn progress(&self, _label: &str, _done: u64, _total: u64) {}
}

struct DeclineView;

impl TransferView for DeclineView {
    fn confirm_receive(&self, _peer: &str, _name: &str, _size: u64) -> bool {
        false
    }

    fn progress(&self, _label: &str, _done: u64, _total: u64) {}
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "glowlink-test-{tag}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn transfers_file_end_to_end() {
    let src_dir = temp_dir("src");
    let dst_dir = temp_dir("dst");
    let source = src_dir.join("sample.bin");
    let data = payload(200_000);
    fs::write(&source, &data).unwrap();

    // 环容量刻意小于文件，验证背压下仍能完整传输。
    let controller = TransferController::new(1024, 4096);
    let (sender, mut receiver) = pair();
    let sender_thread = thread::spawn(move || {
        let mut sender = sender;
        controller.send_file(&mut sender, &source, &AutoView)
    });

    let received = controller
        .receive_file(&mut receiver, &dst_dir, "192.168.1.9", &AutoView)
        .unwrap();
    assert_eq!(sender_thread.join().unwrap().unwrap(), 200_000);
    assert_eq!(received.file_name().unwrap(), "sample.bin");
    assert_eq!(fs::read(&received).unwrap(), data);

    fs::remove_dir_all(&src_dir).unwrap();
    fs::remove_dir_all(&dst_dir).unwrap();
}

#[test]
fn second_transfer_of_same_name_gets_new_path() {
    let src_dir = temp_dir("dup-src");
    let dst_dir = temp_dir("dup-dst");
    let source = src_dir.join("sample.bin");
    fs::write(&source, b"old").unwrap();
    fs::write(dst_dir.join("sample.bin"), b"already here").unwrap();

    let controller = TransferController::new(512, 2048);
    let (sender, mut receiver) = pair();
    let sender_thread = thread::spawn(move || {
        let mut sender = sender;
        controller.send_file(&mut sender, &source, &AutoView)
    });

    let received = controller
        .receive_file(&mut receiver, &dst_dir, "peer", &AutoView)
        .unwrap();
    sender_thread.join().unwrap().unwrap();

    assert_eq!(received.file_name().unwrap(), "sample (1).bin");
    assert_eq!(fs::read(&received).unwrap(), b"old");
    assert_eq!(fs::read(dst_dir.join("sample.bin")).unwrap(), b"already here");

    fs::remove_dir_all(&src_dir).unwrap();
    fs::remove_dir_all(&dst_dir).unwrap();
}

#[test]
fn rejected_offer_fails_both_sides() {
    let src_dir = temp_dir("reject-src");
    let dst_dir = temp_dir("reject-dst");
    let source = src_dir.join("sample.bin");
    fs::write(&source, payload(4096)).unwrap();

    let controller = TransferController::new(512, 2048);
    let (sender, mut receiver) = pair();
    let sender_thread = thread::spawn(move || {
        let mut sender = sender;
        controller.send_file(&mut sender, &source, &AutoView)
    });

    let error = controller
        .receive_file(&mut receiver, &dst_dir, "peer", &DeclineView)
        .unwrap_err();
    assert!(matches!(error, GlowError::Rejected(_)), "接收方应返回拒绝错误");

    let sender_error = sender_thread.join().unwrap().unwrap_err();
    assert!(
        matches!(sender_error, GlowError::Rejected(_)),
        "发送方应收到拒绝错误，实际：{sender_error}"
    );
    assert!(fs::read_dir(&dst_dir).unwrap().next().is_none(), "拒绝后不应留下文件");

    fs::remove_dir_all(&src_dir).unwrap();
    fs::remove_dir_all(&dst_dir).unwrap();
}

#[test]
fn corrupted_chunk_is_detected_and_partial_file_removed() {
    let src_dir = temp_dir("corrupt-src");
    let dst_dir = temp_dir("corrupt-dst");
    let source = src_dir.join("sample.bin");
    fs::write(&source, payload(8192)).unwrap();

    let controller = TransferController::new(512, 4096);
    let (sender, mut receiver) = pair();
    let sender_thread = thread::spawn(move || {
        let mut sender = CorruptingTransport(sender);
        controller.send_file(&mut sender, &source, &AutoView)
    });

    let error = controller
        .receive_file(&mut receiver, &dst_dir, "peer", &AutoView)
        .unwrap_err();
    assert!(
        matches!(&error, GlowError::Protocol(m) if m.contains("分块校验失败")),
        "应检测到分块校验失败，实际：{error}"
    );
    assert!(fs::read_dir(&dst_dir).unwrap().next().is_none(), "校验失败后应删除残片");
    let _ = sender_thread.join();

    fs::remove_dir_all(&src_dir).unwrap();
    fs::remove_dir_all(&dst_dir).unwrap();
}

#[test]
fn path_traversal_name_is_sanitized() {
    let dst_dir = temp_dir("traversal");
    let (mut sender, mut receiver) = pair();
    let data = b"evil".to_vec();
    let size = data.len() as u64;

    sender
        .send_frame(&Frame::Offer { name: "../escaped.txt".into(), size })
        .unwrap();
    sender
        .send_frame(&Frame::Chunk { offset: 0, data: data.clone(), crc: crc32(&data) })
        .unwrap();
    sender
        .send_frame(&Frame::Done { size, crc: crc32(&data) })
        .unwrap();

    let controller = TransferController::new(512, 2048);
    let received = controller
        .receive_file(&mut receiver, &dst_dir, "peer", &AutoView)
        .unwrap();

    assert_eq!(received, dst_dir.join("escaped.txt"));
    assert_eq!(fs::read(&received).unwrap(), data);
    assert!(
        !dst_dir.parent().unwrap().join("escaped.txt").exists(),
        "文件名中的上级路径必须被剥离"
    );

    fs::remove_dir_all(&dst_dir).unwrap();
}

#[test]
fn out_of_order_chunk_is_rejected() {
    let dst_dir = temp_dir("order");
    let (mut sender, mut receiver) = pair();
    let data = b"hello".to_vec();

    sender
        .send_frame(&Frame::Offer { name: "a.bin".into(), size: data.len() as u64 })
        .unwrap();
    // 偏移从 10 开始，接收端应拒绝。
    sender
        .send_frame(&Frame::Chunk { offset: 10, data: data.clone(), crc: crc32(&data) })
        .unwrap();

    let controller = TransferController::new(512, 2048);
    let error = controller
        .receive_file(&mut receiver, &dst_dir, "peer", &AutoView)
        .unwrap_err();
    assert!(
        matches!(&error, GlowError::Protocol(m) if m.contains("分块乱序")),
        "应拒绝乱序分块，实际：{error}"
    );

    fs::remove_dir_all(&dst_dir).unwrap();
}
