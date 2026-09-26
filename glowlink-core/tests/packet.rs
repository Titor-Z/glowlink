use std::io::Cursor;

use glowlink_core::model::packet::{Frame, Hello, VERSION};

fn all_frames() -> Vec<Frame> {
    vec![
        Frame::Hello(Hello { id: "abc123".into(), name: "客厅的 Mac".into(), port: 45678 }),
        Frame::Offer { name: "设计稿.psd".into(), size: 1_048_576 },
        Frame::Accept,
        Frame::Reject { reason: "用户拒绝接收".into() },
        Frame::Chunk { offset: 65_536, data: vec![7u8; 1024], crc: 0xDEAD_BEEF },
        Frame::Done { size: 1_048_576, crc: 0x1234_5678 },
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
