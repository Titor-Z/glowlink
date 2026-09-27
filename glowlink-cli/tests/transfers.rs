//! `glowlink-cli/src/transfers.rs` 的单元测试：守护进程侧的传输登记表。
//!
//! 这张表是界面进度的**唯一真相**，所以三件事必须钉住：
//! 先登记（立刻可见）、终态不被后续回调改回进行中、方向与编号稳定。

use std::sync::Arc;

use glowlink_cli::transfers::Transfers;
use glowlink_core::controller::transfer::TransferView;
use glowlink_ipc::row::{TransferDirection, TransferState};

#[test]
fn a_started_transfer_is_visible_before_any_byte_moves() {
    // 先登记再传：界面要能立刻画出「正在传」，而不是等第一块数据到位才冒出来
    let transfers = Transfers::new();
    let job = transfers.start(TransferDirection::Send, "大文件.bin", "甲机", 1000);

    let rows = transfers.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].token, job.token());
    assert_eq!(rows[0].name, "大文件.bin");
    assert_eq!(rows[0].peer, "甲机");
    assert_eq!(rows[0].total, 1000);
    assert_eq!(rows[0].done, 0);
    assert_eq!(rows[0].state, TransferState::Running);
}

#[test]
fn progress_updates_flow_into_the_rows() {
    let transfers = Transfers::new();
    let job = transfers.start(TransferDirection::Send, "a.bin", "甲机", 1000);
    // 走 `TransferView` 这条路（真正被 core 调用的就是它）
    job.progress("a.bin", 400, 1000);
    assert_eq!(transfers.rows()[0].done, 400);

    job.update(900, 1000);
    assert_eq!(transfers.rows()[0].done, 900);
}

#[test]
fn a_finished_transfer_never_goes_back_to_running() {
    // 传输线程可能在收尾后还收到一次回调：那时把「已完成」改回「进行中」，
    // 界面上的进度条会倒着走。
    let transfers = Transfers::new();
    let job = transfers.start(TransferDirection::Send, "a.bin", "甲机", 1000);
    job.update(1000, 1000);
    job.finish();
    job.progress("a.bin", 500, 1000);

    let row = &transfers.rows()[0];
    assert_eq!(row.state, TransferState::Done);
    assert_eq!(row.done, 1000, "完成后再来的回调不该把进度推回去");
}

#[test]
fn rejected_and_failed_are_different_outcomes() {
    // 「对方拒收」是明确结论，「传输失败」才是错误：界面文案不同，不能并成一团
    let transfers = Transfers::new();
    let rejected = transfers.start(TransferDirection::Send, "a.bin", "甲机", 100);
    rejected.reject("对方未确认");
    let failed = transfers.start(TransferDirection::Receive, "b.bin", "乙机", 100);
    failed.fail("磁盘写满");

    let rows = transfers.rows();
    assert_eq!(rows[0].state, TransferState::Rejected);
    assert_eq!(rows[0].note, "对方未确认");
    assert_eq!(rows[1].state, TransferState::Failed);
    assert_eq!(rows[1].note, "磁盘写满");
    assert_eq!(rows[1].direction, TransferDirection::Receive);
}

#[test]
fn tokens_are_unique_and_keep_their_order() {
    // 编号是界面把卡片与后续指令对上的钥匙：重复或乱序都会让人指错对象
    let transfers = Transfers::new();
    let first = transfers.start(TransferDirection::Send, "a.bin", "甲机", 1);
    let second = transfers.start(TransferDirection::Send, "b.bin", "乙机", 1);
    let third = transfers.start(TransferDirection::Receive, "c.bin", "丙机", 1);

    let tokens = vec![first.token(), second.token(), third.token()];
    assert_eq!(tokens, vec!["001", "002", "003"]);
    let rows = transfers.rows();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].name, "c.bin");
}

#[test]
fn finished_transfers_are_kept_for_a_while_then_dropped() {
    // 保留是为了让界面看到终态（不然进度条在 99% 处直接消失）。
    // 这里不去等 30 秒，只钉住「刚结束的还在表里」这一半；
    // 超时清理靠 `KEEP_FINISHED` 一处常量控制。
    let transfers = Transfers::new();
    let job = transfers.start(TransferDirection::Send, "a.bin", "甲机", 10);
    job.finish();
    assert_eq!(transfers.rows().len(), 1, "刚结束的条目必须还能看见");
    assert_eq!(transfers.rows()[0].state, TransferState::Done);
}

#[test]
fn the_send_side_view_never_accepts_an_incoming_file() {
    // 发送路径上凭空冒出「要不要接收」说明视图接错了：
    // 那里必须拒绝，而不是默默放行一个没人看过的文件。
    let transfers = Transfers::new();
    let job = transfers.start(TransferDirection::Send, "a.bin", "甲机", 10);
    assert!(!job.confirm_receive("甲机", "b.bin", 0));
}

#[test]
fn two_threads_can_report_progress_at_once() {
    // 传输线程写进度、界面线程读表：不能互相等死（这是真机上的并发路径）
    let transfers = Arc::new(Transfers::new());
    let job = transfers.start(TransferDirection::Receive, "a.bin", "甲机", 10_000);
    let writer = {
        let job = job.clone();
        std::thread::spawn(move || {
            for step in 1..=100 {
                job.update(step * 100, 10_000);
            }
            job.finish();
        })
    };
    let reader = {
        let transfers = Arc::clone(&transfers);
        std::thread::spawn(move || {
            let mut seen = 0;
            for _ in 0..1000 {
                seen = seen.max(transfers.rows()[0].done);
            }
            seen
        })
    };
    writer.join().unwrap();
    reader.join().unwrap();
    assert_eq!(transfers.rows()[0].state, TransferState::Done);
}

#[test]
fn canceling_marks_the_transfer_and_stops_the_thread() {
    // 桌面版的 ✕ 打到登记表：置标志 + 落「已取消」终态。
    // 传输线程每个分块问 `TransferView::canceled`，所以这里断言的是两件事：
    // 登记表能按编号找到它、并且标志真的会被读到。
    let transfers = Transfers::new();
    let job = transfers.start(TransferDirection::Send, "big.bin", "iPad mini", 4096);
    job.update(1024, 4096);
    assert!(!job.is_canceled(), "一开始当然没取消");

    assert_eq!(transfers.cancel(&job.token()), Some(job.token()));
    assert!(job.is_canceled(), "标志要能被传输线程读到");
    // 取消是**两段式**：登记表只置标志，状态由传输线程落（它才知道传到哪儿了）——
    // 此刻仍显示「进行中」是对的，假装已经停了才是骗人
    assert_eq!(transfers.rows()[0].state, TransferState::Running);
    // 线程在下一个分块看到标志 → 落终态
    job.cancel();
    let rows = transfers.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, TransferState::Canceled);
    job.cancel();
    assert!(job.is_canceled(), "重复取消不出事，状态也不改口");
    assert_eq!(transfers.rows()[0].state, TransferState::Canceled);
}

#[test]
fn canceling_a_finished_or_unknown_transfer_fails_loudly() {
    // 编号不存在 / 已经结束：如实报错，不假装取消成功（界面据此提示用户）
    let transfers = Transfers::new();
    assert_eq!(transfers.cancel("999"), None);
    let job = transfers.start(TransferDirection::Receive, "a.bin", "甲机", 10);
    job.finish();
    assert_eq!(transfers.cancel(&job.token()), None, "已经结束的不该还能取消");
}

#[test]
fn a_canceled_transfer_never_reports_done() {
    // 终态幂等：取消之后传输线程的收尾回调不能把它改回「已完成」
    let transfers = Transfers::new();
    let job = transfers.start(TransferDirection::Send, "big.bin", "乙机", 100);
    job.cancel();
    job.finish();
    let rows = transfers.rows();
    assert_eq!(rows[0].state, TransferState::Canceled, "终态只有第一个算数");
}
