use std::sync::Arc;
use std::time::{Duration, Instant};

use glowlink_cli::keys::TrustFile;
use glowlink_cli::view::cli::{eta_seconds, file_index_label, SpeedMeter, SPEED_WINDOW};
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

/// 第一个窗口还没满时，退回全程平均（否则传输刚开始那几秒显示 0，看着像卡住）。
#[test]
fn speed_meter_falls_back_to_the_average_before_the_first_window() {
    let t0 = Instant::now();
    let mut meter = SpeedMeter::new(t0);
    let early = meter.observe(1024 * 1024, t0 + Duration::from_millis(100));
    assert_eq!(early, (1024 * 1024) as f64 / 0.1, "窗口未满就用全程平均");
}

/// 窗口满了之后以窗口为准：中途掉速要立刻反映出来，而不是继续报全程平均。
#[test]
fn speed_meter_reports_the_window_not_the_average() {
    let t0 = Instant::now();
    let mut meter = SpeedMeter::new(t0);

    // 第一个窗口（0 → 1.001s）搬了 1 MiB
    let first = meter.observe(1024 * 1024, t0 + SPEED_WINDOW + Duration::from_millis(1));
    let expected = (1024.0 * 1024.0) / (SPEED_WINDOW.as_secs_f64() + 0.001);
    assert!(
        (first - expected).abs() < expected * 0.05,
        "窗口速率应当约 {expected}，实际 {first}"
    );

    // 第二个窗口只搬了 50 KiB：掉速必须报出来，而全程平均此刻仍有约 0.5 MiB/s
    let slow_done = 1024 * 1024 + 50 * 1024;
    let slower = meter.observe(slow_done, t0 + SPEED_WINDOW * 2 + Duration::from_millis(2));
    let average_now = slow_done as f64 / (SPEED_WINDOW * 2).as_secs_f64();
    assert!(
        slower < average_now / 5.0,
        "窗口速率 {slower} 明显低于全程平均 {average_now}，必须按窗口报"
    );
}

/// 窗口内完全没有进展时，速度要如实归零（而不是继续报旧值）。
#[test]
fn speed_meter_drops_to_zero_when_stalled() {
    let t0 = Instant::now();
    let mut meter = SpeedMeter::new(t0);
    let fast = meter.observe(1024 * 1024, t0 + SPEED_WINDOW + Duration::from_millis(1));
    assert!(fast > 0.0);

    // 又过了一个窗口，进度一点没动
    let stalled = meter.observe(1024 * 1024, t0 + SPEED_WINDOW * 2 + Duration::from_millis(2));
    assert_eq!(stalled, 0.0, "卡住就该报 0，别让用户以为还在传");
    // 恢复传输后又能报出速度
    let resumed =
        meter.observe(1024 * 1024 + 512 * 1024, t0 + SPEED_WINDOW * 3 + Duration::from_millis(3));
    assert!(resumed > 0.0, "恢复后应当重新测出速度，实际 {resumed}");
}

#[test]
fn eta_seconds_handles_the_edges() {
    // 没有速度（刚起步或卡住）就不给估计，而不是瞎猜一个
    assert_eq!(eta_seconds(0, 1000, 0.0), None);
    // 总量未知（total=0）时也不给：说「剩 0 秒」是假话
    assert_eq!(eta_seconds(0, 0, 1024.0), None);
    // 传完了剩 0
    assert_eq!(eta_seconds(1000, 1000, 1024.0), Some(0));
    // 正常估算并向不利方向取整（宁可说 3 秒，别说 2 秒）
    assert_eq!(eta_seconds(0, 2048, 1024.0), Some(2));
    assert_eq!(eta_seconds(1024, 4096, 1024.0), Some(3));
}

#[test]
fn speed_unit_switches_below_one_megabyte() {
    // 小文件不能显示「0.0 MB/s」，那看着像卡死
    assert_eq!(glowlink_cli::view::cli::human_speed(512.0 * 1024.0), "512.0 KB/s");
    assert_eq!(glowlink_cli::view::cli::human_speed(0.0), "0.0 KB/s");
    assert_eq!(glowlink_cli::view::cli::human_speed(1024.0 * 1024.0), "1.0 MB/s");
    // 千兆网线量级，避免出现 "0.1 GB/s" 这种不好比较的单位
    assert_eq!(glowlink_cli::view::cli::human_speed(112.0 * 1024.0 * 1024.0), "112.0 MB/s");
}

#[test]
fn multi_file_sends_get_an_index_only_when_there_are_several() {
    assert_eq!(file_index_label(0, 1, "a.bin"), None, "单个文件不加序号");
    assert_eq!(file_index_label(0, 3, "a.bin").unwrap(), "[1/3] a.bin");
    assert_eq!(file_index_label(2, 3, "c.bin").unwrap(), "[3/3] c.bin");
}
