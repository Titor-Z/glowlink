//! 私有二进制协议：帧编解码。
//!
//! 线上格式：
//! ```text
//! [4B 魔数 "GLWK"] [1B 版本] [1B 类型] [负载…]
//! TCP 传输时在最前面额外加 4B 大端长度前缀，便于从字节流中切帧。
//! ```

use std::io::{Read, Write};

use crate::model::error::{GlowError, Result};

pub const MAGIC: [u8; 4] = *b"GLWK";
pub const VERSION: u8 = 1;

/// 局域网发现使用的 UDP 端口（局域网内所有节点必须一致）。
pub const DISCOVERY_PORT: u16 = 19999;
/// 默认分块大小。
pub const CHUNK_SIZE: usize = 64 * 1024;
/// 单帧上限，防御恶意超长帧导致的内存爆炸。
pub const MAX_FRAME: usize = 1024 * 1024;

const KIND_HELLO: u8 = 1;
const KIND_OFFER: u8 = 2;
const KIND_ACCEPT: u8 = 3;
const KIND_REJECT: u8 = 4;
const KIND_CHUNK: u8 = 5;
const KIND_DONE: u8 = 6;
const KIND_QUERY: u8 = 7;

/// 设备广播报文。
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_field_names)]
pub struct Hello {
    pub id: String,
    pub name: String,
    /// 该节点的 TCP 传输端口。
    pub port: u16,
    /// 该节点接收发现报文的 UDP 端口；为 0 表示它只发查询（降级模式）。
    pub reply_port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// 广播查询：向广播地址发出，携带自身身份与传输端口。
    Query(Hello),
    /// 单播应答：对收到的查询回给其来源地址。
    Hello(Hello),
    /// 发送方声明即将发送的文件。
    Offer { name: String, size: u64 },
    Accept,
    Reject { reason: String },
    Chunk { offset: u64, data: Vec<u8>, crc: u32 },
    /// 传输结束，携带整文件大小与 CRC32。
    Done { size: u64, crc: u32 },
}

impl Frame {
    /// 编码为 UDP 数据报可用的裸字节。
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        w.raw(&MAGIC);
        w.u8(VERSION);
        match self {
            Frame::Query(h) => {
                w.u8(KIND_QUERY);
                w.str(&h.id)?;
                w.str(&h.name)?;
                w.u16(h.port);
                w.u16(h.reply_port);
            }
            Frame::Hello(h) => {
                w.u8(KIND_HELLO);
                w.str(&h.id)?;
                w.str(&h.name)?;
                w.u16(h.port);
                w.u16(h.reply_port);
            }
            Frame::Offer { name, size } => {
                w.u8(KIND_OFFER);
                w.str(name)?;
                w.u64(*size);
            }
            Frame::Accept => w.u8(KIND_ACCEPT),
            Frame::Reject { reason } => {
                w.u8(KIND_REJECT);
                w.str(reason)?;
            }
            Frame::Chunk { offset, data, crc } => {
                w.u8(KIND_CHUNK);
                w.u64(*offset);
                w.u32(*crc);
                w.u32(data.len() as u32);
                w.raw(data);
            }
            Frame::Done { size, crc } => {
                w.u8(KIND_DONE);
                w.u64(*size);
                w.u32(*crc);
            }
        }
        Ok(w.buf)
    }

    pub fn decode(buf: &[u8]) -> Result<Frame> {
        let mut r = Reader::new(buf);
        if r.raw(4)? != MAGIC.as_slice() {
            return Err(GlowError::Protocol("魔数不匹配".into()));
        }
        let version = r.u8()?;
        if version != VERSION {
            return Err(GlowError::Protocol(format!("协议版本不支持：{version}")));
        }
        let kind = r.u8()?;
        let frame = match kind {
            KIND_QUERY | KIND_HELLO => {
                let hello = Hello {
                    id: r.str()?,
                    name: r.str()?,
                    port: r.u16()?,
                    reply_port: r.u16()?,
                };
                if kind == KIND_QUERY {
                    Frame::Query(hello)
                } else {
                    Frame::Hello(hello)
                }
            }
            KIND_OFFER => Frame::Offer {
                name: r.str()?,
                size: r.u64()?,
            },
            KIND_ACCEPT => Frame::Accept,
            KIND_REJECT => Frame::Reject { reason: r.str()? },
            KIND_CHUNK => {
                let offset = r.u64()?;
                let crc = r.u32()?;
                let len = r.u32()? as usize;
                if len > MAX_FRAME {
                    return Err(GlowError::Protocol(format!("分块过大：{len}")));
                }
                Frame::Chunk {
                    offset,
                    data: r.raw(len)?.to_vec(),
                    crc,
                }
            }
            KIND_DONE => Frame::Done {
                size: r.u64()?,
                crc: r.u32()?,
            },
            other => return Err(GlowError::Protocol(format!("未知帧类型：{other}"))),
        };
        Ok(frame)
    }

    /// 带长度前缀写入字节流。
    pub fn write_to<W: Write>(&self, w: &mut W) -> Result<()> {
        let bytes = self.encode()?;
        w.write_all(&(bytes.len() as u32).to_be_bytes())?;
        w.write_all(&bytes)?;
        w.flush()?;
        Ok(())
    }

    /// 从字节流中读出一个完整帧。
    pub fn read_from<R: Read>(r: &mut R) -> Result<Frame> {
        let mut len = [0u8; 4];
        r.read_exact(&mut len)?;
        let len = u32::from_be_bytes(len) as usize;
        if len == 0 || len > MAX_FRAME {
            return Err(GlowError::Protocol(format!("帧长度非法：{len}")));
        }
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf)?;
        Frame::decode(&buf)
    }
}

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Writer { buf: Vec::with_capacity(256) }
    }

    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    fn str(&mut self, s: &str) -> Result<()> {
        let bytes = s.as_bytes();
        if bytes.len() > u16::MAX as usize {
            return Err(GlowError::Protocol("字符串字段过长".into()));
        }
        self.u16(bytes.len() as u16);
        self.raw(bytes);
        Ok(())
    }

    fn raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn raw(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(|| GlowError::Protocol("帧长度溢出".into()))?;
        if end > self.buf.len() {
            return Err(GlowError::Protocol("帧数据不完整".into()));
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.raw(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.raw(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.raw(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64> {
        let b = self.raw(8)?;
        Ok(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    fn str(&mut self) -> Result<String> {
        let len = self.u16()? as usize;
        let bytes = self.raw(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| GlowError::Protocol("字符串字段不是合法 UTF-8".into()))
    }
}
