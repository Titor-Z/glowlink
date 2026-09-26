//! `glowlink-cli/src/pending.rs` 的单元测试：多槽位、编号、超时自动拒绝。

use std::sync::Arc;
use std::time::{Duration, Instant};

use glowlink_cli::pending::{PendingOffer, PendingQueue};

fn offer(name: &str) -> PendingOffer {
    PendingOffer { peer: "192.168.31.9".into(), name: name.into(), size: 1024 }
}

/// 等某个请求出现在列表里（另一个线程刚登记完）。
fn wait_until(queue: &PendingQueue, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while queue.count() < count && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(queue.count(), count, "等待登记超时");
}

#[test]
fn ask_blocks_until_decided_by_token() {
    let queue = Arc::new(PendingQueue::new());
    let waiter = Arc::clone(&queue);
    let handle = std::thread::spawn(move || {
        waiter.ask(offer("报告.pdf"), Duration::from_secs(5), |_| {})
    });

    wait_until(&queue, 1);
    let (token, seen) = queue.list().remove(0);
    assert_eq!(seen.name, "报告.pdf", "列表里要能看到请求内容");
    assert_eq!(queue.decide(Some(&token), true).unwrap(), token);

    assert!(handle.join().unwrap(), "接受后应返回 true");
    assert!(queue.list().is_empty(), "处理完槽位要清空");
}

#[test]
fn two_requests_coexist_and_are_decided_independently() {
    // 这就是多槽位存在的理由：第二个发送方不能静默挂在 backlog 里
    let queue = Arc::new(PendingQueue::new());
    let mut handles = Vec::new();
    for name in ["甲.bin", "乙.bin"] {
        let queue = Arc::clone(&queue);
        let name = name.to_string();
        handles.push(std::thread::spawn(move || {
            queue.ask(offer(&name), Duration::from_secs(5), |_| {})
        }));
    }

    wait_until(&queue, 2);
    let listed = queue.list();
    let tokens: Vec<String> = listed.iter().map(|(t, _)| t.clone()).collect();
    assert_eq!(tokens.len(), 2);
    assert_ne!(tokens[0], tokens[1], "两个请求必须拿到不同编号");

    // 只决定第一个，第二个必须保持等待
    queue.decide(Some(&tokens[0]), true).unwrap();
    assert_eq!(queue.count(), 1, "只应消掉被指定的那一个");

    queue.decide(Some(&tokens[1]), false).unwrap();
    let results: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|ok| **ok).count(), 1, "只有一条被接受");
}

#[test]
fn decide_without_token_requires_a_unique_request() {
    let queue = Arc::new(PendingQueue::new());
    // 空队列：报错，不能假装成功
    assert!(queue.decide(None, true).is_err());

    let first = Arc::clone(&queue);
    let second = Arc::clone(&queue);
    let h1 = std::thread::spawn(move || first.ask(offer("a.bin"), Duration::from_secs(5), |_| {}));
    wait_until(&queue, 1);
    let h2 = std::thread::spawn(move || second.ask(offer("b.bin"), Duration::from_secs(5), |_| {}));
    wait_until(&queue, 2);

    // 多个待确认时不给编号必须报错，避免点错对象
    let err = queue.decide(None, true).unwrap_err().to_string();
    assert!(err.contains("2 个待确认请求"), "实际：{err}");

    for (token, _) in queue.list() {
        queue.decide(Some(&token), false).unwrap();
    }
    h1.join().unwrap();
    h2.join().unwrap();
}

#[test]
fn unknown_token_is_rejected() {
    let queue = Arc::new(PendingQueue::new());
    let waiter = Arc::clone(&queue);
    let handle = std::thread::spawn(move || waiter.ask(offer("a.bin"), Duration::from_secs(5), |_| {}));
    wait_until(&queue, 1);

    let err = queue.decide(Some("999"), true).unwrap_err().to_string();
    assert!(err.contains("999"), "实际：{err}");
    assert_eq!(queue.count(), 1, "错的编号不该动到任何一个槽位");

    let (token, _) = queue.list().remove(0);
    queue.decide(Some(&token), false).unwrap();
    assert!(!handle.join().unwrap());
}

#[test]
fn timeout_auto_rejects() {
    let queue = PendingQueue::new();
    // 没人管的时候宁可什么都不收
    assert!(!queue.ask(offer("a.bin"), Duration::from_millis(60), |_| {}));
    assert!(queue.list().is_empty(), "超时后槽位必须自己清掉");
}

#[test]
fn timeout_of_one_does_not_disturb_the_other() {
    let queue = Arc::new(PendingQueue::new());
    let short = Arc::clone(&queue);
    let quick = std::thread::spawn(move || short.ask(offer("快.bin"), Duration::from_millis(80), |_| {}));
    wait_until(&queue, 1);

    let long = Arc::clone(&queue);
    let slow = std::thread::spawn(move || long.ask(offer("慢.bin"), Duration::from_secs(5), |_| {}));
    wait_until(&queue, 2);

    assert!(!quick.join().unwrap(), "短超时的那条应被自动拒绝");
    assert_eq!(queue.count(), 1, "另一条必须还在等");

    let (token, seen) = queue.list().remove(0);
    assert_eq!(seen.name, "慢.bin");
    queue.decide(Some(&token), true).unwrap();
    assert!(slow.join().unwrap());
}

#[test]
fn register_callback_receives_the_token() {
    let queue = PendingQueue::new();
    let got = Arc::new(std::sync::Mutex::new(String::new()));
    let sink = Arc::clone(&got);
    std::thread::spawn({
        let queue = Arc::new(queue);
        let sink = Arc::clone(&sink);
        move || {
            queue.ask(offer("a.bin"), Duration::from_millis(100), |token| {
                *sink.lock().unwrap() = token.to_string();
            });
        }
    })
    .join()
    .unwrap();

    let token = got.lock().unwrap().clone();
    assert!(!token.is_empty(), "登记回调必须能拿到编号，用于提示用户");
}
