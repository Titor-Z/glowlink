//! 配对：槽位表 + 两条流程（本机发起、对端发起）。
//!
//! 为什么与待确认接收（`pending.rs`）分开：配对的等待是**双向**的——
//! 本机发起时要等对方拍板，对端发起时要等本机用户拍板；而且用户还要输入 6 位码。
//! 两套语义混进一个队列，会让「accept 到底是收文件还是收配对」变得没有答案。
//!
//! 状态放在守护进程里，界面只是它的前端：配对会话必须由持有 TCP 连接与密钥的
//! 那一方保持着，界面退出不能把配对带走。

use std::collections::VecDeque;
use std::net::TcpStream;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use glowlink_core::model::crypto::identity::Identity;
use glowlink_core::model::error::{GlowError, Result};
use glowlink_core::patterns::adapter::TcpTransportAdapter;
use glowlink_core::patterns::secure::{session_slot, TrustList};
use glowlink_core::{Device, DeviceIdentity, PairingSession, MAX_CODE_ATTEMPTS};
use glowlink_ipc::row::{PairingRow, PairingState};
use glowlink_ipc::state::append_log;

use crate::keys::TrustFile;

/// 建立连接的等待时长（本机网络，超过说明对端没在听）。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 等用户拍板时每次醒来的间隔：切片到期只意味着「去看看对面还在不在」。
const DECISION_SLICE: Duration = Duration::from_millis(200);

/// 已结束的配对槽位保留多久：界面要能看见成败，但不能无限堆积。
const OUTGOING_TTL: Duration = Duration::from_secs(60);

/// 用户对一个待决定配对请求的答复。
enum Decision {
    /// 允许，并带上用户敲进来的 6 位码。
    Allow(String),
    Deny,
}

/// 等待的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// 用户允许并给出了码（**还没校验**，由调用方交给会话比对）。
    Allow(String),
    /// 用户拒绝。
    Deny,
    /// 等超时了（没人管的时候宁可什么都不收）。
    Timeout,
}

struct IncomingSlot {
    token: String,
    peer: String,
    code: String,
    attempts: u8,
    inbox: VecDeque<Decision>,
}

struct OutgoingSlot {
    token: String,
    peer: String,
    code: String,
    state: PairingState,
    finished_at: Option<Instant>,
    /// 这条连接的**另一个句柄**：取消时用它 `shutdown`，让阻塞在等对方答复的
    /// 后台线程立刻醒过来。没有它就只能干等（最长 `pending_timeout_secs`）。
    stream: Option<TcpStream>,
}

#[derive(Default)]
struct Inner {
    /// 单调递增的取号器：编号只用于让人念出来，不与槽位下标绑定。
    issued: u64,
    incoming: Vec<IncomingSlot>,
    outgoing: Vec<OutgoingSlot>,
}

/// 配对槽位表（两个方向共用一个取号器，编号在界面上不会撞）。
#[derive(Default)]
pub struct PairingQueue {
    inner: Mutex<Inner>,
    ready: Condvar,
}

impl PairingQueue {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 取一个新编号（三位数，与待确认接收同一套习惯，便于人念）。
    fn next_token(inner: &mut Inner) -> String {
        inner.issued += 1;
        format!("{:03}", inner.issued)
    }

    /// 对端发起：登记一个等待决定的请求，返回编号。
    pub fn register(&self, peer: String, code: String) -> String {
        let mut inner = self.lock();
        let token = Self::next_token(&mut inner);
        inner.incoming.push(IncomingSlot {
            token: token.clone(),
            peer,
            code,
            attempts: MAX_CODE_ATTEMPTS,
            inbox: VecDeque::new(),
        });
        token
    }

    /// 等用户对某个请求的答复（一次尝试）。
    pub fn wait_decision(&self, token: &str, timeout: Duration) -> Verdict {
        let deadline = Instant::now() + timeout;
        let mut inner = self.lock();
        loop {
            let Some(slot) = inner.incoming.iter_mut().find(|s| s.token == token) else {
                // 槽位被摘掉（用户已落决定、界面已处理）⇒ 当作拒绝
                return Verdict::Deny;
            };
            if let Some(decision) = slot.inbox.pop_front() {
                return match decision {
                    Decision::Allow(code) => Verdict::Allow(code),
                    Decision::Deny => Verdict::Deny,
                };
            }
            let now = Instant::now();
            if now >= deadline {
                return Verdict::Timeout;
            }
            let (guard, _) = self
                .ready
                .wait_timeout(inner, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            inner = guard;
        }
    }

    /// 记一次「码不符」，返回还能不能再试。
    pub fn record_failure(&self, token: &str) -> bool {
        let mut inner = self.lock();
        let Some(slot) = inner.incoming.iter_mut().find(|s| s.token == token) else {
            return false;
        };
        slot.attempts = slot.attempts.saturating_sub(1);
        slot.attempts > 0
    }

    /// 摘掉一个请求槽位（成功、拒绝、超时都走这里）。
    pub fn remove(&self, token: &str) {
        self.lock().incoming.retain(|s| s.token != token);
    }

    /// 本机发起：开一个槽位（结果由后台线程回填），返回编号。
    pub fn open_outgoing(&self, peer: String, code: String, stream: Option<TcpStream>) -> String {
        let mut inner = self.lock();
        let token = Self::next_token(&mut inner);
        inner.outgoing.push(OutgoingSlot {
            token: token.clone(),
            peer,
            code,
            state: PairingState::WaitingPeer,
            finished_at: None,
            stream,
        });
        token
    }

    /// 回填本机发起的配对结果。
    ///
    /// **终态只有第一个算数**：用户先点了 ✕（放弃），后台线程随后拿着
    /// 「连接被掐断」的错误回来，不能把「本机已放弃」改口成「失败」。
    pub fn finish_outgoing(&self, token: &str, state: PairingState) {
        let mut inner = self.lock();
        if let Some(slot) = inner.outgoing.iter_mut().find(|s| s.token == token) {
            if slot.finished_at.is_some() {
                return;
            }
            slot.state = state;
            slot.finished_at = Some(Instant::now());
            slot.stream = None;
        }
    }

    /// 本机放弃一次发起的配对（配对卡上的 ✕）。
    ///
    /// 两件事一起做：① 槽位落成「本机已放弃」（界面据此收起卡片）；
    /// ② 掐掉连接，让后台那个阻塞等答复的线程立刻退出（否则它要等到超时）。
    pub fn cancel_outgoing(&self, token: &str) -> Result<String> {
        let mut inner = self.lock();
        let Some(slot) = inner.outgoing.iter_mut().find(|s| s.token == token) else {
            return Err(GlowError::Missing(format!("没有编号为 {token} 的配对")));
        };
        if slot.finished_at.is_some() {
            return Err(GlowError::Missing(format!("配对 #{token} 已经结束了")));
        }
        slot.state = PairingState::Abandoned;
        slot.finished_at = Some(Instant::now());
        if let Some(stream) = slot.stream.take() {
            // 掐断：等答复的那个线程会立刻拿到错误并退出
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        Ok(token.to_string())
    }

    /// 本机发起的配对槽位（含刚结束的，供界面显示成败）。
    pub fn outgoing_rows(&self) -> Vec<PairingRow> {
        let mut inner = self.lock();
        let now = Instant::now();
        inner.outgoing.retain(|s| match s.finished_at {
            Some(at) => now.duration_since(at) < OUTGOING_TTL,
            None => true,
        });
        inner
            .outgoing
            .iter()
            .map(|s| PairingRow {
                token: s.token.clone(),
                peer: s.peer.clone(),
                code: s.code.clone(),
                state: s.state,
                remaining: 0,
            })
            .collect()
    }

    /// 对端发起、还没落决定的请求。
    pub fn pending_rows(&self) -> Vec<PairingRow> {
        self.lock()
            .incoming
            .iter()
            .map(|s| PairingRow {
                token: s.token.clone(),
                peer: s.peer.clone(),
                code: s.code.clone(),
                state: PairingState::WaitingUser,
                remaining: s.attempts,
            })
            .collect()
    }

    /// 落定用户决定：`code` 为 `Some` 表示允许并提交用户输入的码。
    pub fn decide(&self, token: &str, code: Option<String>) -> Result<String> {
        let mut inner = self.lock();
        let Some(slot) = inner.incoming.iter_mut().find(|s| s.token == token) else {
            return Err(GlowError::Missing(format!(
                "没有编号为 {token} 的配对请求（可能已结束）"
            )));
        };
        slot.inbox
            .push_back(match code {
                Some(code) => Decision::Allow(code),
                None => Decision::Deny,
            });
        drop(inner);
        self.ready.notify_all();
        Ok(token.to_string())
    }
}

/// 配对的两条流程。持有身份与信任列表——配对成功就是往信任列表里写一笔。
pub struct PairingHub {
    queue: Arc<PairingQueue>,
    identity: Arc<Identity>,
    trust: Arc<TrustFile>,
    /// 一次配对最多等多久（沿用 `pending_timeout_secs`）。
    wait: Duration,
}

impl PairingHub {
    pub fn new(identity: Arc<Identity>, trust: Arc<TrustFile>, wait: Duration) -> Arc<Self> {
        Arc::new(PairingHub {
            queue: Arc::new(PairingQueue::new()),
            identity,
            trust,
            wait,
        })
    }

    pub fn queue(&self) -> &Arc<PairingQueue> {
        &self.queue
    }

    /// 这个指纹（就是设备 ID）是否已在信任列表里。
    ///
    /// 界面会问这句：已信任的设备拖动即可配对（免码），陌生的必须走码。
    pub fn is_trusted(&self, fingerprint: &str) -> bool {
        self.trust.contains(fingerprint)
    }

    /// 本机发起配对：连上、握手、登记槽位后立刻返回 `(编号, 校验码)`。
    ///
    /// **立刻返回码**是有意的：界面要第一时间把它显示给人核对，
    /// 等配对结束再回就晚了（那时人已经忘了该核对什么）。
    /// 等待对方输入的过程在后台线程里进行。
    pub fn start(self: &Arc<Self>, device: &Device) -> Result<(String, String)> {
        let stream = TcpStream::connect_timeout(&device.addr, CONNECT_TIMEOUT)?;
        // 给读操作设上限：对端不回答时不能让这个线程无限期挂着
        stream.set_read_timeout(Some(self.wait))?;
        // 留一个句柄给「取消」用（适配器会拿走原 stream，所以先克隆）
        let cancel_handle = stream.try_clone().ok();
        let transport = TcpTransportAdapter::from_stream(stream)?;
        let mut session = PairingSession::initiate(
            Box::new(transport),
            Arc::clone(&self.identity),
            Arc::clone(&self.trust) as Arc<dyn TrustList>,
            session_slot(),
        )?;

        let code = session.code()?;
        let token = self.queue.open_outgoing(device.name.clone(), code.clone(), cancel_handle);
        // 自报的是**显示名**（来自本机身份单例），不是密钥身份
        session.request(&DeviceIdentity::global().name)?;

        let hub = Arc::clone(self);
        let peer_name = device.name.clone();
        let wait_token = token.clone();
        std::thread::spawn(move || {
            let state = match session.wait() {
                Ok(outcome) => {
                    // 指纹来自握手里的静态公钥（不是对方声称的字符串）
                    match hub.trust.trust(&outcome.peer_fingerprint, &peer_name) {
                        Ok(()) => {
                            append_log(&format!(
                                "已与「{peer_name}」配对（指纹 {}，校验码 {}）",
                                outcome.peer_fingerprint, outcome.code
                            ));
                            PairingState::Paired
                        }
                        Err(e) => {
                            append_log(&format!("配对成功但写入信任列表失败：{e}"));
                            PairingState::Failed
                        }
                    }
                }
                Err(GlowError::Rejected(reason)) => {
                    append_log(&format!("与「{peer_name}」配对被拒绝：{reason}"));
                    PairingState::Denied
                }
                Err(e) => {
                    // 用户点了 ✕ 之后这里会拿到「连接被掐断」的错误：
                    // 那不是失败，是取消（槽位已是终态，`finish_outgoing` 会忽略）
                    append_log(&format!("与「{peer_name}」配对未完成：{e}"));
                    PairingState::Failed
                }
            };
            hub.queue.finish_outgoing(&wait_token, state);
        });

        Ok((token, code))
    }

    /// 对端发起配对：登记请求、等用户决定、按结果回帧；同意则写入信任列表。
    ///
    /// 返回是否配对成功。
    pub fn handle_incoming(&self, mut session: PairingSession, peer_name: String) -> Result<bool> {
        let code = session.code()?;
        let token = self.queue.register(peer_name.clone(), code.clone());
        append_log(&format!(
            "配对请求 #{token} 来自「{peer_name}」；本机校验码 {code}"
        ));

        // 先把「等人拍板」跑完，**再无条件摘掉槽位**。
        //
        // 原先是在函数末尾 `remove`，而流程里有多处 `?`（approve / deny 出错就返回），
        // 那些路径会**跳过** remove —— 于是对端一断线就留下一条「等待确认」的幽灵请求，
        // 永远挂着（用户看到的是一屏没人认领的配对卡，重启守护进程才会消失）。
        let outcome = self.await_decision(&mut session, &token);
        self.queue.remove(&token);
        let accepted = outcome?;

        if accepted {
            let peer = session.peer()?;
            self.trust.trust(&peer.peer_fingerprint, &peer_name)?;
            append_log(&format!(
                "已与「{peer_name}」配对（指纹 {}，校验码 {}）",
                peer.peer_fingerprint, peer.code
            ));
        } else {
            append_log(&format!("配对 #{token} 未完成"));
        }
        Ok(accepted)
    }

    /// 等本机用户拍板（允许并输码 / 拒绝 / 超时 / 对方放弃）。
    ///
    /// 只管**判断**，不碰槽位表：摘除由调用方无条件做（见上面的注释）。
    fn await_decision(&self, session: &mut PairingSession, token: &str) -> Result<bool> {
        let deadline = Instant::now() + self.wait;
        // 短切片轮询而不是一觉睡到超时：等的是「用户拍板」，但也可能是
        // **对方放弃了**（关窗走人）。不轮询的话我们这边要傻等到超时。
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let slice = remaining.min(DECISION_SLICE);
            match self.queue.wait_decision(token, slice) {
                Verdict::Allow(typed) => {
                    if session.approve(&typed)? {
                        return Ok(true);
                    }
                    // 码不符：不发拒绝帧，会话保持，让用户再输一次
                    append_log(&format!("配对 #{token}：校验码不符"));
                    if !self.queue.record_failure(token) {
                        append_log(&format!("配对 #{token}：校验码错误次数用尽，已终止"));
                        session.deny("校验码不符")?;
                        return Ok(false);
                    }
                }
                Verdict::Deny => {
                    session.deny("用户拒绝")?;
                    return Ok(false);
                }
                Verdict::Timeout => {
                    // 切片到期 ≠ 总时限到期：只有真的等满了才自动拒绝
                    if Instant::now() >= deadline {
                        session.deny("等待确认超时")?;
                        append_log(&format!("配对 #{token}：等待确认超时，已自动拒绝"));
                        return Ok(false);
                    }
                    if session.peer_closed() {
                        append_log(&format!("配对 #{token}：对方放弃了配对，已收起"));
                        return Ok(false);
                    }
                }
            }
        }
    }
}

/// 对端发来配对、但本端是前台 `serve` 模式（没有常驻队列可问人）时的统一答复。
pub fn refuse_in_serve_mode(session: &mut PairingSession) -> Result<()> {
    session.deny("对端未开启配对（请先运行 glowlink on）")
}
