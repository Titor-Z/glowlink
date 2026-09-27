//! `glowlink-core/src/patterns/secure.rs` 的单元测试。
//!
//! 用**内存管道**当底层通道：可以精确制造「对方发明文」「密文被改」这类场景，
//! 而且所有阻塞读取都带超时——测试绝不允许无限挂住（这是本项目栽过的坑）。

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use glowlink_core::controller::transfer::TransferTransport;
use glowlink_core::model::error::{GlowError, Result};
use glowlink_core::model::packet::Frame;
use glowlink_core::patterns::secure::{session_slot, NoTrust, SecureTransport, TrustList};

use glowlink_core::Identity;

/// 读取等待上限：超过就报错而不是永远等下去。
const WAIT: Duration = Duration::from_secs(3);

/// 内存帧管道的一端：往 `out` 推、从 `inbox` 取。
struct Pipe {
    out: Arc<Queue>,
    inbox: Arc<Queue>,
    /// 为真时把发出去的加密帧里翻一个 bit（模拟链路上被篡改）。
    corrupt: bool,
}

#[derive(Default)]
struct Queue {
    frames: Mutex<VecDeque<Frame>>,
    ready: Condvar,
}

impl Queue {
    fn push(&self, frame: Frame) {
        let mut guard = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        guard.push_back(frame);
        self.ready.notify_all();
    }

    fn pop(&self, timeout: Duration) -> Result<Frame> {
        let deadline = Instant::now() + timeout;
        let mut guard = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(frame) = guard.pop_front() {
                return Ok(frame);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(GlowError::Protocol("测试管道等待超时".into()));
            }
            let (next, _) = self
                .ready
                .wait_timeout(guard, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            guard = next;
        }
    }
}

impl TransferTransport for Pipe {
    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        if self.corrupt {
            if let Frame::SecureData(payload) = frame {
                let mut damaged = payload.clone();
                damaged[0] ^= 0x01;
                self.out.push(Frame::SecureData(damaged));
                return Ok(());
            }
        }
        self.out.push(frame.clone());
        Ok(())
    }

    fn recv_frame(&mut self) -> Result<Frame> {
        self.inbox.pop(WAIT)
    }
}

/// 造一对相连的管道。
fn pair() -> (Box<dyn TransferTransport>, Box<dyn TransferTransport>) {
    let (ours, theirs, _) = linked();
    (Box::new(ours), Box::new(theirs))
}

/// 造一对相连的管道，并把「我方的收件队列」「对方的收件队列」也交出来，
/// 以便测试直接扮演一个**根本不参与加密**的原始对端。
fn linked() -> (Pipe, Pipe, (Arc<Queue>, Arc<Queue>)) {
    let to_a = Arc::new(Queue::default());
    let to_b = Arc::new(Queue::default());
    (
        Pipe { out: Arc::clone(&to_b), inbox: Arc::clone(&to_a), corrupt: false },
        Pipe { out: Arc::clone(&to_a), inbox: Arc::clone(&to_b), corrupt: false },
        (Arc::clone(&to_a), Arc::clone(&to_b)),
    )
}

/// 造一对相连的管道，其中「我方发出的加密帧」在链路上会被篡改。
fn pair_with_corruption() -> (Box<dyn TransferTransport>, Box<dyn TransferTransport>) {
    let (ours, theirs) = {
        let (mut a, b, _) = linked();
        a.corrupt = true;
        (a, b)
    };
    (Box::new(ours), Box::new(theirs))
}

/// 信任某一把指纹的白名单。
struct TrustOnly(String);

impl TrustList for TrustOnly {
    fn is_trusted(&self, fingerprint: &str) -> bool {
        self.0 == fingerprint
    }
}

/// 在两条线程里各跑一次操作（两端动作对称，谁先触发都行）。
fn both_ways<A, B>(left: A, right: B) -> (Result<()>, Result<()>)
where
    A: FnOnce() -> Result<()> + Send + 'static,
    B: FnOnce() -> Result<()> + Send + 'static,
{
    let right_handle = std::thread::spawn(right);
    let left_result = left();
    let right_result = right_handle.join().expect("对端线程不应 panic");
    (left_result, right_result)
}

#[test]
fn handshake_completes_and_share_the_same_code() {
    let (a_pipe, b_pipe) = pair();
    let (alice, bob) = (Arc::new(Identity::generate()), Arc::new(Identity::generate()));
    let (a_slot, b_slot) = (session_slot(), session_slot());

    let mut alice_transport = SecureTransport::initiator(a_pipe, Arc::clone(&alice), Arc::new(NoTrust), Arc::clone(&a_slot));
    let mut bob_transport = SecureTransport::responder(b_pipe, Arc::clone(&bob), Arc::new(NoTrust), Arc::clone(&b_slot));

    // 双方各发一帧，帧里带内容用来验证「加密后仍能原样解出」
    let (left, right) = both_ways(
        move || {
            alice_transport.send_frame(&Frame::Offer { name: "报告.pdf".into(), size: 2048 })?;
            match alice_transport.recv_frame()? {
                Frame::Accept => Ok(()),
                other => Err(GlowError::Protocol(format!("期望 Accept，收到 {other:?}"))),
            }
        },
        move || {
            match bob_transport.recv_frame()? {
                Frame::Offer { name, size } => {
                    assert_eq!(name, "报告.pdf");
                    assert_eq!(size, 2048);
                }
                other => return Err(GlowError::Protocol(format!("期望 Offer，收到 {other:?}"))),
            }
            bob_transport.send_frame(&Frame::Accept)
        },
    );
    left.expect("发起方应成功");
    right.expect("响应方应成功");

    let a_info = a_slot.lock().unwrap().clone().expect("发起方应记录会话信息");
    let b_info = b_slot.lock().unwrap().clone().expect("响应方应记录会话信息");

    assert_eq!(a_info.code, b_info.code, "两端校验码必须一致，否则用户无法比对");
    assert_eq!(a_info.code.len(), 6);
    assert_eq!(a_info.peer_fingerprint, bob.fingerprint(), "对端指纹应指向对方身份");
    assert_eq!(b_info.peer_fingerprint, alice.fingerprint());
    assert!(a_info.initiator && !b_info.initiator, "角色应分别标记");
    assert!(!a_info.trusted, "谁都不信任时不麻该标成已信任");
}

#[test]
fn trusted_fingerprint_is_flagged_for_auto_accept() {
    let (a_pipe, b_pipe) = pair();
    let (alice, bob) = (Arc::new(Identity::generate()), Arc::new(Identity::generate()));
    // 响应方信任发起方的指纹
    let trust: Arc<dyn TrustList> = Arc::new(TrustOnly(alice.fingerprint()));
    let (a_slot, b_slot) = (session_slot(), session_slot());

    let mut alice_transport = SecureTransport::initiator(a_pipe, Arc::clone(&alice), Arc::new(NoTrust), Arc::clone(&a_slot));
    let mut bob_transport = SecureTransport::responder(b_pipe, Arc::clone(&bob), trust, Arc::clone(&b_slot));

    let (left, right) = both_ways(
        move || {
            alice_transport.send_frame(&Frame::Reject { reason: "测试".into() })?;
            alice_transport.recv_frame()?;
            Ok::<(), GlowError>(())
        },
        move || {
            bob_transport.recv_frame()?;
            bob_transport.send_frame(&Frame::Accept)?;
            Ok::<(), GlowError>(())
        },
    );
    left.expect("发起方应成功");
    right.expect("响应方应成功");

    let b_info = b_slot.lock().unwrap().clone().unwrap();
    assert!(b_info.trusted, "已信任的指纹要能免确认自动接收");
    assert!(!a_slot.lock().unwrap().clone().unwrap().trusted, "发起方没信任任何人");
}

#[test]
fn plaintext_offer_is_refused_no_downgrade() {
    // 对端完全不参与加密：收到我方 hello 后直接回一个明文 Offer。
    // 我方必须明确报「期望加密握手」而不是「那就算了，用明文吧」。
    let (ours, _theirs, (to_us, to_them)) = linked();
    let identity = Arc::new(Identity::generate());
    let mut secure =
        SecureTransport::responder(Box::new(ours), identity, Arc::new(NoTrust), session_slot());

    to_us.push(Frame::Offer { name: "攻击者".into(), size: 1 });
    let error = secure.send_frame(&Frame::Accept).expect_err("明文回应必须被拒");
    let text = error.to_string();
    assert!(text.contains("明文"), "应指出收到的是明文帧，实际：{text}");

    // 我方在明文阶段只应发出握手 hello，不能泄露 Accepted/Offer 之类的内容
    let sent = to_them.pop(Duration::from_millis(200)).expect("应发出握手 hello");
    assert!(
        matches!(sent, Frame::SecureHello(_)),
        "握手阶段只允许发明文 SecureHello，实际发了 {sent:?}"
    );
    assert!(to_them.pop(Duration::from_millis(50)).is_err(), "不应额外泄露其它帧");
}

#[test]
fn tampered_ciphertext_is_rejected() {
    // 链路上把加密帧翻一个 bit：对端必须认证失败，绝不能「解出垃圾明文」
    let (a_pipe, b_pipe) = pair_with_corruption();
    let (alice, bob) = (Arc::new(Identity::generate()), Arc::new(Identity::generate()));
    let (a_slot, b_slot) = (session_slot(), session_slot());
    let mut alice =
        SecureTransport::initiator(a_pipe, alice, Arc::new(NoTrust), Arc::clone(&a_slot));
    let mut bob =
        SecureTransport::responder(b_pipe, bob, Arc::new(NoTrust), Arc::clone(&b_slot));

    // 握手帧是明文（不经过加密），不会被篡改；握手应能完成
    let (left, right) = both_ways(
        move || {
            let sent = alice.send_frame(&Frame::Offer { name: "被篡改".into(), size: 9 });
            // 对方若因认证失败断开，发送方可能已经写成功，这里只要求不 panic
            let _ = sent;
            Ok::<(), GlowError>(())
        },
        move || bob.recv_frame().map(|_| ()),
    );
    left.expect("发起方线程不应出错");
    let error = right.expect_err("被篡改的帧必须被拒");
    assert!(
        matches!(error, GlowError::Crypto(_)),
        "应是加密/认证错误，实际：{error}"
    );
    assert!(a_slot.lock().unwrap().is_some(), "握手本身应已成功");
}

#[test]
fn unencrypted_first_frame_never_reaches_transfer() {
    // 会话未建立、对方直接发明文 Reject：必须**立刻**拒绝（而不是等到读超时），
    // 且会话槽保持为空——否则上层会以为这是一次合法连接。
    let (ours, _theirs, (to_us, _to_them)) = linked();
    let slot = session_slot();
    let mut secure = SecureTransport::responder(
        Box::new(ours),
        Arc::new(Identity::generate()),
        Arc::new(NoTrust),
        Arc::clone(&slot),
    );

    to_us.push(Frame::Reject { reason: "伪造".into() });
    let started = Instant::now();
    let error = secure.recv_frame().expect_err("明文首帧必须被拒");
    assert!(started.elapsed() < WAIT, "应是当场拒绝，而不是等到读超时：{error}");
    assert!(error.to_string().contains("明文"), "实际：{error}");
    assert!(slot.lock().unwrap().is_none(), "握手未完成不应留下会话信息");
}

#[test]
fn two_sessions_of_the_same_peers_use_different_keys() {
    // 同一对设备连两次：校验码与会话密钥都应变化（前向保密）
    let (alice, bob) = (Arc::new(Identity::generate()), Arc::new(Identity::generate()));
    let mut codes = Vec::new();

    for _ in 0..2 {
        let (a_pipe, b_pipe) = pair();
        let (a_slot, b_slot) = (session_slot(), session_slot());
        let mut a = SecureTransport::initiator(a_pipe, Arc::clone(&alice), Arc::new(NoTrust), Arc::clone(&a_slot));
        let mut b = SecureTransport::responder(b_pipe, Arc::clone(&bob), Arc::new(NoTrust), Arc::clone(&b_slot));

        let (left, right) = both_ways(
            move || {
                a.send_frame(&Frame::Accept)?;
                a.recv_frame()?;
                Ok::<(), GlowError>(())
            },
            move || {
                b.recv_frame()?;
                b.send_frame(&Frame::Accept)?;
                Ok::<(), GlowError>(())
            },
        );
        left.expect("会话应成功");
        right.expect("会话应成功");
        codes.push(a_slot.lock().unwrap().clone().unwrap().code);
    }
    assert_ne!(codes[0], codes[1], "每次会话的校验码都应不同");
}
