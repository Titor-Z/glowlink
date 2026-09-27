//! `glowlink-cli/src/pairing.rs`：配对槽位表（本机发起的放弃是这次新加的）。

use glowlink_cli::pairing::PairingQueue;
use glowlink_ipc::row::PairingState;

#[test]
fn canceling_a_pairing_marks_it_abandoned_not_failed() {
    // 「本机放弃」必须与「被拒」「失败」分开：用户要知道是自己取消的
    let queue = PairingQueue::new();
    let token = queue.open_outgoing("iPad mini".into(), "123456".into(), None);
    assert_eq!(queue.outgoing_rows()[0].state, PairingState::WaitingPeer);

    assert_eq!(queue.cancel_outgoing(&token).unwrap(), token);
    let row = &queue.outgoing_rows()[0];
    assert_eq!(row.state, PairingState::Abandoned);
    assert_ne!(row.state, PairingState::Failed);
    assert_ne!(row.state, PairingState::Denied);
}

#[test]
fn a_finished_pairing_can_not_be_canceled() {
    // 已经配对成功的槽位再点 ✕ 只该收起卡片，**不许**把状态改掉
    // （否则会把「已配对」改成「已放弃」，信任列表里明明已经写进去了）
    let queue = PairingQueue::new();
    let token = queue.open_outgoing("iPad mini".into(), "123456".into(), None);
    queue.finish_outgoing(&token, PairingState::Paired);
    assert!(queue.cancel_outgoing(&token).is_err(), "已结束的不该还能取消");
    assert_eq!(queue.outgoing_rows()[0].state, PairingState::Paired);
    assert!(queue.cancel_outgoing("999").is_err(), "不存在的编号要报错");
}

#[test]
fn a_canceled_pairing_is_not_overwritten_by_the_worker_thread() {
    // 终态只有第一个算数：用户先点 ✕，后台线程随后拿着「连接被掐断」的错误回来，
    // 不能把「本机已放弃」改口成「失败」
    let queue = PairingQueue::new();
    let token = queue.open_outgoing("iPad mini".into(), "123456".into(), None);
    queue.cancel_outgoing(&token).unwrap();
    queue.finish_outgoing(&token, PairingState::Failed);
    assert_eq!(
        queue.outgoing_rows()[0].state,
        PairingState::Abandoned,
        "先落定的结论才算数（与传输的终态幂等是同一条规矩）"
    );
}
