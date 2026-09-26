//! `glowlink-cli/src/view/daemon.rs` 的单元测试。
//!
//! 重点验证「没有终端的守护进程怎么问人」：阻塞等待 IPC 落决定，超时按拒绝。

use std::sync::Arc;
use std::time::{Duration, Instant};

use glowlink_cli::pending::{PendingOffer, PendingQueue};
use glowlink_cli::DaemonView;
use glowlink_core::controller::transfer::TransferView;


/// 把 HOME 指到临时目录，避免测试往真实的 ~/.glowlink/daemon.log 里写日志。
fn isolate_home(tag: &str) {
    let dir = std::env::temp_dir().join(format!("glowlink-daemonview-{tag}"));
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("HOME", &dir);
}

fn view(slot: Arc<PendingQueue>, timeout: Duration) -> DaemonView {
    DaemonView::new(slot, timeout)
}

#[test]
fn confirm_receive_waits_for_accept() {
    isolate_home("accept");
    let slot = Arc::new(PendingQueue::new());
    let view = view(Arc::clone(&slot), Duration::from_secs(5));

    let decider = {
        let slot = Arc::clone(&slot);
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while slot.count() == 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let (token, _) = slot.list().remove(0);
            slot.decide(Some(&token), true).unwrap();
        })
    };

    assert!(view.confirm_receive("192.168.31.9", "报告.pdf", 2048));
    decider.join().unwrap();
}

#[test]
fn confirm_receive_waits_for_reject() {
    isolate_home("reject");
    let slot = Arc::new(PendingQueue::new());
    let view = view(Arc::clone(&slot), Duration::from_secs(5));

    let decider = {
        let slot = Arc::clone(&slot);
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while slot.count() == 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let (token, _) = slot.list().remove(0);
            slot.decide(Some(&token), false).unwrap();
        })
    };

    assert!(!view.confirm_receive("192.168.31.9", "报告.pdf", 2048));
    decider.join().unwrap();
}

#[test]
fn unanswered_request_times_out_as_reject() {
    isolate_home("timeout");
    let slot = Arc::new(PendingQueue::new());
    let view = view(slot, Duration::from_millis(60));
    // 没人确认就什么都不收
    assert!(!view.confirm_receive("10.0.0.2", "a.bin", 1));
}

#[test]
fn two_pending_offers_do_not_block_each_other() {
    // 多槽位的意义：第二个发送方不能静默挂在 backlog 里
    isolate_home("multi");
    let queue = Arc::new(PendingQueue::new());
    let mut handles = Vec::new();
    for (peer, name) in [("10.0.0.5", "一.bin"), ("10.0.0.6", "二.bin")] {
        let view = DaemonView::new(Arc::clone(&queue), Duration::from_secs(5));
        handles.push(std::thread::spawn(move || view.confirm_receive(peer, name, 10)));
    }

    let deadline = Instant::now() + Duration::from_secs(2);
    while queue.count() < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(queue.count(), 2, "两个请求应同时挂着");

    let listed = queue.list();
    queue.decide(Some(&listed[0].0), true).unwrap();
    queue.decide(Some(&listed[1].0), false).unwrap();

    let results: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|ok| **ok).count(), 1);
}

#[test]
fn progress_is_a_noop_on_daemon() {
    isolate_home("progress");
    let view = view(Arc::new(PendingQueue::new()), Duration::from_secs(1));
    // 守护进程没有终端，进度只写日志会淹掉有用信息；这里只要求不 panic
    view.progress("对端", 512, 1024);
}

#[test]
fn pending_offer_is_visible_while_waiting() {
    isolate_home("visible");
    let slot = Arc::new(PendingQueue::new());
    let probe = Arc::clone(&slot);
    let handle = std::thread::spawn(move || {
        let view = DaemonView::new(Arc::clone(&probe), Duration::from_secs(5));
        view.confirm_receive("10.0.0.3", "b.bin", 99)
    });

    let wait = Arc::clone(&slot);
    let deadline = Instant::now() + Duration::from_secs(2);
    while wait.count() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let (token, seen) = slot.list().remove(0);
    assert_eq!(
        seen,
        PendingOffer { peer: "10.0.0.3".into(), name: "b.bin".into(), size: 99 },
        "等待期间要能从外部看到请求，否则用户无从确认"
    );
    assert!(!token.is_empty(), "列表必须带编号，用户要按编号确认");
    slot.decide(Some(&token), false).unwrap();
    assert!(!handle.join().unwrap());
}
