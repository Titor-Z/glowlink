//! `glowlink-cli/src/sendqueue.rs`：发送队列（FIFO + 顺序执行的基础）。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use glowlink_cli::sendqueue::{Item, SendQueue};
use glowlink_cli::transfers::Transfers;
use glowlink_ipc::row::TransferState;

fn item(transfers: &Transfers, path: &str) -> Item {
    Item {
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000),
        peer: "甲机".into(),
        path: PathBuf::from(path),
        job: transfers.start_queued(path, "甲机", 1024),
    }
}

#[test]
fn the_queue_is_first_in_first_out() {
    // 一次投放 = 一批：顺序必须是用户拖进来的顺序（不能并行、不能乱序）
    let transfers = Transfers::new();
    let queue = SendQueue::new();
    queue.push(item(&transfers, "1.mov"));
    queue.push(item(&transfers, "2.mov"));
    queue.push(item(&transfers, "3.mov"));
    assert_eq!(queue.len(), 3);

    let mut order = Vec::new();
    while let Some(next) = queue.try_pop() {
        order.push(next.path.display().to_string());
    }
    assert_eq!(order, vec!["1.mov", "2.mov", "3.mov"]);
    assert!(queue.is_empty());
}

#[test]
fn a_queued_item_says_waiting_not_running() {
    // 「等待中」与「正在传」是两回事：前者数字不会动，文案也不一样
    let transfers = Transfers::new();
    let job = transfers.start_queued("大文件.mov", "甲机", 4096);
    let row = transfers.rows().pop().expect("应有登记");
    assert_eq!(row.state, TransferState::Queued);
    assert_eq!(row.done, 0);
    assert!(row.code.is_empty(), "还没握手就没有码");

    // 轮到它了：转成「正在传」，并记下本次握手的码
    job.mark_running();
    job.set_code("513904");
    let row = transfers.rows().pop().expect("应有登记");
    assert_eq!(row.state, TransferState::Running);
    assert_eq!(row.code, "513904", "码跟着行给界面");
}

#[test]
fn canceling_a_queued_item_sticks() {
    // 排队期间点 ✕：状态落成「已取消」，工作线程取到它时会跳过（不假装成功）
    let transfers = Transfers::new();
    let queue = SendQueue::new();
    let queued = item(&transfers, "取消这个.mov");
    let token = queued.job.token();
    queue.push(queued);

    assert_eq!(transfers.cancel(&token).as_deref(), Some(token.as_str()));
    let row = transfers.rows().pop().expect("应有登记");
    assert_eq!(row.state, TransferState::Canceled);

    let next = queue.try_pop().expect("还在队列里");
    assert!(next.job.is_canceled(), "工作线程据此跳过它");
}
