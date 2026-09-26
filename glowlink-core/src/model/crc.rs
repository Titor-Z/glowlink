//! 自定义二进制协议使用的 CRC32 校验（多项式 0xEDB88320，与 zlib 一致）。

/// 增量 CRC32，便于流式校验大文件而不必整体驻留内存。
#[derive(Debug, Clone)]
pub struct Crc32 {
    state: u32,
}

impl Crc32 {
    pub fn new() -> Self {
        Crc32 { state: 0xFFFF_FFFF }
    }

    pub fn update(&mut self, data: &[u8]) -> &mut Self {
        let mut crc = self.state;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        self.state = crc;
        self
    }

    pub fn finish(&self) -> u32 {
        self.state ^ 0xFFFF_FFFF
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(data);
    crc.finish()
}
