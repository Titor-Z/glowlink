//! `glowlink-cli/src/view/daemon.rs` 的单元测试。
//!
//! 重点验证「没有终端的守护进程怎么问人」：阻塞等待 IPC 落决定，超时按拒绝。

use std::sync::Arc;
use std::time::{Duration, Instant};

use glowlink_cli::daemon::{PendingOffer, PendingSlot};
use glowlink_cli::DaemonView;
use glowlink_core::controller::transfer::TransferView;

/// 把 HOME 指到临时目录，避免测试往真实的 ~/.glowlink/daemon.log 里写日志。
fn isolate_home(tag: &str) {
    let dir = std::env::temp_dir().join(format!("glowlink-test-{tag}"));
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("HOME", &dir);
}

fn view(slot: Arc<PendingSlot>, timeout: Duration) -> DaemonView {
    DaemonView::new(slot, timeout)
}

#[test]
fn confirm_receive_waits_for_accept() {
    isolate_home("accept");
    let slot = Arc::new(PendingSlot::new());
    let view = view(Arc::clone(&slot), Duration::from_secs(5));

    let decider = {
        let slot = Arc::clone(&slot);
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while slot.current().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            slot.decide(true).unwrap();
        })
    };

    assert!(view.confirm_receive("192.168.31.9", "报告.pdf", 2048));
    decider.join().unwrap();
}

#[test]
fn confirm_receive_waits_for_reject() {
    isolate_home("reject");
    let slot = Arc::new(PendingSlot::new());
    let view = view(Arc::clone(&slot), Duration::from_secs(5));

    let decider = {
        let slot = Arc::clone(&slot);
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while slot.current().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            slot.decide(false).unwrap();
        })
    };

    assert!(!view.confirm_receive("192.168.31.9", "报告.pdf", 2048));
    decider.join().unwrap();
}

#[test]
fn unanswered_request_times_out_as_reject() {
    isolate_home("timeout");
    let slot = Arc::new(PendingSlot::new());
    let view = view(slot, Duration::from_millis(60));
    // 没人确认就什么都不收
    assert!(!view.confirm_receive("10.0.0.2", "a.bin", 1));
}

#[test]
fn progress_is_a_noop_on_daemon() {
    isolate_home("progress");
    let view = view(Arc::new(PendingSlot::new()), Duration::from_secs(1));
    // 守护进程没有终端，进度只写日志会淹掉有用信息；这里只要求不 panic
    view.progress("对端", 512, 1024);
}

#[test]
fn pending_offer_is_visible_while_waiting() {
    isolate_home("visible");
    let slot = Arc::new(PendingSlot::new());
    let probe = Arc::clone(&slot);
    let handle = std::thread::spawn(move || {
        let view = DaemonView::new(Arc::clone(&probe), Duration::from_secs(5));
        view.confirm_receive("10.0.0.3", "b.bin", 99)
    });

    let wait = Arc::clone(&slot);
    let deadline = Instant::now() + Duration::from_secs(2);
    while wait.current().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        slot.current(),
        Some(PendingOffer { peer: "10.0.0.3".into(), name: "b.bin".into(), size: 99 }),
        "等待期间要能从外部看到请求，否则用户无从确认"
    );
    slot.decide(false).unwrap();
    assert!(!handle.join().unwrap());
}
