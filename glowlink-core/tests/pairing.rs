//! `controller/pairing.rs` 的测试：真实回环 TCP 上的配对会话。
//!
//! 为什么不用内存通道：配对要证明的正是「真连接上握手 → 一侧显示码 → 另一侧输码」
//! 这条链路，用假通道会把 TCP 层的读超时、连接断开这些真实行为绕过去。
//!
//! 两个会话对象都不能跨线程（内含 `Box<dyn TransferTransport>`），
//! 所以接收方整段逻辑放在线程里跑，只把**数据**（校验码/指纹/结论）经 channel 传回。

use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use glowlink_core::controller::pairing::{CodeAttempts, PairingSession, MAX_CODE_ATTEMPTS};
use glowlink_core::controller::transfer::TransferListener;
use glowlink_core::model::crypto::identity::Identity;
use glowlink_core::model::error::GlowError;
use glowlink_core::patterns::adapter::{TcpListenerAdapter, TcpTransportAdapter};
use glowlink_core::patterns::secure::{NoTrust, TrustList, session_slot};

/// 只认一个指纹的信任列表：用来验证「已信任」这一路。
struct TrustOnly(String);

impl TrustList for TrustOnly {
    fn is_trusted(&self, fingerprint: &str) -> bool {
        fingerprint == self.0
    }

    fn name_of(&self, _fingerprint: &str) -> Option<String> {
        Some("老朋友".into())
    }
}

/// 接收方线程里要做的事。
enum Answer {
    /// 用「它自己算出来的码」同意（正常配对）。
    ApproveWithOwnCode,
    /// 先输错一次、再用正确的码同意（验证「输错不废掉会话」）。
    ApproveWrongThenRight,
    /// 输错到次数用尽后显式拒绝。
    ExhaustThenDeny,
    /// 直接拒绝。
    Deny,
}

/// 接收方看到的东西。
struct ResponderReport {
    /// 接收方自己算出的 6 位码。
    code: String,
    /// 接收方自己的指纹（用来证明发起方记下的确实是它的）。
    own_fingerprint: String,
    /// 发起方自报的设备名。
    requested_by: String,
    /// 是否同意了。
    approved: bool,
    /// 接收方自己那一侧看到的「是否已信任对方」。
    self_trusts_peer: bool,
}

/// 起一次真实回环配对：返回（发起方结果, 接收方报告）。
///
/// 两个 `trusts_peer` 分开传是有意的：**本机信任对方 ≠ 对方信任本机**，
/// 两侧各自查自己的信任列表，谁都不把对方的说法当真。
/// 造一个**保证不同**的 6 位码：拿真实码逐位加 1（仍在 0-9 内）。
fn wrong_code(code: &str) -> String {
    let wrong: String = code
        .chars()
        .map(|c| c.to_digit(10).map(|d| (d + 1) % 10).unwrap_or_default())
        .map(|d| d.to_string())
        .collect();
    assert_ne!(wrong, code, "用来测「输错」的码必须真的不同");
    wrong
}

fn pair(
    answer: Answer,
    initiator_trusts_peer: bool,
    responder_trusts_peer: bool,
) -> (
    Result<glowlink_core::PairingOutcome, GlowError>,
    ResponderReport,
) {
    let listener = TcpListenerAdapter::bind(0).expect("应能绑到一个回环端口");
    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", listener.local_port().unwrap())
        .parse()
        .unwrap();

    let responder_identity = Arc::new(Identity::generate());
    let responder_fingerprint = responder_identity.fingerprint();
    let initiator_identity = Arc::new(Identity::generate());
    let initiator_fingerprint = initiator_identity.fingerprint();
    let (report_tx, report_rx) = mpsc::channel::<ResponderReport>();

    // 指纹既要给线程（写进报告），也要留给主线程（配信任列表），故各持一份
    let responder_fingerprint_for_thread = responder_fingerprint.clone();
    let handle = thread::spawn(move || {
        let (transport, _peer) = listener.accept().expect("应能接受连接");
        let trust: Arc<dyn TrustList> = if responder_trusts_peer {
            Arc::new(TrustOnly(initiator_fingerprint))
        } else {
            Arc::new(NoTrust)
        };
        let mut session = PairingSession::accept(transport, responder_identity, trust, session_slot())
            .expect("接收方握手应成功");

        let requested_by = session.recv_request().expect("应能读到配对请求");
        let code = session.code().expect("握手后应有校验码");
        let self_trusts_peer = session.peer().expect("应有会话信息").trusted;

        let approved = match answer {
            Answer::ApproveWithOwnCode => session.approve(&code).expect("应答应发得出去"),
            Answer::ApproveWrongThenRight => {
                let wrong = wrong_code(&code);
                assert!(!session.approve(&wrong).expect("比对不该出错"), "错码不该被接受");
                // 关键：输错之后会话仍然可用，能再用正确的码同意
                session.approve(&code).expect("应答应发得出去")
            }
            Answer::ExhaustThenDeny => {
                let wrong = wrong_code(&code);
                let mut attempts = CodeAttempts::new();
                loop {
                    assert!(!session.approve(&wrong).expect("比对不该出错"), "错码不该被接受");
                    if !attempts.record(false) {
                        break;
                    }
                }
                assert_eq!(attempts.remaining(), 0, "三次之后必须用尽");
                session.deny("校验码不符").expect("拒绝应发得出去");
                false
            }
            Answer::Deny => {
                session.deny("用户拒绝").expect("应答应发得出去");
                false
            }
        };

        report_tx
            .send(ResponderReport {
                code,
                own_fingerprint: responder_fingerprint_for_thread,
                requested_by,
                approved,
                self_trusts_peer,
            })
            .expect("报告应发得回去");
    });

    let initiator_trust: Arc<dyn TrustList> = if initiator_trusts_peer {
        Arc::new(TrustOnly(responder_fingerprint))
    } else {
        Arc::new(NoTrust)
    };
    let transport = TcpTransportAdapter::connect(addr, Duration::from_secs(5)).expect("应能连上");
    let mut session =
        PairingSession::initiate(Box::new(transport), initiator_identity, initiator_trust, session_slot())
            .expect("发起方握手应成功");
    session.request("发起机").expect("请求应发得出去");
    let outcome = session.wait();

    handle.join().expect("接收方线程不应 panic");
    let report = report_rx.recv_timeout(Duration::from_secs(5)).expect("应收到报告");
    (outcome, report)
}

#[test]
fn pairing_succeeds_when_code_matches() {
    let (outcome, report) = pair(Answer::ApproveWithOwnCode, false, false);

    let outcome = outcome.expect("码相符时必须配对成功");
    assert!(report.approved);
    assert_eq!(report.requested_by, "发起机", "接收方应看到来访设备名");
    assert_eq!(outcome.code, report.code, "两端必须算出同一个校验码");
    assert_eq!(outcome.code.len(), 6, "码是 6 位");
    // 指纹必须是自己算的、不是对方声称的：用接收方身份独立算一遍来对
    assert_eq!(outcome.peer_fingerprint.len(), 16, "指纹是 16 位十六进制");
    assert!(!outcome.trusted, "没在信任列表里就不该标成已信任");
}

#[test]
fn initiator_records_the_responder_fingerprint() {
    let (outcome, report) = pair(Answer::ApproveWithOwnCode, false, false);
    let outcome = outcome.expect("配对应成功");
    // 发起方记下的必须**就是接收方**的指纹（否则信任列表会存错人）
    assert_eq!(outcome.peer_fingerprint, report.own_fingerprint);
}

#[test]
fn wrong_code_does_not_close_the_session() {
    // 输错一次不该废掉整个配对：对方不必重新拖一次、也不必重新认一个新码
    let (outcome, report) = pair(Answer::ApproveWrongThenRight, false, false);
    let outcome = outcome.expect("输错一次后再输对应能配对成功");
    assert!(report.approved, "第二次输对就应当被接受");
    assert_eq!(outcome.code, report.code);
}

#[test]
fn exhausting_attempts_then_denying_reaches_the_initiator() {
    let (outcome, report) = pair(Answer::ExhaustThenDeny, false, false);
    assert!(!report.approved, "次数用尽后不能同意");
    let error = outcome.expect_err("次数用尽后发起方必须收到拒绝");
    assert!(matches!(error, GlowError::Rejected(_)), "实际：{error:?}");
    assert!(error.to_string().contains("校验码"), "原因要说清是码不符：{error}");
}

#[test]
fn explicit_deny_reaches_the_initiator_as_rejected() {
    let (outcome, report) = pair(Answer::Deny, false, false);
    assert!(!report.approved);
    let error = outcome.expect_err("被拒绝时不该配对成功");
    assert!(matches!(error, GlowError::Rejected(_)), "实际：{error:?}");
    assert!(error.to_string().contains("用户拒绝"), "应带回拒绝原因：{error}");
}

#[test]
fn trust_is_decided_locally_on_each_side() {
    // 关键不变量：**本机信任对方 ≠ 对方信任本机**。两侧各查自己的信任列表，
    // 谁都不把对方的说法当真，否则一侧的信任就能替另一侧做决定。
    let (outcome, report) = pair(Answer::ApproveWithOwnCode, true, false);
    let outcome = outcome.expect("配对应成功");
    assert!(outcome.trusted, "发起方信任对方时应标成已信任");
    assert!(!report.self_trusts_peer, "接收方没信任发起方，就不该被发起方那一侧带跑");

    // 反过来也一样
    let (outcome, report) = pair(Answer::ApproveWithOwnCode, false, true);
    let outcome = outcome.expect("配对应成功");
    assert!(!outcome.trusted, "发起方不信任对方时不能因为对方信任自己就标成已信任");
    assert!(report.self_trusts_peer);
}

#[test]
fn code_attempts_run_out_after_three_tries() {
    // 这条规则放在 core 里，因为它决定「还要不要再让用户输一次」，
    // 与界面无关但绝不能各写一份。
    let mut attempts = CodeAttempts::new();
    assert_eq!(attempts.remaining(), MAX_CODE_ATTEMPTS);
    assert!(attempts.record(false), "第 1 次输错后还能再试");
    assert!(attempts.record(false), "第 2 次输错后还能再试");
    assert!(!attempts.record(false), "第 3 次输错后必须作废");
    assert_eq!(attempts.remaining(), 0);
    assert!(!attempts.record(false), "作废之后不能再试");
}

#[test]
fn correct_code_does_not_consume_attempts() {
    let mut attempts = CodeAttempts::new();
    assert!(attempts.record(true), "输对了当然还能继续");
    assert_eq!(attempts.remaining(), MAX_CODE_ATTEMPTS, "输对不该扣次数");
}

#[test]
fn relayed_code_cannot_pass_the_third_party() {
    // 中间人把一条连接终结成两段（I—M 与 M—R）。它能拿到**两段各自的码**，
    // 但它改不了双方的抄本：I 屏幕上显示的码与 R 派生的码必然不同。
    // 而 R 那侧要输入的码来自 I 的屏幕（人类信道），M 注不进去——
    // 这就是「一侧显示、一侧输入」能挡住中间人的全部理由。
    let r_listener = TcpListenerAdapter::bind(0).expect("接收方要能先监听");
    let r_addr: std::net::SocketAddr =
        format!("127.0.0.1:{}", r_listener.local_port().unwrap()).parse().unwrap();
    let (relay_tx, relay_rx) = mpsc::channel::<String>(); // M → R：转发的码
    let (verdict_tx, verdict_rx) = mpsc::channel::<bool>(); // R → 主线程：是否同意
    let (r_code_tx, r_code_rx) = mpsc::channel::<String>(); // R → 主线程：R 自己的码

    let r_identity = Arc::new(Identity::generate());
    let r_thread = thread::spawn(move || {
        let (transport, _peer) = r_listener.accept().expect("应能接受中间人的连接");
        let mut session =
            PairingSession::accept(transport, r_identity, Arc::new(NoTrust), session_slot())
                .expect("接收方握手应成功");
        let _ = session.recv_request().expect("应能读到配对请求");
        r_code_tx.send(session.code().expect("应有校验码")).unwrap();
        // 人来输：输的是 I 屏幕上那个码（被中间人转发过来的）
        let relayed = relay_rx.recv_timeout(Duration::from_secs(5)).expect("应收到转发的码");
        verdict_tx.send(session.approve(&relayed).expect("应答应发得出去")).unwrap();
    });

    let m_listener = TcpListenerAdapter::bind(0).expect("中间人要能监听");
    let m_addr: std::net::SocketAddr =
        format!("127.0.0.1:{}", m_listener.local_port().unwrap()).parse().unwrap();
    let (m_code_tx, m_code_rx) = mpsc::channel::<String>();
    let m_thread = thread::spawn(move || {
        // 第一段：冒充接收方与 I 握手
        let (transport, _peer) = m_listener.accept().expect("应能接受 I 的连接");
        let mut towards_i = PairingSession::accept(
            transport,
            Arc::new(Identity::generate()),
            Arc::new(NoTrust),
            session_slot(),
        )
        .expect("中间人在 I 这侧握手应成功");
        let _ = towards_i.recv_request().expect("应能读到 I 的配对请求");
        let code_towards_i = towards_i.code().expect("应有校验码");
        relay_tx.send(code_towards_i.clone()).unwrap();
        m_code_tx.send(code_towards_i).unwrap();

        // 第二段：冒充发起方与 R 握手。两段的临时密钥不同，所以它算不出同一个码。
        let transport = TcpTransportAdapter::connect(r_addr, Duration::from_secs(5))
            .expect("应能连上接收方");
        let mut towards_r = PairingSession::initiate(
            Box::new(transport),
            Arc::new(Identity::generate()),
            Arc::new(NoTrust),
            session_slot(),
        )
        .expect("中间人在 R 这侧握手应成功");
        towards_r.request("中间人").expect("请求应发得出去");
        // 它拿不到合法的同意：R 会按人类输入的码下判断
        let _ = towards_r.wait();
    });

    // I：连到中间人，发出配对请求后就等对方的结论
    let transport = TcpTransportAdapter::connect(m_addr, Duration::from_secs(5)).expect("应能连上");
    let mut initiator = PairingSession::initiate(
        Box::new(transport),
        Arc::new(Identity::generate()),
        Arc::new(NoTrust),
        session_slot(),
    )
    .expect("I 侧握手应成功");
    initiator.request("发起机").expect("请求应发得出去");
    let code_towards_i = initiator.code().expect("应有校验码");

    let code_towards_r = r_code_rx.recv_timeout(Duration::from_secs(5)).expect("应收到 R 的码");
    let relayed = m_code_rx.recv_timeout(Duration::from_secs(5)).expect("应收到被转发的码");
    assert_eq!(relayed, code_towards_i, "被转发的就是 I 屏幕上那个码");
    assert_ne!(
        code_towards_i, code_towards_r,
        "两端派生的码必须不同——这正是中间人过不去的原因"
    );
    assert!(
        !verdict_rx.recv_timeout(Duration::from_secs(5)).expect("应收到 R 的结论"),
        "转发过来的码在接收方必然对不上"
    );

    drop(initiator);
    m_thread.join().expect("中间人线程不应 panic");
    r_thread.join().expect("接收方线程不应 panic");
}

#[test]
fn detect_routes_pair_request_to_pairing_and_keeps_the_frame_for_transfer() {
    use glowlink_core::controller::transfer::TransferTransport;
    use glowlink_core::model::packet::{Frame, Hello};
    use glowlink_core::patterns::secure::SecureTransport;
    use glowlink_core::{detect, Intent, Preloaded};

    // 同一个端口上的两种意图：差别只在握手后的第一帧
    fn one_shot(first: Frame) -> Result<Intent, GlowError> {
        let listener = TcpListenerAdapter::bind(0)?;
        let addr: std::net::SocketAddr =
            format!("127.0.0.1:{}", listener.local_port()?).parse().unwrap();
        let responder_identity = Arc::new(Identity::generate());
        let (intent_tx, intent_rx) = mpsc::channel::<(Intent, Frame)>();

        let handle = thread::spawn(move || {
            let (transport, _peer) = listener.accept().expect("应能接受连接");
            let mut secure = SecureTransport::responder(
                transport,
                responder_identity,
                Arc::new(NoTrust),
                session_slot(),
            );
            let (intent, frame) = detect(&mut secure).expect("应能判定意图");
            intent_tx.send((intent.clone(), frame.clone())).unwrap();
            // 传输路径：把第一帧放回给下游，拿到的必须是同一帧
            if intent == Intent::Transfer {
                let mut preloaded = Preloaded::new(secure, frame.clone());
                assert_eq!(preloaded.recv_frame().unwrap(), frame, "放回的必须是同一帧");
                let _ = preloaded.send_frame(&Frame::Accept);
            }
        });

        let transport = TcpTransportAdapter::connect(addr, Duration::from_secs(5))?;
        let mut secure = SecureTransport::initiator(
            Box::new(transport),
            Arc::new(Identity::generate()),
            Arc::new(NoTrust),
            session_slot(),
        );
        secure.handshake()?;
        secure.send_frame(&first)?;
        let (intent, _frame) = intent_rx.recv_timeout(Duration::from_secs(5)).expect("应收到意图");
        handle.join().expect("线程不应 panic");
        Ok(intent)
    }

    let hello = Hello {
        id: "abc".into(),
        name: "甲机".into(),
        port: 1,
        reply_port: 0,
        kind: glowlink_core::DeviceKind::Unknown,
    };

    assert_eq!(
        one_shot(Frame::PairRequest { name: "发起机".into() }).unwrap(),
        Intent::Pair { name: "发起机".into() },
        "配对请求必须被分到配对路径"
    );
    assert_eq!(
        one_shot(Frame::Offer { name: "报告.pdf".into(), size: 12 }).unwrap(),
        Intent::Transfer,
        "Offer 必须被分到传输路径（且第一帧不能丢）"
    );
    // 顺手确认另一种明文帧也不会被误判成配对
    assert_eq!(one_shot(Frame::Hello(hello)).unwrap(), Intent::Transfer);
}
