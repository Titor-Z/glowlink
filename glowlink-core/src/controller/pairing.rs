//! 配对会话：一次**不带文件**的握手 + 一次配对确认。
//!
//! 为什么单独一个控制器：配对是「建立信任」这个动作本身，不该只有想传文件时
//! 才顺带发生。协议上它与传输共用同一套加密握手（`SecureHello` / `SecureProof`），
//! 区别只在握手之后交换什么——配对交换的是 `PairRequest` / `PairOk` / `PairDeny`，
//! 而不是 `Offer`。
//!
//! **校验码怎么用**：`code` 由双方各自从握手抄本派生（见 `crypto::handshake`），
//! **不经过网络**。发起方把它显示给人看，接收方让人输入并**本地**比对。
//! 中间人把一条连接终结成两段时，两端派生的码必然不同，所以输入永远对不上——
//! 这正是「一侧显示、一侧输入」等价于（且强于）「双方各比一次」的原因：
//! 它多了一条攻击者注不进去的人类信道，而双方只动一次手。
//!
//! **不做降级**：明文配对帧由 [`SecureTransport`] 当场拒绝，不给中间人
//! 一条「请用明文配对」的通道。
//!
//! **指纹不靠声称**：结果里的指纹从握手里的静态公钥算出来（[`PairingOutcome`]），
//! 网络上传来的字符串一律不信——声称的指纹可以随便填。

use std::sync::Arc;

use crate::controller::transfer::TransferTransport;
use crate::model::crypto::identity::Identity;
use crate::model::error::{GlowError, Result};
use crate::model::packet::Frame;
use crate::patterns::secure::{frame_name, SecureTransport, SessionInfo, SessionSlot, TrustList};

/// 配对成功后的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingOutcome {
    /// 对端设备指纹——由握手里的静态公钥算出，**不是**对方声称的字符串。
    pub peer_fingerprint: String,
    /// 6 位校验码（两端相同）。
    pub code: String,
    /// 该指纹此前是否已在信任列表里（决定要不要请人核对）。
    pub trusted: bool,
}

/// 校验码的错误尝试上限。
///
/// 3 次之后就作废整个请求：6 位码的暴力空间只有 100 万，把尝试次数放开等于
/// 让人用脚本穷举。上限放在 core 里，因为「还要不要再让用户输一次」这条规则
/// 与界面无关，但绝不能各写一份。
pub const MAX_CODE_ATTEMPTS: u8 = 3;

/// 校验码尝试计数（纯计数，便于把「3 次作废」这条规则单测下来）。
#[derive(Clone, Debug)]
pub struct CodeAttempts {
    left: u8,
}

impl Default for CodeAttempts {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeAttempts {
    pub fn new() -> Self {
        CodeAttempts { left: MAX_CODE_ATTEMPTS }
    }

    /// 还剩几次机会。
    pub fn remaining(&self) -> u8 {
        self.left
    }

    /// 记一次输入结果，返回「还能不能再试」。
    ///
    /// 输对不扣次数——用户输对了却被顺手扣掉一次，是纯粹的恶意设计。
    pub fn record(&mut self, matched: bool) -> bool {
        if matched {
            return true;
        }
        self.left = self.left.saturating_sub(1);
        self.left > 0
    }
}

/// 一次配对会话。构造即完成握手，之后按角色调用。
///
/// 接收方的典型流程：`recv_request()` → （用户允许）`approve(用户输的码)`，
/// 不符可以再调一次（次数由 [`CodeAttempts`] 记），最后要么成功要么 `deny`。
pub struct PairingSession {
    transport: SecureTransport,
}

/// 同一个端口上的两种意图：握手后的第一帧决定这次连接是干什么的。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// 配对：对方说「我想和你配对」，带上它自报的名字。
    Pair { name: String },
    /// 传输：第一帧已经读出来了，交给 `transfer` 控制器继续。
    Transfer,
}

/// 握手并读第一帧，判定这次连接是配对还是传输。
///
/// 返回意图与**已读到的那一帧**：传输路径必须把它交回给 `transfer`，不能丢掉。
pub fn detect(transport: &mut SecureTransport) -> Result<(Intent, Frame)> {
    transport.handshake()?;
    let first = transport.recv_frame()?;
    let intent = match &first {
        Frame::PairRequest { name } => Intent::Pair { name: name.clone() },
        _ => Intent::Transfer,
    };
    Ok((intent, first))
}

/// 把已经读到的一帧「放回」给下游。
///
/// 存在的理由：配对与传输**共用同一个端口**，分流时不能不先把第一帧读出来看，
/// 而 `transfer` 控制器要求「第一帧由它自己读」——于是这里把它放回，
/// 对下游完全透明（适配器模式），`transfer.rs` 一行不改。
pub struct Preloaded {
    inner: SecureTransport,
    first: Option<Frame>,
}

impl Preloaded {
    pub fn new(inner: SecureTransport, first: Frame) -> Self {
        Preloaded { inner, first: Some(first) }
    }
}

impl TransferTransport for Preloaded {
    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        self.inner.send_frame(frame)
    }

    fn recv_frame(&mut self) -> Result<Frame> {
        if let Some(first) = self.first.take() {
            return Ok(first);
        }
        self.inner.recv_frame()
    }
}

impl PairingSession {
    /// 发起方（主动拖拽的那一侧，负责**显示**校验码）。
    pub fn initiate(
        transport: Box<dyn TransferTransport>,
        identity: Arc<Identity>,
        trust: Arc<dyn TrustList>,
        session: SessionSlot,
    ) -> Result<Self> {
        Self::new(SecureTransport::initiator(transport, identity, trust, session))
    }

    /// 接收方（被请求的那一侧，负责**输入**校验码）。
    pub fn accept(
        transport: Box<dyn TransferTransport>,
        identity: Arc<Identity>,
        trust: Arc<dyn TrustList>,
        session: SessionSlot,
    ) -> Result<Self> {
        Self::new(SecureTransport::responder(transport, identity, trust, session))
    }

    fn new(mut transport: SecureTransport) -> Result<Self> {
        // 构造即握手：调用方拿到 session 时就能显示校验码，
        // 不必等文件发完才知道码（这也是「先配对、再传文件」的前提）。
        // `handshake` 幂等：分流时已经握过手的传输再调一次不会重握手。
        transport.handshake()?;
        Ok(PairingSession { transport })
    }

    /// 用在**已经握过手**的传输上（分流认出这是配对请求之后）。
    /// 握手幂等，所以它不会把已建立的会话推倒重来。
    pub fn over(transport: SecureTransport) -> Result<Self> {
        Self::new(transport)
    }

    /// 本次会话的对端信息（指纹、校验码、是否已信任）。
    pub fn peer(&self) -> Result<SessionInfo> {
        self.transport
            .session_info()
            .ok_or_else(|| GlowError::Crypto("配对会话尚未完成握手".into()))
    }

    /// 本端在这次连接里是不是发起方（决定「显示码」还是「输入码」）。
    pub fn is_initiator(&self) -> Result<bool> {
        Ok(self.peer()?.initiator)
    }

    /// 6 位校验码。
    /// 对方还在这条连接上吗（用来发现「对方放弃了配对」）。
    ///
    /// 非消耗性：只看连接状态，不碰帧。
    pub fn peer_closed(&mut self) -> bool {
        self.transport.peer_closed().unwrap_or(true)
    }

    pub fn code(&self) -> Result<String> {
        Ok(self.peer()?.code)
    }

    /// 校验码是否相符。**常量时间比较**：码是认证材料，别把它变成计时旁路。
    ///
    /// 长度先比——码长是公开的（恒为 6 位），比长度不泄露内容。
    pub fn code_matches(&self, typed: &str) -> bool {
        use subtle::ConstantTimeEq;
        let code = match self.code() {
            Ok(code) => code,
            Err(_) => return false,
        };
        let typed = typed.trim().as_bytes();
        let expected = code.as_bytes();
        typed.len() == expected.len() && bool::from(typed.ct_eq(expected))
    }

    /// 发起方 → 接收方：我想和你配对。
    pub fn request(&mut self, name: &str) -> Result<()> {
        self.transport.send_frame(&Frame::PairRequest { name: name.to_string() })
    }

    /// 接收方：读配对请求，返回来访设备名（对方自报，仅供显示）。
    pub fn recv_request(&mut self) -> Result<String> {
        match self.transport.recv_frame()? {
            Frame::PairRequest { name } => Ok(name),
            other => Err(GlowError::Protocol(format!(
                "配对期望 PairRequest，收到 {}",
                frame_name(&other)
            ))),
        }
    }

    /// 接收方：校验用户输入的码。
    ///
    /// 相符就发 `PairOk` 并返回 `true`；**不符不发任何帧、返回 `false`**——
    /// 会话继续保持，用户可以再输一次。这不是「宽容」：如果一输错就断，
    /// 对方得重新拖一次、重新显示一个**新**的码（临时密钥每次都换），
    /// 人隔着屏幕核对一次的代价就变成两次。
    ///
    /// 次数用尽或用户拒绝时，由调用方显式 [`Self::deny`]——发不发拒绝帧
    /// 是调用方的决定，不是比对的副作用。
    pub fn approve(&mut self, typed: &str) -> Result<bool> {
        if self.code_matches(typed) {
            self.transport.send_frame(&Frame::PairOk)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// 接收方：拒绝这次配对（用户点了拒绝、或等待超时）。
    pub fn deny(&mut self, reason: &str) -> Result<()> {
        self.transport.send_frame(&Frame::PairDeny { reason: reason.to_string() })
    }

    /// 发起方：等对方的结论。被拒绝时返回 [`GlowError::Rejected`]。
    pub fn wait(&mut self) -> Result<PairingOutcome> {
        match self.transport.recv_frame()? {
            Frame::PairOk => {
                let peer = self.peer()?;
                // 对端**名字**不在这里：握手不携带显示名，发起方本来就知道自己
                // 连的是哪台设备（名字来自发现层），由调用方自己填。
                Ok(PairingOutcome {
                    peer_fingerprint: peer.peer_fingerprint,
                    code: peer.code,
                    trusted: peer.trusted,
                })
            }
            Frame::PairDeny { reason } => Err(GlowError::Rejected(reason)),
            other => Err(GlowError::Protocol(format!(
                "配对期望 PairOk/PairDeny，收到 {}",
                frame_name(&other)
            ))),
        }
    }
}
