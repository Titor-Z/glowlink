use std::io::Cursor;

use glowlink_core::model::packet::{Frame, Hello, VERSION};
use glowlink_core::model::packet::DeviceKind;

fn all_frames() -> Vec<Frame> {
    vec![
        Frame::Query(Hello {
            id: "abc123".into(),
            name: "客厅的 Mac".into(),
            port: 45678,
            reply_port: 9267,
            kind: DeviceKind::Laptop,
        }),
        Frame::Hello(Hello {
            id: "abc123".into(),
            name: "客厅的 Mac".into(),
            port: 45678,
            reply_port: 9267,
            kind: DeviceKind::Laptop,
        }),
        Frame::Offer { name: "设计稿.psd".into(), size: 1_048_576 },
        Frame::Accept,
        Frame::Reject { reason: "用户拒绝接收".into() },
        Frame::Chunk { offset: 65_536, data: vec![7u8; 1024], crc: 0xDEAD_BEEF },
        Frame::Done { size: 1_048_576, crc: 0x1234_5678 },
        // 配对三帧：一律只出现在 `SecureData` 内层（见 secure 的明文拒绝用例）
        Frame::PairRequest { name: "客厅的 Mac".into() },
        Frame::PairOk,
        Frame::PairDeny { reason: "校验码不符".into() },
    ]
}

#[test]
fn roundtrip_every_frame() {
    for frame in all_frames() {
        let bytes = frame.encode().expect("编码失败");
        assert_eq!(Frame::decode(&bytes).expect("解码失败"), frame);
    }
}

#[test]
fn every_device_kind_survives_the_round_trip() {
    for kind in [
        DeviceKind::Unknown,
        DeviceKind::Phone,
        DeviceKind::Tablet,
        DeviceKind::Desktop,
        DeviceKind::Laptop,
    ] {
        let frame = Frame::Hello(Hello {
            id: "abc123".into(),
            name: "甲机".into(),
            port: 4000,
            reply_port: 9267,
            kind,
        });
        let bytes = frame.encode().unwrap();
        let Frame::Hello(decoded) = Frame::decode(&bytes).unwrap() else {
            panic!("应解出 Hello")
        };
        assert_eq!(decoded.kind, kind, "类型必须原样传回");
    }
}

#[test]
fn legacy_hello_without_kind_reads_as_unknown() {
    // 老版本节点的报文到 reply_port 就结束了。把尾字节去掉即还原老格式：
    // 必须解成「未知类型」，而不是「报文损坏」——否则一次升级就把旧节点全挤出网络。
    let frame = Frame::Hello(Hello {
        id: "abc123".into(),
        name: "老节点".into(),
        port: 4000,
        reply_port: 9267,
        kind: DeviceKind::Laptop,
    });
    let bytes = frame.encode().unwrap();
    let legacy = &bytes[..bytes.len() - 1];
    let Frame::Hello(decoded) = Frame::decode(legacy).expect("老报文必须仍能解出") else {
        panic!("应解出 Hello")
    };
    assert_eq!(decoded.kind, DeviceKind::Unknown);
    assert_eq!(decoded.name, "老节点");
}

#[test]
fn unknown_kind_value_reads_as_unknown() {
    // 将来的版本可能报出我们不认识的值：归为「未知」，不报错
    let frame = Frame::Hello(Hello {
        id: "abc123".into(),
        name: "新节点".into(),
        port: 4000,
        reply_port: 9267,
        kind: DeviceKind::Unknown,
    });
    let mut bytes = frame.encode().unwrap();
    let last = bytes.len() - 1;
    bytes[last] = 9;
    let Frame::Hello(decoded) = Frame::decode(&bytes).expect("不认识的类型不应报错") else {
        panic!("应解出 Hello")
    };
    assert_eq!(decoded.kind, DeviceKind::Unknown);
}

#[test]
fn roundtrip_over_byte_stream() {
    let frames = all_frames();
    let mut stream = Vec::new();
    for frame in &frames {
        frame.write_to(&mut stream).expect("写入失败");
    }
    let mut cursor = Cursor::new(stream);
    for expected in &frames {
        assert_eq!(&Frame::read_from(&mut cursor).expect("读取失败"), expected);
    }
    assert!(Frame::read_from(&mut cursor).is_err(), "流已耗尽仍能读到帧");
}

#[test]
fn rejects_bad_magic() {
    let mut bytes = Frame::Accept.encode().unwrap();
    bytes[0] = b'X';
    assert!(Frame::decode(&bytes).is_err());
}

#[test]
fn rejects_unsupported_version() {
    let mut bytes = Frame::Accept.encode().unwrap();
    bytes[4] = VERSION + 1;
    assert!(Frame::decode(&bytes).is_err());
}

#[test]
fn rejects_unknown_kind() {
    let mut bytes = Frame::Accept.encode().unwrap();
    bytes[5] = 99;
    assert!(Frame::decode(&bytes).is_err());
}

#[test]
fn rejects_truncated_payload() {
    let bytes = Frame::Chunk { offset: 0, data: vec![1, 2, 3], crc: 0 }.encode().unwrap();
    for cut in 5..bytes.len() {
        assert!(Frame::decode(&bytes[..cut]).is_err(), "截断到 {cut} 字节却解码成功");
    }
}

#[test]
fn rejects_oversized_frame_length_prefix() {
    let mut stream = Vec::new();
    stream.extend_from_slice(&u32::MAX.to_be_bytes());
    stream.extend_from_slice(&[0u8; 16]);
    assert!(Frame::read_from(&mut Cursor::new(stream)).is_err());
}
