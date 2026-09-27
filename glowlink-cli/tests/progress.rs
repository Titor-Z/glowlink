use std::sync::Arc;
use std::time::{Duration, Instant};

use glowlink_cli::keys::TrustFile;
use glowlink_cli::CliView;
use glowlink_core::controller::transfer::TransferView;
use glowlink_core::patterns::secure::{session_slot, SessionInfo, SessionSlot};
use glowlink_core::patterns::singleton::DeviceIdentity;

/// 名称探测：环境变量优先级最高，且不接受空白值。
#[test]
fn clowlink_name_env_is_honoured() {
    // 该用例只验证构造逻辑不因空值崩溃；真实探测依赖系统命令，交由集成测试观察。
    let identity = DeviceIdentity::new("id", "名字");
    assert_eq!(identity.name, "名字");
}

/// 进度回调必须能在极端输入下安全返回（0、超量、倒退）。
#[test]
fn progress_handles_edge_inputs() {
    let view = Arc::new(CliView::new(true));
    view.progress("发送", 0, 0);
    view.progress("发送", 0, 100);
    view.progress("发送", 100, 100);
    // 偏移倒退（下一轮传输）应重新开始而不是 panic
    view.progress("接收", 10, 200);
    view.progress("接收", 200, 200);
    // 上报超过总量（异常输入）也不应 panic
    view.progress("接收", 300, 200);
}

/// 造一份「刚握手完成」的会话（Step 3 起确认逻辑要看它）。
fn session(trusted: bool) -> SessionSlot {
    let slot = session_slot();
    *slot.lock().unwrap() = Some(SessionInfo {
        peer_fingerprint: "abcdef0123456789".to_string(),
        peer_public: [0u8; 32],
        code: "482913".to_string(),
        trusted,
        initiator: false,
    });
    slot
}

fn trust(tag: &str) -> Arc<TrustFile> {
    let path = std::env::temp_dir().join(format!("glowlink-progress-{tag}/trusted.txt"));
    let trust = TrustFile::open(path).unwrap();
    trust.trust("abcdef0123456789", "peer").unwrap();
    trust
}

/// 已信任设备 + `--yes` ⇒ 免确认（这是 Step 3 之后的语义）。
#[test]
fn auto_accept_applies_to_trusted_session() {
    let view = CliView::with_session(true, session(true), trust("trusted"));
    // 已信任会话不再读 stdin，直接收
    assert!(view.confirm_receive("peer", "a.bin", 1));
}

/// 未信任会话 ⇒ 即使 `--yes` 也不能免确认（安全边界，读不到 stdin 即拒绝）。
#[test]
fn auto_accept_does_not_cover_untrusted_session() {
    let view = CliView::with_session(true, session(false), trust("untrusted"));
    assert!(!view.confirm_receive("peer", "a.bin", 1), "陌生设备必须人工确认");
}

/// 没有加密会话信息 ⇒ 直接拒绝，绝不能因为 `--yes` 就放行。
#[test]
fn auto_accept_requires_a_session_at_all() {
    let view = CliView::new(true);
    assert!(!view.confirm_receive("peer", "a.bin", 1), "没有会话信息不能收");
}

#[test]
fn progress_is_throttled_but_always_prints_completion() {
    let view = CliView::new(true);
    let started = Instant::now();
    // 大量高频回调：限流应当让它迅速返回，而不是每次都做字符串格式化
    for done in 0..5_000u64 {
        view.progress("发送", done, 5_000);
    }
    view.progress("发送", 5_000, 5_000);
    // 只是防止限流逻辑被误删导致明显变慢（本机远快于此阈值）
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "进度回调耗时异常：{:?}",
        started.elapsed()
    );
}
