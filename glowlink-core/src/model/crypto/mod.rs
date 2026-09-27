//! 密码学原语的唯一入口（`model` 层：纯计算，不做任何 IO）。
//!
//! 依赖政策：**本模块是全项目唯一允许引入第三方依赖的地方**（见 AGENTS.md「编码规范」）。
//! 一律使用成熟实现，禁止自己写 SHA-256 / HKDF / HMAC / X25519 / ChaCha20-Poly1305：
//! 手写实现即使通过 RFC 测试向量，也只证明「对向量正确」，证明不了无侧信道、无边界错误。
//!
//! 分层：
//! - [`kdf`]：摘要与密钥派生（SHA-256 / HMAC-SHA256 / HKDF）
//! - [`identity`]：长期身份密钥（X25519）与指纹
//! - [`handshake`]：握手抄本、会话密钥、6 位校验码、身份证明
//! - [`cipher`]：帧级 AEAD（ChaCha20-Poly1305 + 单向递增 nonce）
//!
//! 密钥文件落盘（路径、0600 权限）属于 IO，放在 CLI 侧（`identity_file.rs`）。

pub mod cipher;
pub mod handshake;
pub mod identity;
pub mod kdf;

pub use cipher::FrameCipher;
pub use handshake::{SecureHello, Role, Session, SAS_DIGITS};
pub use identity::{fingerprint_of, Identity, FINGERPRINT_LEN};
pub use kdf::{hkdf_expand, hmac_sha256, sha256};
