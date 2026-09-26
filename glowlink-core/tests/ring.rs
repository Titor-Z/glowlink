use std::sync::Arc;
use std::thread;
use std::time::Duration;

use glowlink_core::model::ring::{RingBuffer, SharedRing};

#[test]
fn push_and_pop_wrap_around() {
    let mut ring = RingBuffer::new(8);
    assert_eq!(ring.push(b"abcdef"), 6);
    assert_eq!(ring.len(), 6);

    let mut out = [0u8; 4];
    assert_eq!(ring.pop(&mut out), 4);
    assert_eq!(&out, b"abcd");

    // 回绕写入：尾部剩余 2 字节 + 头部 2 字节。
    assert_eq!(ring.push(b"wxyz"), 4);
    assert_eq!(ring.len(), 6);

    let mut out = [0u8; 6];
    assert_eq!(ring.pop(&mut out), 6);
    assert_eq!(&out, b"efwxyz");
    assert!(ring.is_empty());
}

#[test]
fn push_reports_partial_write_when_full() {
    let mut ring = RingBuffer::new(4);
    assert_eq!(ring.push(b"abcdefgh"), 4, "满时只应写入可用空间");
    assert_eq!(ring.free(), 0);
    assert_eq!(ring.push(b"x"), 0);
}

#[test]
fn pop_reports_partial_read_when_short() {
    let mut ring = RingBuffer::new(16);
    ring.push(b"ab");
    let mut out = [0u8; 8];
    assert_eq!(ring.pop(&mut out), 2);
    assert_eq!(&out[..2], b"ab");
}

#[test]
fn shared_ring_streams_across_threads_with_backpressure() {
    // 环容量远小于数据量，只有生产者被阻塞 + 消费者持续取走才能跑完。
    let ring = Arc::new(SharedRing::new(16));
    let producer = {
        let ring = Arc::clone(&ring);
        thread::spawn(move || {
            let data: Vec<u8> = (0..5_000u32).map(|i| (i % 256) as u8).collect();
            let written = ring.push_all(&data);
            ring.close();
            (data, written)
        })
    };

    let mut received = Vec::new();
    let mut buf = [0u8; 7];
    loop {
        let n = ring.pop_into(&mut buf);
        if n == 0 {
            break;
        }
        received.extend_from_slice(&buf[..n]);
    }

    let (data, written) = producer.join().unwrap();
    assert_eq!(written, data.len());
    assert_eq!(received, data);
    assert!(ring.is_closed());
}

#[test]
fn close_unblocks_pending_producer() {
    let ring = Arc::new(SharedRing::new(4));
    let worker = {
        let ring = Arc::clone(&ring);
        thread::spawn(move || ring.push_all(&[0u8; 1024]))
    };
    thread::sleep(Duration::from_millis(20));
    ring.close();
    assert!(worker.join().unwrap() < 1024, "关闭后生产者应立即返回");
}
