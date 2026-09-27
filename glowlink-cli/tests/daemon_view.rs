//! `glowlink-cli/src/view/daemon.rs` 的单元测试。
//!
//! 重点验证「没有终端的守护进程怎么问人」，以及 `--yes` 的**边界**：
//! 只有「已信任设备 + 开了 --yes」才免确认；陌生设备即使开了 --yes 也必须人工确认。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glowlink_cli::keys::TrustFile;
use glowlink_cli::pending::PendingQueue;
use glowlink_cli::DaemonView;
use glowlink_core::controller::transfer::TransferView;
use glowlink_core::patterns::secure::{session_slot, NoTrust, SessionInfo, SessionSlot};
use glowlink_core::patterns::secure::TrustList;

/// 这些用例会改进程级 `HOME`，必须串行。
static HOME_LOCK: Mutex<()> = Mutex::new(());

/// 当前用例的临时目录（避免碰真实的 ~/.glowlink）。
fn isolate_home(tag: &str) -> std::sync::MutexGuard<'static, ()> {
    let guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("glowlink-daemonview-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("HOME", &dir);
    guard
}

const PEER: &str = "192.168.31.9";
const FINGERPRINT: &str = "abcdef0123456789";
const CODE: &str = "482913";

/// 造一份「某个对端刚握手完成」的会话槽。
fn session(trusted: bool) -> SessionSlot {
    let slot = session_slot();
    *slot.lock().unwrap() = Some(SessionInfo {
        peer_fingerprint: FINGERPRINT.to_string(),
        peer_public: [0u8; 32],
        code: CODE.to_string(),
        trusted,
        initiator: false,
    });
    slot
}

/// 造一份空的会话槽（模拟「握手没完成就走到这里」，属于异常路径）。
fn empty_session() -> SessionSlot {
    session_slot()
}

fn trust_file(tag: &str, trusted: bool) -> Arc<TrustFile> {
    let path: PathBuf = std::env::temp_dir()
        .join(format!("glowlink-daemonview-{tag}"))
        .join("trusted.txt");
    let trust = TrustFile::open(path).unwrap();
    if trusted {
        trust.trust(FINGERPRINT, "甲机").unwrap();
    }
    trust
}

/// 默认视图：**未信任** + 不开自动接收 ⇒ 一定走「登记待确认」。
fn view(queue: Arc<PendingQueue>, timeout: Duration, tag: &str) -> DaemonView {
    DaemonView::new(queue, timeout, session(false), trust_file(tag, false), false)
}

/// 已信任 + `--yes` ⇒ 免确认自动接收。
fn auto_view(queue: Arc<PendingQueue>, timeout: Duration, tag: &str) -> DaemonView {
    DaemonView::new(queue, timeout, session(true), trust_file(tag, true), true)
}

fn wait_until(queue: &PendingQueue, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while queue.count() < count && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(queue.count(), count, "等待登记超时");
}

#[test]
fn confirm_receive_waits_for_accept() {
    let _guard = isolate_home("accept");
    let queue = Arc::new(PendingQueue::new());
    let view = view(Arc::clone(&queue), Duration::from_secs(5), "accept");

    let decider = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || {
            wait_until(&queue, 1);
            let (token, _) = queue.list().remove(0);
            queue.decide(Some(&token), true).unwrap();
        })
    };

    assert!(view.confirm_receive(PEER, "报告.pdf", 2048));
    decider.join().unwrap();
}

#[test]
fn confirm_receive_waits_for_reject() {
    let _guard = isolate_home("reject");
    let queue = Arc::new(PendingQueue::new());
    let view = view(Arc::clone(&queue), Duration::from_secs(5), "reject");

    let decider = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || {
            wait_until(&queue, 1);
            let (token, _) = queue.list().remove(0);
            queue.decide(Some(&token), false).unwrap();
        })
    };

    assert!(!view.confirm_receive(PEER, "报告.pdf", 2048));
    decider.join().unwrap();
}

#[test]
fn unanswered_request_times_out_as_reject() {
    let _guard = isolate_home("timeout");
    let view = view(Arc::new(PendingQueue::new()), Duration::from_millis(60), "timeout");
    // 没人确认就什么都不收
    assert!(!view.confirm_receive("10.0.0.2", "a.bin", 1));
}

#[test]
fn pending_offer_carries_fingerprint_and_code() {
    let _guard = isolate_home("visible");
    let queue = Arc::new(PendingQueue::new());
    let probe = Arc::clone(&queue);
    let handle = std::thread::spawn(move || {
        let view = view(Arc::clone(&probe), Duration::from_secs(5), "visible");
        view.confirm_receive("10.0.0.3", "b.bin", 99)
    });

    wait_until(&queue, 1);
    let (token, seen) = queue.list().remove(0);
    assert_eq!(seen.peer, "10.0.0.3");
    assert_eq!(seen.name, "b.bin");
    assert_eq!(seen.size, 99);
    // 用户要靠这两项核对来源，缺了就无从判断是不是中间人
    assert_eq!(seen.fingerprint, FINGERPRINT, "待确认项必须带指纹");
    assert_eq!(seen.code, CODE, "待确认项必须带 6 位校验码");
    assert!(!token.is_empty(), "列表必须带编号，用户按编号确认");

    queue.decide(Some(&token), false).unwrap();
    assert!(!handle.join().unwrap());
}

#[test]
fn two_pending_offers_do_not_block_each_other() {
    let _guard = isolate_home("multi");
    let queue = Arc::new(PendingQueue::new());
    let mut handles = Vec::new();
    for (peer, name) in [("10.0.0.5", "一.bin"), ("10.0.0.6", "二.bin")] {
        let view = view(Arc::clone(&queue), Duration::from_secs(5), "multi");
        handles.push(std::thread::spawn(move || view.confirm_receive(peer, name, 10)));
    }

    wait_until(&queue, 2);
    let listed = queue.list();
    queue.decide(Some(&listed[0].0), true).unwrap();
    queue.decide(Some(&listed[1].0), false).unwrap();

    let results: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|ok| **ok).count(), 1);
}

// ------------------------------------------------------- --yes 的边界（安全）

#[test]
fn trusted_device_with_yes_is_accepted_without_asking() {
    let _guard = isolate_home("auto");
    let queue = Arc::new(PendingQueue::new());
    let view = auto_view(Arc::clone(&queue), Duration::from_secs(5), "auto");

    // 免确认：直接返回 true，且不应产生待确认项
    assert!(view.confirm_receive(PEER, "自动收.bin", 4096));
    assert_eq!(queue.count(), 0, "已信任设备不该占用待确认槽位");
}

#[test]
fn untrusted_device_is_never_auto_accepted_even_with_yes() {
    let _guard = isolate_home("yes-untrusted");
    let queue = Arc::new(PendingQueue::new());
    // 开了 --yes，但指纹**不在**信任列表里 ⇒ 必须人工确认
    let view = DaemonView::new(
        Arc::clone(&queue),
        Duration::from_secs(5),
        session(false),
        trust_file("yes-untrusted", false),
        true,
    );
    let worker = std::thread::spawn(move || view.confirm_receive(PEER, "陌生.bin", 4096));

    // 走了人工确认这条路：一定会留下待确认项，用户才能看到校验码去核对
    wait_until(&queue, 1);
    let (token, offer) = queue.list().remove(0);
    assert_eq!(offer.fingerprint, FINGERPRINT);
    assert_eq!(offer.code, CODE, "用户必须能看到校验码");
    queue.decide(Some(&token), false).unwrap();
    assert!(!worker.join().unwrap(), "陌生设备即使开了 --yes 也不能自动接收");
}

#[test]
fn confirming_a_new_device_remembers_it_for_next_time() {
    let _guard = isolate_home("remember");
    let queue = Arc::new(PendingQueue::new());
    let trust = trust_file("remember", false);
    let view = DaemonView::new(
        Arc::clone(&queue),
        Duration::from_secs(5),
        session(false),
        Arc::clone(&trust),
        false,
    );

    assert!(!trust.is_trusted(FINGERPRINT), "确认之前不该在信任列表里");

    let decider = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || {
            wait_until(&queue, 1);
            let (token, _) = queue.list().remove(0);
            queue.decide(Some(&token), true).unwrap();
        })
    };
    assert!(view.confirm_receive(PEER, "首次.bin", 1024));
    decider.join().unwrap();

    // 用户说了 yes ⇒ 记住这台设备，下次（配合 --yes）就能免确认
    assert!(trust.is_trusted(FINGERPRINT), "确认后应写入信任列表");
    assert_eq!(trust.name_of(FINGERPRINT).as_deref(), Some(PEER));
}

#[test]
fn rejecting_a_new_device_does_not_remember_it() {
    let _guard = isolate_home("forget");
    let queue = Arc::new(PendingQueue::new());
    let trust = trust_file("forget", false);
    let view = DaemonView::new(
        Arc::clone(&queue),
        Duration::from_secs(5),
        session(false),
        Arc::clone(&trust),
        false,
    );

    let decider = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || {
            wait_until(&queue, 1);
            let (token, _) = queue.list().remove(0);
            queue.decide(Some(&token), false).unwrap();
        })
    };
    assert!(!view.confirm_receive(PEER, "拒绝.bin", 1024));
    decider.join().unwrap();
    assert!(!trust.is_trusted(FINGERPRINT), "拒绝接收不该记住这台设备");
}

#[test]
fn missing_session_is_refused_outright() {
    let _guard = isolate_home("no-session");
    let queue = Arc::new(PendingQueue::new());
    // 没有会话信息 == 这不是一次加密连接 ⇒ 直接拒绝，不登记、不询问
    let view = DaemonView::new(
        Arc::clone(&queue),
        Duration::from_secs(5),
        empty_session(),
        trust_file("no-session", false),
        false,
    );

    assert!(!view.confirm_receive(PEER, "可疑.bin", 1));
    assert_eq!(queue.count(), 0, "不该为没有加密会话的连接登记待确认");
    // 顺带确认 NoTrust 的默认语义（测试替身也走一遍）
    assert!(!NoTrust.is_trusted(FINGERPRINT));
}

#[test]
fn progress_is_a_noop_on_daemon() {
    let _guard = isolate_home("progress");
    let view = view(Arc::new(PendingQueue::new()), Duration::from_secs(1), "progress");
    // 守护进程没有终端，进度只写日志会淹掉有用信息；这里只要求不 panic
    view.progress("对端", 512, 1024);
}

/// 已信任设备走人工确认时，日志里也必须留着校验码。
///
/// 「本机信任对方」不等于「对方信任本机」：发送方那边可能正要求核对，
/// 而日志是事后唯一能查到「这次到底跟谁核对过」的地方。
#[test]
fn trusted_pending_still_logs_the_code() {
    use glowlink_cli::view::daemon::pending_hint;
    let hint = pending_hint("abcdef0123456789", CODE, true);
    assert!(hint.contains(CODE), "已信任也要留码：{hint}");
}
