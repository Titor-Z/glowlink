//! 摘要与密钥派生：SHA-256 / HMAC-SHA256 / HKDF-SHA256（薄封装，不含自研算法）。

use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// 一次 SHA-256。
pub fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// HMAC-SHA256。
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key)
        .expect("HMAC 接受任意长度密钥");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// HKDF-SHA256 的 expand 阶段（extract 由 [`extract`] 单独暴露）。
pub fn hkdf_expand(prk: &[u8; 32], info: &[u8], length: usize) -> Vec<u8> {
    let hkdf = Hkdf::<Sha256>::from_prk(prk).expect("32 字节 PRK 长度合法");
    let mut out = vec![0u8; length];
    hkdf.expand(info, &mut out).expect("输出长度在 HKDF 上限内");
    out
}

/// HKDF-SHA256 的 extract 阶段。
pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; 32] {
    let (prk, _) = Hkdf::<Sha256>::extract(Some(salt), ikm);
    prk.into()
}
