//! `model/format.rs`：字节/速度/时间/百分比 → 界面字符。
//!
//! 这些函数与 CLI 的 `view/cli.rs` 是**两份实现、同一口径**（desktop 不许依赖 cli）。
//! 所以这里的用例是「口径」本身：数字格式、单位切换的边界、不编造指标。

use glowlink_desktop::model::format::{eta_seconds, human_eta, human_size, human_speed, percent};

#[test]
fn sizes_switch_unit_at_the_right_boundaries() {
    // 1 KB = 1024 B（不是 1000），换档点就在 1024 的整数次幂
    assert_eq!(human_size(0), "0 B");
    assert_eq!(human_size(1023), "1023 B");
    assert_eq!(human_size(1024), "1.0 KB");
    assert_eq!(human_size(1024 * 1024 - 1), "1024.0 KB");
    assert_eq!(human_size(1024 * 1024), "1.0 MB");
    assert_eq!(human_size(1024 * 1024 * 1024), "1.00 GB");
}

#[test]
fn speed_never_reads_as_a_stuck_zero() {
    // 低于 1 MB/s 用 KB/s：否则 0.3 MB/s 之类的传输会一直显示「0.0 MB/s」，像卡死了
    assert_eq!(human_speed(0.0), "0 KB/s");
    assert_eq!(human_speed(1024.0), "1 KB/s");
    assert_eq!(human_speed(512.0 * 1024.0), "512 KB/s");
    assert_eq!(human_speed(1024.0 * 1024.0), "1.0 MB/s");
    assert_eq!(human_speed(3.5 * 1024.0 * 1024.0), "3.5 MB/s");
}

#[test]
fn eta_is_not_invented_when_there_is_no_speed() {
    // 速度为 0（停了 / 还没开始）时不许说「剩 0 秒」
    assert_eq!(eta_seconds(1024, 0.0), None);
    assert_eq!(eta_seconds(1024, -1.0), None);
    assert_eq!(eta_seconds(2048, 1024.0), Some(2.0));
}

#[test]
fn human_eta_keeps_seconds_and_minutes_readable() {
    assert_eq!(human_eta(0.0), "不足 0.1 秒");
    assert_eq!(human_eta(0.05), "不足 0.1 秒");
    assert_eq!(human_eta(12.34), "12.3 秒");
    assert_eq!(human_eta(60.0), "1 分 0 秒");
    assert_eq!(human_eta(125.4), "2 分 5 秒");
}

#[test]
fn percent_never_goes_backwards_or_over_a_hundred() {
    // 进度偶尔回跳（快照抖动）时数字不能跟着退，也不能超过 100%
    assert_eq!(percent(0, 100), 0);
    assert_eq!(percent(30, 100), 30);
    assert_eq!(percent(100, 100), 100);
    assert_eq!(percent(120, 100), 100, "超过总量也最多 100%");
    assert_eq!(percent(5, 0), 0, "总量未知时不瞎算");
}
