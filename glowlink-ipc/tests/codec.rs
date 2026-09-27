//! `glowlink-ipc/src/codec.rs` 的单元测试：错误种类、`OK` 载荷、`RESOLVE` 应答。

use std::io::ErrorKind;

use glowlink_core::model::error::GlowError;
use glowlink_ipc::codec::{error_from_line, error_kind, error_reply, is_network_io, ok_payload, parse_resolve};

#[test]
fn ok_payload_strips_the_prefix_and_trims() {
    assert_eq!(ok_payload("OK 已接受 #007"), "已接受 #007");
    assert_eq!(ok_payload("没有前缀"), "没有前缀");
    assert_eq!(ok_payload("OK   "), "");
}

#[test]
fn resolve_reply_is_parsed_into_three_fields() {
    let (name, id, addr) = parse_resolve("OK 甲机\tabc123def4567890\t192.168.31.37:53500").unwrap();
    assert_eq!(name, "甲机");
    assert_eq!(id, "abc123def4567890");
    assert_eq!(addr.to_string(), "192.168.31.37:53500");
}

#[test]
fn resolve_reply_with_broken_address_is_a_protocol_error() {
    let err = parse_resolve("OK 甲机\tabc\t不是地址").unwrap_err();
    assert!(matches!(err, GlowError::Protocol(_)), "实际：{err:?}");
    // 字段不够同样是协议错误，不能给出半个结果
    assert!(parse_resolve("OK 甲机").is_err());
}

#[test]
fn error_kind_names_are_stable() {
    // 种类名是协议的一部分：改名等于改协议，这里逐个钉住
    let cases = [
        (GlowError::Usage("x".into()), "usage"),
        (GlowError::Config("x".into()), "config"),
        (GlowError::NotFound("x".into()), "notfound"),
        (GlowError::Missing("x".into()), "missing"),
        (GlowError::Crypto("x".into()), "crypto"),
        (GlowError::Rejected("x".into()), "rejected"),
        (GlowError::Protocol("x".into()), "protocol"),
        (GlowError::Remote("x".into()), "remote"),
        (
            GlowError::Io(std::io::Error::new(ErrorKind::ConnectionRefused, "x")),
            "net",
        ),
        (
            GlowError::Io(std::io::Error::new(ErrorKind::NotFound, "x")),
            "io",
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error_kind(&error), expected, "实际：{error:?}");
    }
}

#[test]
fn network_io_has_a_single_source_of_truth() {
    // 这套判定同时决定「错误种类名」与 CLI 的退出码；
    // 分两处写就会出现「种类叫 net、退出码算通用」的分裂。
    for kind in [
        ErrorKind::ConnectionRefused,
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionAborted,
        ErrorKind::NotConnected,
        ErrorKind::TimedOut,
        ErrorKind::BrokenPipe,
        ErrorKind::NetworkUnreachable,
        ErrorKind::HostUnreachable,
        ErrorKind::AddrNotAvailable,
    ] {
        assert!(is_network_io(kind), "{kind:?} 应算网络类");
    }
    for kind in [ErrorKind::NotFound, ErrorKind::PermissionDenied, ErrorKind::Other] {
        assert!(!is_network_io(kind), "{kind:?} 不该算网络类");
    }
}

#[test]
fn error_reply_and_from_line_are_inverse() {
    let cases = [
        GlowError::Usage("参数写错了".into()),
        GlowError::NotFound("找不到设备「甲机」".into()),
        GlowError::Crypto("握手失败".into()),
        GlowError::Rejected("对方拒绝".into()),
        GlowError::Protocol("响应坏了".into()),
        GlowError::Remote("远端原文".into()),
        GlowError::Io(std::io::Error::new(ErrorKind::TimedOut, "超时")),
    ];
    for error in cases {
        let line = error_reply(&error);
        let restored = error_from_line(&line);
        assert_eq!(
            error_kind(&restored),
            error_kind(&error),
            "种类必须原样传回才能保住退出码：{line}"
        );
    }
}

#[test]
fn unknown_kind_falls_back_to_remote() {
    // 老守护进程回的行没有种类：不能当成「本机错误」，也不能丢掉原文
    let restored = error_from_line("ERR 未找到设备「甲机」");
    assert!(matches!(restored, GlowError::Remote(_)), "实际：{restored:?}");
    assert!(restored.to_string().contains("甲机"));
}
