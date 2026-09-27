//! 长期身份密钥（X25519 静态密钥）与指纹。
//!
//! 身份密钥的作用不是加密，而是**证明「这台设备就是那台设备」**：
//! 指纹（公钥的 SHA-256 前缀）进信任列表，之后同一台设备再来就能免确认自动接收。
//! 私钥只在本模块内以 `StaticSecret` 形式存在（dalek 负责用后清零），
//! 对外只暴露 `secret_bytes()` 用于持久化，返回值用 `Zeroizing` 包住。

use rand_core::OsRng;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

use crate::model::crypto::kdf::sha256;

/// 指纹长度（字节）：8 字节足够避免局域网内碰撞，又短到能口头核对。
pub const FINGERPRINT_LEN: usize = 8;

/// 设备的长期身份密钥对。
pub struct Identity {
    secret: StaticSecret,
    public: PublicKey,
}

impl Identity {
    /// 现场生成一份新身份（首次运行时调用）。
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        Identity { secret, public }
    }

    /// 从持久化的私钥字节恢复身份。
    pub fn from_secret_bytes(bytes: [u8; 32]) -> Self {
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret);
        Identity { secret, public }
    }

    /// 私钥字节（用于持久化；调用方用完即弃，`Zeroizing` 会清零）。
    pub fn secret_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.secret.to_bytes())
    }

    pub fn public_bytes(&self) -> [u8; 32] {
        self.public.to_bytes()
    }

    pub fn public_key(&self) -> PublicKey {
        self.public
    }

    /// 本机静态密钥（握手证明身份时使用）。
    pub(crate) fn secret(&self) -> &StaticSecret {
        &self.secret
    }

    /// 设备指纹：公钥的 SHA-256 前缀，十六进制小写。
    pub fn fingerprint(&self) -> String {
        fingerprint_of(&self.public_bytes())
    }
}

/// 由公钥字节算指纹（信任列表里存的也是这个）。
pub fn fingerprint_of(public_key: &[u8; 32]) -> String {
    let digest = sha256(public_key);
    digest[..FINGERPRINT_LEN]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 常量时间比较两个指纹，避免用比较耗时反推内容（指纹不算秘密，但习惯一致）。
pub fn fingerprint_eq(left: &str, right: &str) -> bool {
    use subtle::ConstantTimeEq;
    let (a, b) = (left.as_bytes(), right.as_bytes());
    a.len() == b.len() && bool::from(a.ct_eq(b))
}
