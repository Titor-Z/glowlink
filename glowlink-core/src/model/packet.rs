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
/// 协议版本号（线上二进制用的单字节）。
pub const VERSION: u8 = 1;
/// 软件版本字符串，用于端口自检时判断"运行中的是不是旧版本"。
pub const VERSION_STR: &str = env!("CARGO_PKG_VERSION");

/// 局域网发现使用的 UDP 端口（局域网内所有节点必须一致）。
pub const DISCOVERY_PORT: u16 = 9267;
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
/// 探测本机是否已有节点监听（端口自检用，不属于正常发现流程）。
const KIND_PROBE: u8 = 8;
/// 对探测的应答，携带版本与 PID，用于识别“是我们自己的旧进程”。
const KIND_PONG: u8 = 9;

/// 加密握手：携带 [`SecureHello`] 的定长载荷（临时公钥 + 身份公钥 + 随机数）。
const KIND_SECURE_HELLO: u8 = 10;
/// 加密握手：身份证明（能算出它 == 对方持有长期私钥）。
const KIND_SECURE_PROOF: u8 = 11;
/// 加密数据：载荷是内层帧的 AEAD 密文，接收方解密后再解析内层帧。
const KIND_SECURE_DATA: u8 = 12;

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
    /// 端口自检探测：问“这个端口上的监听者是不是我们自己人”。
    Probe(Probe),
    /// 探测应答：告诉对方我们的版本与进程号。
    Pong(Probe),
    /// 加密握手消息（对 `packet` 层是**不透明字节**，语义由 `crypto::handshake` 解释）。
    SecureHello(Vec<u8>),
    /// 加密握手的身份证明。
    SecureProof([u8; 32]),
    /// 加密数据帧：内层帧的密文。
    SecureData(Vec<u8>),
}

/// 端口自检握手载荷。
///
/// 复用的是与 `Hello` 同一套“身份 + 端口”字段，外加版本与 PID：
/// 收到 `Pong` 后能直接判断是“自己的旧进程”还是“别的程序”。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// 发送方身份 id，便于日志区分。
    pub id: String,
    /// 软件版本字符串（如 `0.1.5`）。
    pub version: String,
    /// 进程号；用于判断“已在运行”时是否需要顶替。
    pub pid: u32,
    /// 发送方接收发现报文的 UDP 端口；0 表示未监听。
    pub reply_port: u16,
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
            Frame::Probe(p) => {
                w.u8(KIND_PROBE);
                w.probe(p)?;
            }
            Frame::Pong(p) => {
                w.u8(KIND_PONG);
                w.probe(p)?;
            }
            Frame::SecureHello(payload) => {
                w.u8(KIND_SECURE_HELLO);
                w.u32(payload.len() as u32);
                w.raw(payload);
            }
            Frame::SecureProof(proof) => {
                w.u8(KIND_SECURE_PROOF);
                w.raw(proof);
            }
            Frame::SecureData(payload) => {
                w.u8(KIND_SECURE_DATA);
                w.u32(payload.len() as u32);
                w.raw(payload);
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
            KIND_PROBE => Frame::Probe(r.probe()?),
            KIND_PONG => Frame::Pong(r.probe()?),
            KIND_SECURE_HELLO => {
                let len = r.u32()? as usize;
                if len > MAX_FRAME {
                    return Err(GlowError::Protocol(format!("加密握手载荷过大：{len}")));
                }
                Frame::SecureHello(r.raw(len)?.to_vec())
            }
            KIND_SECURE_PROOF => {
                let mut proof = [0u8; 32];
                proof.copy_from_slice(r.raw(32)?);
                Frame::SecureProof(proof)
            }
            KIND_SECURE_DATA => {
                let len = r.u32()? as usize;
                if len > MAX_FRAME {
                    return Err(GlowError::Protocol(format!("加密帧过大：{len}")));
                }
                Frame::SecureData(r.raw(len)?.to_vec())
            }
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

    fn probe(&mut self, p: &Probe) -> Result<()> {
        self.str(&p.id)?;
        self.str(&p.version)?;
        self.u32(p.pid);
        self.u16(p.reply_port);
        Ok(())
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

    fn probe(&mut self) -> Result<Probe> {
        Ok(Probe {
            id: self.str()?,
            version: self.str()?,
            pid: self.u32()?,
            reply_port: self.u16()?,
        })
    }
}
