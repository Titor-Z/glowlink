//! 帧级 AEAD：ChaCha20-Poly1305 + 单向递增 nonce。
//!
//! 每个方向一把独立密钥、一个独立计数器，nonce = 4 字节零 ‖ 8 字节大端计数。
//! **计数器绝不能重复使用**（AEAD 的 nonce 复用会导致明文可恢复），
//! 所以：
//! - 计数只增不减，溢出即报错而不是回绕；
//! - 握手（`Hello`）是明文，不消耗计数器；此后每一帧都必须加解密一次。
//!
//! 关联数据（AAD）固定为协议标签：把帧与协议绑定，防止跨协议重放。

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

use crate::model::error::{GlowError, Result};

/// 关联数据：与协议名绑定，防止把别处的密文塞进来。
const AAD: &[u8] = b"glowlink v1 frame";

/// Poly1305 认证标签长度。
pub const TAG_LEN: usize = 16;

pub struct FrameCipher {
    cipher: ChaCha20Poly1305,
    /// 已使用的 nonce 计数（下一个可用值）。
    counter: u64,
}

impl FrameCipher {
    pub fn new(key: &[u8; 32]) -> Self {
        FrameCipher {
            cipher: ChaCha20Poly1305::new(Key::from_slice(key)),
            counter: 0,
        }
    }

    /// nonce：4 字节零 ‖ 8 字节大端计数。
    ///
    /// 这是**协议约定**（两端必须逐字节一致），所以公开出来，让测试能把它钉死；
    /// 改动它就等于改协议，必须同时升版本号。
    pub fn nonce_for(counter: u64) -> [u8; 12] {
        let mut bytes = [0u8; 12];
        bytes[4..].copy_from_slice(&counter.to_be_bytes());
        bytes
    }

    fn nonce(counter: u64) -> Nonce {
        *Nonce::from_slice(&Self::nonce_for(counter))
    }

    /// 加密一帧；返回密文（含 16 字节标签）。
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = Self::nonce(self.counter);
        let sealed = self
            .cipher
            .encrypt(&nonce, Payload { msg: plaintext, aad: AAD })
            .map_err(|_| GlowError::Crypto("帧加密失败".into()))?;
        // 只有加密成功才推进计数：失败时 nonce 没被用掉，不能跳号
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or_else(|| GlowError::Crypto("帧计数器溢出，必须重新握手".into()))?;
        Ok(sealed)
    }

    /// 解密一帧；密文被篡改或密钥不对都会失败（这是完整性保证）。
    pub fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        if ciphertext.len() < TAG_LEN {
            return Err(GlowError::Crypto(format!(
                "密文长度 {} 小于认证标签长度 {TAG_LEN}",
                ciphertext.len()
            )));
        }
        let nonce = Self::nonce(self.counter);
        let plaintext = self
            .cipher
            .decrypt(&nonce, Payload { msg: ciphertext, aad: AAD })
            .map_err(|_| GlowError::Crypto("帧完整性校验失败（密钥不符或数据被篡改）".into()))?;
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or_else(|| GlowError::Crypto("帧计数器溢出，必须重新握手".into()))?;
        Ok(plaintext)
    }

    /// 已使用的 nonce 数（自检与测试用）。
    pub fn used_nonces(&self) -> u64 {
        self.counter
    }

    /// 密文相对明文的开销。
    pub fn overhead() -> usize {
        TAG_LEN
    }
}
