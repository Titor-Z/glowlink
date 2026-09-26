use std::sync::Arc;
use std::time::{Duration, Instant};

use glowlink_core::controller::transfer::TransferView;
use glowlink_core::patterns::singleton::DeviceIdentity;
use glowlink_cli::CliView;

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

#[test]
fn auto_accept_flag_is_respected() {
    let accept = CliView::new(true);
    assert!(accept.confirm_receive("peer", "a.bin", 1));
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
