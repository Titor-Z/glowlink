//! `glowlink-core/src/model/crypto/` 的单元测试。
//!
//! 分两类：
//! - **RFC 官方向量**：证明我们**用对了**这些库（而非自己实现对不对）；
//! - **行为约束**：密钥一致、篡改必失败、nonce 不复用、校验码两端相同。

use glowlink_core::model::crypto::cipher::{FrameCipher, TAG_LEN};
use glowlink_core::model::crypto::handshake::{
    finish, keys_equal, respond, start, Role, SecureHello, HANDSHAKE_VERSION,
};
use glowlink_core::model::crypto::identity::{fingerprint_eq, fingerprint_of, Identity};
use glowlink_core::model::crypto::kdf::{hkdf_expand, hkdf_extract, hmac_sha256, sha256};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

// ---------------------------------------------------------------- RFC 向量

#[test]
fn sha256_matches_fips_vector() {
    // FIPS 180-4 附录：SHA-256("abc")
    assert_eq!(
        hex(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // 空串
    assert_eq!(
        hex(&sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn hmac_matches_rfc4231_case1() {
    // RFC 4231 Test Case 1：key = 0x0b × 20，data = "Hi There"
    let key = vec![0x0b; 20];
    assert_eq!(
        hex(&hmac_sha256(&key, b"Hi There")),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
}

#[test]
fn hkdf_matches_rfc5869_case1() {
    // RFC 5869 Test Case 1
    let ikm = vec![0x0b; 22];
    let salt = unhex("000102030405060708090a0b0c");
    let info = unhex("f0f1f2f3f4f5f6f7f8f9");
    let prk = hkdf_extract(&salt, &ikm);
    assert_eq!(
        hex(&prk),
        "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"
    );
    let okm = hkdf_expand(&prk, &info, 42);
    assert_eq!(
        hex(&okm),
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
    );
}

#[test]
fn nonce_layout_is_pinned() {
    // 协议约定：4 字节零 ‖ 8 字节大端计数。改这里等于改协议（需升版本号）。
    assert_eq!(FrameCipher::nonce_for(0), [0u8; 12]);
    assert_eq!(
        FrameCipher::nonce_for(1),
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
    );
    assert_eq!(
        FrameCipher::nonce_for(0x0102_0304_0506_0708),
        [0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]
    );
}

#[test]
fn ciphertext_produced_with_a_foreign_nonce_is_rejected() {
    // RFC 8439 §2.8.2 的 ChaCha20-Poly1305 真实密文（nonce 为 070000004041424344454647，
    // 与本实现的计数式布局不同）。同一把密钥下必须认证失败——
    // 这钉住的是「nonce 布局参与认证」，而不是原语本身：
    // 原语正确性由上游 crate 的自带 RFC 测试负责（这正是改用 crate 的理由）。
    let rfc_ciphertext = unhex(
        "d31a8d34648e60db7b86afbc53ef7ec2\
         a4aded51296e08fea9e2b5a736ee62d6\
         3dbea45e8ca9671282fafb69da92728b\
         1a71de0a9e060b2905d6a5b67ecd3b36\
         92ddbd7f2d778b8c9803aee328091b58\
         fab324e4fad675945585808b4831d7bc\
         3ff4def08e4b7a9de576d26586cec64b\
         6116",
    );
    let key = unhex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
    let mut key_array = [0u8; 32];
    key_array.copy_from_slice(&key);
    let mut cipher = FrameCipher::new(&key_array);
    assert!(
        cipher.open(&rfc_ciphertext).is_err(),
        "nonce 布局不符时必须认证失败，绝不能「解出垃圾明文」"
    );
    assert_eq!(cipher.used_nonces(), 0, "失败不应消耗 nonce");
}

#[test]
fn x25519_matches_rfc7748_vector() {
    // RFC 7748 §6.1：标量 a 与基点 u=9 的公钥
    let secret = unhex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
    let mut secret_array = [0u8; 32];
    secret_array.copy_from_slice(&secret);
    let identity = Identity::from_secret_bytes(secret_array);
    assert_eq!(
        hex(&identity.public_bytes()),
        "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
    );
}

// ---------------------------------------------------------------- 身份与指纹

#[test]
fn fingerprint_is_stable_and_short() {
    let identity = Identity::generate();
    let expected = fingerprint_of(&identity.public_bytes());
    assert_eq!(identity.fingerprint(), expected);
    assert_eq!(expected.len(), 16, "8 字节 → 16 个十六进制字符");
    // 两次生成不同身份 ⇒ 指纹不同
    assert_ne!(Identity::generate().fingerprint(), expected);
    assert!(fingerprint_eq(&expected, &identity.fingerprint()));
    assert!(!fingerprint_eq(&expected, "0000000000000000"));
}

#[test]
fn identity_roundtrips_through_secret_bytes() {
    let identity = Identity::generate();
    let restored = Identity::from_secret_bytes(*identity.secret_bytes());
    assert_eq!(restored.public_bytes(), identity.public_bytes());
    assert_eq!(restored.fingerprint(), identity.fingerprint());
}

// ---------------------------------------------------------------- 握手

/// 跑一次完整握手，返回（发起方会话，响应方会话，双方 Hello）。
fn handshake(
    initiator_id: &Identity,
    responder_id: &Identity,
) -> (
    glowlink_core::Session,
    glowlink_core::Session,
    SecureHello,
    SecureHello,
) {
    let (init_secret, init_hello) = start(initiator_id);
    let (resp_secret, resp_hello) = respond(responder_id, &init_hello);
    let init_session = finish(Role::Initiator, init_secret, &init_hello, &resp_hello, initiator_id)
        .expect("发起方握手应成功");
    let resp_session =
        finish(Role::Responder, resp_secret, &resp_hello, &init_hello, responder_id)
            .expect("响应方握手应成功");
    (init_session, resp_session, init_hello, resp_hello)
}

#[test]
fn both_sides_derive_crossed_identical_keys() {
    let (a, b) = (Identity::generate(), Identity::generate());
    let (init, resp, _, _) = handshake(&a, &b);

    assert!(keys_equal(&init.send_key, &resp.recv_key), "发起方的发送密钥 = 响应方的接收密钥");
    assert!(keys_equal(&init.recv_key, &resp.send_key), "反向同理");
    assert!(!keys_equal(&init.send_key, &init.recv_key), "两个方向必须是不同密钥");
    assert_eq!(init.transcript, resp.transcript, "抄本哈希必须一致");
}

#[test]
fn both_sides_show_the_same_code() {
    let (a, b) = (Identity::generate(), Identity::generate());
    let (init, resp, _, _) = handshake(&a, &b);
    assert_eq!(init.code, resp.code, "校验码必须两端相同，否则用户没法比对");
    assert_eq!(init.code.len(), 6, "六位数字");
    assert!(init.code.chars().all(|c| c.is_ascii_digit()));
}

#[test]
fn code_differs_across_sessions() {
    // 同样的两台设备再连一次，临时密钥变了 ⇒ 校验码也应不同
    let (a, b) = (Identity::generate(), Identity::generate());
    let (first, ..) = handshake(&a, &b);
    let (second, ..) = handshake(&a, &b);
    assert_ne!(first.code, second.code, "每次会话都应重新比对（除非已信任指纹）");
    assert!(!keys_equal(&first.send_key, &second.send_key), "会话密钥不能复用");
}

#[test]
fn peer_proof_confirms_long_term_identity() {
    let (a, b) = (Identity::generate(), Identity::generate());
    let (init, resp, _, _) = handshake(&a, &b);
    // 双方各自算出的证明相同 ⇒ 都持有自己的长期私钥
    assert!(init.verify_peer_proof(&resp.proof));
    assert!(resp.verify_peer_proof(&init.proof));

    // 伪造的证明必须被拒
    let mut forged = init.proof;
    forged[0] ^= 0xff;
    assert!(!init.verify_peer_proof(&forged));
}

#[test]
fn mitm_produces_mismatched_codes() {
    // 中间人与两端各握一次手：两端看到的校验码必然不同（用户比对即可发现）
    let (a, b) = (Identity::generate(), Identity::generate());
    let attacker = Identity::generate();

    let (a_side, _mitm_as_b, _, _) = handshake(&a, &attacker);
    let (_mitm_as_a, b_side, _, _) = handshake(&attacker, &b);
    assert_ne!(a_side.code, b_side.code, "中间人应当导致两端校验码不一致");
}

#[test]
fn hello_roundtrips_and_rejects_damage() {
    let identity = Identity::generate();
    let (_, hello) = start(&identity);
    let bytes = hello.encode();
    assert_eq!(bytes.len(), 81, "定长线格式");
    assert_eq!(SecureHello::decode(&bytes).unwrap(), hello);

    assert!(SecureHello::decode(&bytes[..80]).is_err(), "截断必须被拒");
    let mut bad_version = bytes.clone();
    bad_version[0] = HANDSHAKE_VERSION + 1;
    assert!(SecureHello::decode(&bad_version).is_err(), "版本不符必须被拒");
}

#[test]
fn small_order_public_key_is_refused() {
    // 全零公钥是小阶点 ⇒ 共享密钥可预测，必须拒绝而不是照常协商
    let identity = Identity::generate();
    let (secret, own) = start(&identity);
    let mut peer = own.clone();
    peer.ephemeral = [0u8; 32];
    assert!(finish(Role::Initiator, secret, &own, &peer, &identity).is_err());
}

// ---------------------------------------------------------------- 帧加密

#[test]
fn frame_seal_open_roundtrip_and_counter_advances() {
    let key = [7u8; 32];
    let mut sender = FrameCipher::new(&key);
    let mut receiver = FrameCipher::new(&key);

    for round in 0..4 {
        let plaintext = format!("第 {round} 帧内容").into_bytes();
        let sealed = sender.seal(&plaintext).unwrap();
        assert_eq!(sealed.len(), plaintext.len() + TAG_LEN, "只有标签开销");
        assert_eq!(receiver.open(&sealed).unwrap(), plaintext);
    }
    assert_eq!(sender.used_nonces(), 4);
    assert_eq!(receiver.used_nonces(), 4);

    // 同一密钥、同一明文，两次加密必须不同（证明 nonce 在推进，没有复用）
    let mut another = FrameCipher::new(&key);
    let first = another.seal(b"same").unwrap();
    let second = another.seal(b"same").unwrap();
    assert_ne!(first, second, "nonce 复用是 AEAD 的致命错误");
    assert_eq!(another.used_nonces(), 2);
}

#[test]
fn tampered_frame_is_rejected_without_burning_a_nonce() {
    let key = [3u8; 32];
    let mut sender = FrameCipher::new(&key);
    let mut receiver = FrameCipher::new(&key);

    let sealed = sender.seal("重要内容".as_bytes()).unwrap();
    let mut damaged = sealed.clone();
    damaged[0] ^= 0x01;
    assert!(receiver.open(&damaged).is_err(), "改动密文必须被认证拒绝");
    // 认证失败**不能**推进计数器：否则一轮攻击就能把 nonce 空间用掉一个
    assert_eq!(receiver.used_nonces(), 0, "失败不应消耗 nonce");
    // 状态没被搞乱：原样重发同一帧仍能解开
    assert_eq!(receiver.open(&sealed).unwrap(), "重要内容".as_bytes());
    assert_eq!(receiver.used_nonces(), 1);
}

#[test]
fn wrong_key_or_out_of_order_is_rejected() {
    let mut sender = FrameCipher::new(&[1u8; 32]);
    let mut stranger = FrameCipher::new(&[2u8; 32]);
    let sealed = sender.seal(b"hello").unwrap();
    assert!(stranger.open(&sealed).is_err(), "密钥不符必须失败");

    // 顺序错乱（跳过一帧）也必须失败：nonce 不对
    let mut receiver = FrameCipher::new(&[1u8; 32]);
    let _first = sender.seal("第一帧".as_bytes()).unwrap();
    let second = sender.seal("第二帧".as_bytes()).unwrap();
    assert!(receiver.open(&second).is_err(), "跳号解密必须失败");

    // 太短的数据直接判错，不喂给 AEAD
    let mut fresh = FrameCipher::new(&[1u8; 32]);
    assert!(fresh.open(&[0u8; 4]).is_err());
}

#[test]
fn session_keys_actually_protect_the_frames() {
    // 端到端串起来：握手 → 用会话密钥加密一帧 → 对端解开
    let (a, b) = (Identity::generate(), Identity::generate());
    let (init, resp, _, _) = handshake(&a, &b);

    let payload = r#"{"name":"报告.pdf"}"#.as_bytes();
    let mut out = FrameCipher::new(&init.send_key);
    let mut incoming = FrameCipher::new(&resp.recv_key);
    let sealed = out.seal(payload).unwrap();
    assert_eq!(incoming.open(&sealed).unwrap(), payload);

    // 第三方的密钥解不开
    let mut stranger = FrameCipher::new(&Identity::generate().public_bytes());
    assert!(stranger.open(&sealed).is_err());
}
