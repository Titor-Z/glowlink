use glowlink_core::model::crc::{crc32, Crc32};

#[test]
fn matches_known_vectors() {
    // zlib/PNG 使用的标准向量。
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
    assert_eq!(crc32(b"a"), 0xE8B7_BE43);
}

#[test]
fn incremental_equals_oneshot() {
    let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
    let mut crc = Crc32::new();
    for chunk in data.chunks(997) {
        crc.update(chunk);
    }
    assert_eq!(crc.finish(), crc32(&data));
}

#[test]
fn detects_single_bit_flip() {
    let original = b"glowlink payload".to_vec();
    let mut corrupted = original.clone();
    corrupted[3] ^= 0x01;
    assert_ne!(crc32(&original), crc32(&corrupted));
}
