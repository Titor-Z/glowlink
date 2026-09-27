//! 握手：临时密钥协商 + 身份证明 + 6 位校验码。
//!
//! 流程（三步，全部在 OFFER 之前）：
//! 1. 双方各发一个 [`SecureHello`]：临时 X25519 公钥 + **长期**身份公钥 + 随机数；
//! 2. 双方算出会话密钥与抄本哈希，并用**静态-静态 DH**派生的密钥做身份证明
//!    （能算出证明 == 对手确实持有那把长期私钥，冒名者算不出）；
//! 3. 双方各自显示同一个 6 位校验码，用户口头比对一次即可完成后配对。
//!
//! 为什么要有校验码：临时 DH 本身是匿名的，中间人可以把两条连接各自终结。
//! 但中间人无法让两端算出**相同的校验码**，用户一比对就能发现。
//! 配对完成后指纹进信任列表，此后凭身份证明自动通过，不再需要比对。

use rand_core::{OsRng, RngCore};
use x25519_dalek::{EphemeralSecret, PublicKey};

use crate::model::crypto::identity::Identity;
use crate::model::crypto::kdf::{hkdf_expand, hkdf_extract, hmac_sha256, sha256};
use crate::model::error::{GlowError, Result};

/// 握手协议版本：写入抄本，避免版本混淆攻击。
pub const HANDSHAKE_VERSION: u8 = 1;

/// 校验码位数。
pub const SAS_DIGITS: u32 = 6;

/// 抄本前缀（域分隔，避免与其它用途的哈希混用）。
const TRANSCRIPT_LABEL: &[u8] = b"glowlink v1 handshake";

/// 方向标签：发起方 → 响应方。
const KEY_INFO_INITIATOR: &[u8] = b"glowlink v1 initiator->responder";
/// 方向标签：响应方 → 发起方。
const KEY_INFO_RESPONDER: &[u8] = b"glowlink v1 responder->initiator";
/// 校验码派生标签。
const SAS_INFO: &[u8] = b"glowlink v1 sas";

/// 握手中的角色。发起方是 TCP 连接方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Initiator,
    Responder,
}

impl Role {
    pub fn is_initiator(self) -> bool {
        matches!(self, Role::Initiator)
    }
}

/// 明文握手消息（这一步还没有密钥，所以必须明文发出）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecureHello {
    pub version: u8,
    /// 临时 X25519 公钥（每次连接都换，提供前向保密）。
    pub ephemeral: [u8; 32],
    /// 长期身份公钥（用于识别设备）。
    pub identity: [u8; 32],
    /// 随机数（让同样的密钥每次握手也产生不同的抄本）。
    pub nonce: [u8; 16],
}

impl SecureHello {
    fn build(identity: &Identity, ephemeral: &PublicKey) -> Self {
        let mut nonce = [0u8; 16];
        OsRng.fill_bytes(&mut nonce);
        SecureHello {
            version: HANDSHAKE_VERSION,
            ephemeral: ephemeral.to_bytes(),
            identity: identity.public_bytes(),
            nonce,
        }
    }

    /// 线格式：版本(1) ‖ 临时公钥(32) ‖ 身份公钥(32) ‖ 随机数(16)，定长 81 字节。
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(81);
        out.push(self.version);
        out.extend_from_slice(&self.ephemeral);
        out.extend_from_slice(&self.identity);
        out.extend_from_slice(&self.nonce);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 81 {
            return Err(GlowError::Protocol(format!(
                "握手消息长度应为 81 字节，收到 {}",
                bytes.len()
            )));
        }
        if bytes[0] != HANDSHAKE_VERSION {
            return Err(GlowError::Protocol(format!(
                "不支持的握手版本 {}（本机 {HANDSHAKE_VERSION}）",
                bytes[0]
            )));
        }
        let mut ephemeral = [0u8; 32];
        let mut identity = [0u8; 32];
        let mut nonce = [0u8; 16];
        ephemeral.copy_from_slice(&bytes[1..33]);
        identity.copy_from_slice(&bytes[33..65]);
        nonce.copy_from_slice(&bytes[65..81]);
        Ok(SecureHello { version: bytes[0], ephemeral, identity, nonce })
    }
}

/// 握手完成后得到的东西。
pub struct Session {
    /// 本端发送用的密钥。
    pub send_key: [u8; 32],
    /// 本端接收用的密钥。
    pub recv_key: [u8; 32],
    /// 6 位校验码（两端相同，用户比对用）。
    pub code: String,
    /// 身份证明：对端需要发来同样的值，才能确认它持有那把长期私钥。
    pub proof: [u8; 32],
    /// 抄本哈希（调试与日志用，不含密钥）。
    pub transcript: [u8; 32],
}

impl Session {
    /// 用常量时间比较校验对端发来的身份证明。
    pub fn verify_peer_proof(&self, received: &[u8; 32]) -> bool {
        use subtle::ConstantTimeEq;
        bool::from(self.proof.ct_eq(received))
    }
}

/// 发起方：生成临时密钥并产出自己的 `SecureHello`。
pub fn start(identity: &Identity) -> (EphemeralSecret, SecureHello) {
    let secret = EphemeralSecret::random_from_rng(OsRng);
    let hello = SecureHello::build(identity, &PublicKey::from(&secret));
    (secret, hello)
}

/// 响应方：收到对方 `SecureHello` 后生成自己的 `SecureHello`。
pub fn respond(identity: &Identity, _peer: &SecureHello) -> (EphemeralSecret, SecureHello) {
    let secret = EphemeralSecret::random_from_rng(OsRng);
    let hello = SecureHello::build(identity, &PublicKey::from(&secret));
    (secret, hello)
}

/// 双方：算会话密钥、校验码与身份证明。
///
/// `own` / `peer` 必须按角色顺序传入（发起方的先），否则两端抄本不一致。
pub fn finish(
    role: Role,
    ephemeral: EphemeralSecret,
    own: &SecureHello,
    peer: &SecureHello,
    identity: &Identity,
) -> Result<Session> {
    let (initiator, responder) = match role {
        Role::Initiator => (own, peer),
        Role::Responder => (peer, own),
    };

    // 抄本：两端都按「发起方在前」的固定顺序拼，保证算出的哈希一致
    let mut transcript_input = Vec::with_capacity(TRANSCRIPT_LABEL.len() + 162);
    transcript_input.extend_from_slice(TRANSCRIPT_LABEL);
    transcript_input.extend_from_slice(&initiator.encode());
    transcript_input.extend_from_slice(&responder.encode());
    let transcript = sha256(&transcript_input);

    // 临时-临时 DH：会话密钥的来源，每次连接都换（前向保密）
    let peer_ephemeral = PublicKey::from(peer.ephemeral);
    let dh_ephemeral = ephemeral.diffie_hellman(&peer_ephemeral);
    if !dh_ephemeral.was_contributory() {
        // 对方临时公钥是小阶点 ⇒ 共享密钥退化为可预测值
        return Err(GlowError::Protocol(
            "对方临时公钥无效（小阶点），拒绝该连接".into(),
        ));
    }

    // 静态-静态 DH：身份证明的来源，能算出它才说明对手持有长期私钥
    let peer_static = PublicKey::from(peer.identity);
    let dh_static = identity.secret().diffie_hellman(&peer_static);
    if !dh_static.was_contributory() {
        return Err(GlowError::Protocol(
            "对方身份公钥无效（小阶点），拒绝该连接".into(),
        ));
    }

    // 密钥派生：盐用抄本哈希，输入材料是两条 DH 结果
    let mut ikm = Vec::with_capacity(64);
    ikm.extend_from_slice(dh_ephemeral.as_bytes());
    ikm.extend_from_slice(dh_static.as_bytes());
    let prk = hkdf_extract(&transcript, &ikm);

    let k_initiator = hkdf_expand(&prk, KEY_INFO_INITIATOR, 32);
    let k_responder = hkdf_expand(&prk, KEY_INFO_RESPONDER, 32);
    let (send_key, recv_key) = if role.is_initiator() {
        (k_initiator, k_responder)
    } else {
        (k_responder, k_initiator)
    };

    let sas_bytes = hkdf_expand(&prk, SAS_INFO, 4);
    let code = format!(
        "{:0width$}",
        u32::from_be_bytes([sas_bytes[0], sas_bytes[1], sas_bytes[2], sas_bytes[3]])
            % 10u32.pow(SAS_DIGITS),
        width = SAS_DIGITS as usize
    );

    Ok(Session {
        send_key: to_array(&send_key)?,
        recv_key: to_array(&recv_key)?,
        code,
        proof: hmac_sha256(dh_static.as_bytes(), &transcript),
        transcript,
    })
}

/// 常量时间比较两把会话密钥（仅测试与自检用）。
pub fn keys_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    use subtle::ConstantTimeEq;
    bool::from(left.ct_eq(right))
}

fn to_array(bytes: &[u8]) -> Result<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| GlowError::Protocol("派生的密钥长度不是 32 字节".into()))
}
