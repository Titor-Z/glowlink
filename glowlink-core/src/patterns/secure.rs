//! `SecureTransport`：把任意 [`TransferTransport`] 包成「握手 + 全程加密」的通道。
//!
//! 这是[适配器模式]：对 `transfer` 控制器而言它还是那个 `TransferTransport`，
//! 所以 `transfer.rs` 的阻塞 IO、GLWK 帧、增量 CRC32、背压、`../` 路径剥离
//! **一行都不用改**。
//!
//! 线上顺序（全部在 `Offer` 之前完成）：
//! ```text
//! 双方 → SecureHello(临时公钥+身份公钥+随机数)   ← 明文，因为此时还没有密钥
//! 双方 → SecureProof(身份证明)                    ← 证明自己持有长期私钥
//! 之后 → SecureData(内层帧的 AEAD 密文)            ← Offer/Chunk/Done 全在里面
//! ```
//! 两端都是「先发 hello 再收 hello」，流程对称，谁先触发都不会卡住。
//!
//! **不做降级**：对端若发来明文 `Offer` 而不是 `SecureHello`，直接报错断开。
//! 允许降级等于给中间人一条「请用明文」的通道。

use std::sync::{Arc, Mutex};

use crate::controller::transfer::TransferTransport;
use crate::model::crypto::cipher::FrameCipher;
use crate::model::crypto::handshake::{self, Role, SecureHello};
use crate::model::crypto::identity::{fingerprint_of, Identity};
use crate::model::error::{GlowError, Result};
use crate::model::packet::Frame;

/// 判断某个设备指纹是否已被信任（决定能否免确认自动接收）。
pub trait TrustList: Send + Sync {
    fn is_trusted(&self, fingerprint: &str) -> bool;
    /// 设备显示名（信任列表里存的名字，用于日志/提示）。
    fn name_of(&self, fingerprint: &str) -> Option<String> {
        let _ = fingerprint;
        None
    }
}

/// 谁都不信任：所有连接都需要人工确认。
pub struct NoTrust;

impl TrustList for NoTrust {
    fn is_trusted(&self, _fingerprint: &str) -> bool {
        false
    }
}

/// 一次连接握手后的对端信息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    /// 对端设备指纹。
    pub peer_fingerprint: String,
    /// 对端长期身份公钥。
    pub peer_public: [u8; 32],
    /// 6 位校验码（两端相同，未配对时供用户比对）。
    pub code: String,
    /// 该指纹是否已在信任列表里 ⇒ 免确认。
    pub trusted: bool,
    /// 本端在这次连接里是发起方还是响应方。
    pub initiator: bool,
}

/// 与视图/CLI 共享的会话槽：握手完成后写入，供提示「来自 甲机（校验码 123456）」。
pub type SessionSlot = Arc<Mutex<Option<SessionInfo>>>;

/// 新建一个空的会话槽。
pub fn session_slot() -> SessionSlot {
    Arc::new(Mutex::new(None))
}

enum State {
    /// 还没握手。
    Fresh,
    /// 已握手，双方各自持有发送/接收密钥。
    Ready { send: FrameCipher, recv: FrameCipher },
}

pub struct SecureTransport {
    inner: Box<dyn TransferTransport>,
    identity: Arc<Identity>,
    role: Role,
    trust: Arc<dyn TrustList>,
    session: SessionSlot,
    state: State,
}

impl SecureTransport {
    /// 发起方（TCP 连接方）。
    pub fn initiator(
        inner: Box<dyn TransferTransport>,
        identity: Arc<Identity>,
        trust: Arc<dyn TrustList>,
        session: SessionSlot,
    ) -> Self {
        Self::new(inner, identity, Role::Initiator, trust, session)
    }

    /// 响应方（TCP 接受方）。
    pub fn responder(
        inner: Box<dyn TransferTransport>,
        identity: Arc<Identity>,
        trust: Arc<dyn TrustList>,
        session: SessionSlot,
    ) -> Self {
        Self::new(inner, identity, Role::Responder, trust, session)
    }

    fn new(
        inner: Box<dyn TransferTransport>,
        identity: Arc<Identity>,
        role: Role,
        trust: Arc<dyn TrustList>,
        session: SessionSlot,
    ) -> Self {
        SecureTransport {
            inner,
            identity,
            role,
            trust,
            session,
            state: State::Fresh,
        }
    }

    /// 本次连接的会话信息（尚未握手时为 `None`）。
    pub fn session_info(&self) -> Option<SessionInfo> {
        self.session.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 握手：交换 hello 与身份证明，派生会话密钥。
    ///
    /// 幂等——已握手就直接返回。发送方应在发送前显式调用一次，
    /// 好把 6 位校验码先打印出来给用户核对（否则要等文件发完才知道码）。
    pub fn handshake(&mut self) -> Result<()> {
        if !matches!(self.state, State::Fresh) {
            return Ok(());
        }

        // 1. 双方都先发自己的 hello，再读对方的（流程对称，不依赖谁先开口）
        let (ephemeral, own) = handshake::begin(&self.identity);
        self.inner.send_frame(&Frame::SecureHello(own.encode()))?;
        let peer_bytes = match self.inner.recv_frame()? {
            Frame::SecureHello(payload) => payload,
            other => {
                return Err(GlowError::Protocol(format!(
                    "期望加密握手（SecureHello），收到明文 {}；本版本不接受未加密连接",
                    frame_name(&other)
                )))
            }
        };
        let peer = SecureHello::decode(&peer_bytes)?;

        // 2. 派生会话密钥与校验码
        let session = handshake::finish(self.role, ephemeral, &own, &peer, &self.identity)?;

        // 3. 交换身份证明：能对上才说明对方真的持有那把长期私钥
        self.inner.send_frame(&Frame::SecureProof(session.proof))?;
        let received = match self.inner.recv_frame()? {
            Frame::SecureProof(proof) => proof,
            other => {
                return Err(GlowError::Protocol(format!(
                    "期望身份证明（SecureProof），收到 {}",
                    frame_name(&other)
                )))
            }
        };
        if !session.verify_peer_proof(&received) {
            return Err(GlowError::Crypto(
                "对端身份证明校验失败：对方可能并未持有该指纹对应的私钥".into(),
            ));
        }

        // 4. 记录会话信息（信任与否由调用方据此决定是否要人工确认）
        let peer_fingerprint = fingerprint_of(&peer.identity);
        let info = SessionInfo {
            trusted: self.trust.is_trusted(&peer_fingerprint),
            peer_fingerprint,
            peer_public: peer.identity,
            code: session.code.clone(),
            initiator: self.role.is_initiator(),
        };
        *self.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(info);

        self.state = State::Ready {
            send: FrameCipher::new(&session.send_key),
            recv: FrameCipher::new(&session.recv_key),
        };
        Ok(())
    }

    fn send_cipher(&mut self) -> Result<&mut FrameCipher> {
        match &mut self.state {
            State::Ready { send, .. } => Ok(send),
            State::Fresh => Err(GlowError::Crypto("尚未完成加密握手".into())),
        }
    }

    fn recv_cipher(&mut self) -> Result<&mut FrameCipher> {
        match &mut self.state {
            State::Ready { recv, .. } => Ok(recv),
            State::Fresh => Err(GlowError::Crypto("尚未完成加密握手".into())),
        }
    }
}

impl TransferTransport for SecureTransport {
    /// 转发给内层：加密层自己不知道有没有字节，能等的只有那条 TCP。
    fn wait_readable(&mut self, timeout: std::time::Duration) -> Result<bool> {
        self.inner.wait_readable(timeout)
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        self.handshake()?;
        let plaintext = frame.encode()?;
        let sealed = self.send_cipher()?.seal(&plaintext)?;
        self.inner.send_frame(&Frame::SecureData(sealed))
    }

    fn recv_frame(&mut self) -> Result<Frame> {
        self.handshake()?;
        let sealed = match self.inner.recv_frame()? {
            Frame::SecureData(payload) => payload,
            other => {
                return Err(GlowError::Crypto(format!(
                    "期望加密帧（SecureData），收到明文 {}；连接可能被降级",
                    frame_name(&other)
                )))
            }
        };
        let plaintext = self.recv_cipher()?.open(&sealed)?;
        Frame::decode(&plaintext)
    }
}

/// 帧类型名（错误信息里说清楚收到的是什么，别只说「格式错误」）。
pub(crate) fn frame_name(frame: &Frame) -> &'static str {
    match frame {
        Frame::Query(_) => "Query",
        Frame::Hello(_) => "Hello",
        Frame::Offer { .. } => "Offer",
        Frame::Accept => "Accept",
        Frame::Reject { .. } => "Reject",
        Frame::Chunk { .. } => "Chunk",
        Frame::Done { .. } => "Done",
        Frame::Probe(_) => "Probe",
        Frame::Pong(_) => "Pong",
        Frame::SecureHello(_) => "SecureHello",
        Frame::SecureProof(_) => "SecureProof",
        Frame::SecureData(_) => "SecureData",
        Frame::PairRequest { .. } => "PairRequest",
        Frame::PairOk => "PairOk",
        Frame::PairDeny { .. } => "PairDeny",
    }
}
