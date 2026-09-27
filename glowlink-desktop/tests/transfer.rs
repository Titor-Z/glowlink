//! `model/transfer.rs`：传输卡的逻辑（速度、选哪条、卡上写什么）。

use std::time::{Duration, Instant};

use glowlink_desktop::model::transfer::{action, numbers, pick, Action, SpeedMeter};
use glowlink_ipc::row::{TransferDirection, TransferRow, TransferState};

fn row(done: u64, total: u64, state: TransferState) -> TransferRow {
    TransferRow {
        token: "001".into(),
        direction: TransferDirection::Receive,
        name: "大文件.bin".into(),
        peer: "iPad mini".into(),
        done,
        total,
        state,
        note: String::new(),
    }
}

#[test]
fn speed_comes_from_a_one_second_window_not_the_whole_average() {
    // 中途掉速时全程平均会长时间停在旧的高值上（CLI 那边就是这么栽的），
    // 所以这里必须是滑动窗口：窗口一滑走，速度就掉下来。
    let start = Instant::now();
    let mut meter = SpeedMeter::default();
    // 第 1 秒：1 MB/s
    for step in 1..=10u64 {
        meter.sample(step * 100 * 1024, start + Duration::from_millis(step * 100));
    }
    let fast = meter.speed();
    assert!(fast > 900.0 * 1024.0 && fast < 1100.0 * 1024.0, "第一秒该是 1MB/s：{fast:.0}");
    // 第 2 秒：原地不动（字节数不再增长）
    let mut stalled = 0.0;
    for step in 11..=30u64 {
        stalled = meter.sample(1024 * 1024, start + Duration::from_millis(step * 100));
    }
    assert!(stalled < 50.0 * 1024.0, "停了就该掉到 0 附近：{stalled:.0}");
}

#[test]
fn a_stalled_transfer_reports_zero_instead_of_the_old_peak() {
    let start = Instant::now();
    let mut meter = SpeedMeter::default();
    meter.sample(0, start);
    meter.sample(512 * 1024, start + Duration::from_millis(200));
    assert!(meter.speed() > 0.0);
    // 5 秒后还在同一个字节数：窗口早就滑走了
    let after = meter.sample(512 * 1024, start + Duration::from_secs(5));
    assert_eq!(after, 0.0, "不许拿旧峰值骗人");
}

#[test]
fn resetting_the_meter_forgets_the_previous_transfer() {
    // 换了一条传输：字节数不能相加（两条传输各有各的进度）
    let start = Instant::now();
    let mut meter = SpeedMeter::default();
    meter.sample(0, start);
    meter.sample(8 * 1024 * 1024, start + Duration::from_millis(300));
    assert!(meter.speed() > 0.0);
    meter.reset();
    assert_eq!(meter.speed(), 0.0);
}

#[test]
fn a_running_transfer_wins_over_a_finished_one() {
    // 卡片只显示一条：正在跑的优先，完成的留在屏幕上等用户自己关
    let list = vec![
        row(100, 100, TransferState::Done),
        row(10, 100, TransferState::Running),
    ];
    assert_eq!(pick(&list), Some(1));
    let only_done = vec![row(100, 100, TransferState::Done)];
    assert_eq!(pick(&only_done), Some(0), "没有在跑的也要显示最近一条");
    assert_eq!(pick(&[]), None, "空表就别显示卡片");
}

#[test]
fn the_card_numbers_come_from_real_bytes() {
    let running = row(3 * 1024 * 1024, 10 * 1024 * 1024, TransferState::Running);
    let card = numbers(&running, 1024.0 * 1024.0);
    assert!((card.fraction - 0.3).abs() < 0.001);
    assert_eq!(card.percent, "30%");
    // 元信息要有方向、大小、速度、剩余——全都来自真实字节与耗时
    assert!(card.meta.contains("接收中"), "{}", card.meta);
    assert!(card.meta.contains("3.0 MB"), "{}", card.meta);
    assert!(card.meta.contains("1.0 MB/s"), "{}", card.meta);
    assert!(card.meta.contains("剩 7.0 秒"), "{}", card.meta);
}

#[test]
fn a_stalled_transfer_does_not_invent_a_speed_or_an_eta() {
    // 「不编造没有来源的指标」：速度为 0 时既不给速度也不给剩余时间
    let running = row(1024, 4096, TransferState::Running);
    let card = numbers(&running, 0.0);
    assert!(!card.meta.contains("MB/s"), "速度为 0 就不该出现速度：{}", card.meta);
    assert!(!card.meta.contains("剩"), "也别编「剩 0 秒」：{}", card.meta);
    assert!(card.meta.contains("接收中"), "但「在传」这件事要说：{}", card.meta);
}

#[test]
fn unknown_total_does_not_produce_a_percentage() {
    // 总量未知（对端没报大小）时不能瞎算百分比
    let mut running = row(1024, 0, TransferState::Running);
    running.total = 0;
    let card = numbers(&running, 0.0);
    assert_eq!(card.fraction, 0.0);
    assert_eq!(card.percent, "0%");
}

#[test]
fn every_end_state_has_its_own_wording() {
    // 失败与拒绝都必须说清楚，不能只说「结束了」（用户要知道文件到底有没有到）
    let done = numbers(&row(10, 10, TransferState::Done), 0.0);
    assert_eq!(done.percent, "已完成");
    assert_eq!(done.fraction, 1.0);
    let rejected = numbers(&row(3, 10, TransferState::Rejected), 0.0);
    assert_eq!(rejected.percent, "被拒绝");
    let mut failed = row(3, 10, TransferState::Failed);
    failed.note = "磁盘未写入：没有空间".into();
    let failed = numbers(&failed, 0.0);
    assert_eq!(failed.percent, "失败");
    assert_eq!(failed.meta, "磁盘未写入：没有空间", "失败原因要如实转述");
}

#[test]
fn only_received_files_can_be_opened() {
    // 发送方本机就有这个文件，「打开」没有意义
    let mut received = row(10, 10, TransferState::Done);
    received.direction = TransferDirection::Receive;
    assert_eq!(action(&received), Some(Action::OpenFolder));
    let mut sent = row(10, 10, TransferState::Done);
    sent.direction = TransferDirection::Send;
    assert_eq!(action(&sent), None);
    assert_eq!(action(&row(5, 10, TransferState::Running)), None, "没传完不给打开");
}
