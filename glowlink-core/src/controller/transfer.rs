//! 传输控制器：发送 / 接收文件，含分块校验与整文件校验。
//!
//! 发送侧用环形缓冲池 + 独立读盘线程实现背压；接收侧逐帧校验偏移与 CRC32。

use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use crate::model::crc::{crc32, Crc32};
use crate::model::error::{GlowError, Result};
use crate::model::packet::Frame;
use crate::model::ring::SharedRing;

/// 「等对方答复」的轮询步长：够短才能让取消立刻生效，够长则不打转。
/// 150ms 对用户是无感的（比一次点击的反馈还快）。
pub const ANSWER_POLL: std::time::Duration = std::time::Duration::from_millis(150);

/// 传输通道（接口）。
pub trait TransferTransport: Send {
    fn send_frame(&mut self, frame: &Frame) -> Result<()>;
    fn recv_frame(&mut self) -> Result<Frame>;

    /// 等对方发来东西，最多等 `timeout`；返回「到底有没有可读的」。
    ///
    /// **必须是非消耗性的**（TCP 上用 `peek`）：实现不能把半个帧读走，
    /// 否则加密层的流位置就错位了（那种 bug 只在超时发生时出现，极难查）。
    ///
    /// 默认返回 `true`（内存通道 / 测试替身总是「马上就有」）：
    /// 这样调用方不用为测试特判，而真实 TCP 实现会真的等。
    fn wait_readable(&mut self, _timeout: std::time::Duration) -> Result<bool> {
        Ok(true)
    }
}

/// 监听端（接口），接收方使用。
pub trait TransferListener: Send + Sync {
    fn local_port(&self) -> Result<u16>;
    fn accept(&self) -> Result<(Box<dyn TransferTransport>, SocketAddr)>;
}

/// 视图层（接口）：确认接收与进度汇报，由 UI 层实现。
pub trait TransferView: Send + Sync {
    /// 收到 OFFER 后询问用户是否接收。
    fn confirm_receive(&self, peer: &str, name: &str, size: u64) -> bool;
    fn progress(&self, label: &str, done: u64, total: u64);
    /// 用户在半途取消了这次传输吗？**每个分块问一次**。
    ///
    /// 默认 `false`：不需要取消的调用方（CLI 的一次性进度视图、测试替身）什么都不用写。
    /// 有取消需求的是守护进程（桌面版的 ✕ 按钮经 IPC 打到这儿）。
    fn canceled(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TransferController {
    chunk_size: usize,
    ring_capacity: usize,
}

impl TransferController {
    pub fn new(chunk_size: usize, ring_capacity: usize) -> Self {
        TransferController {
            chunk_size: chunk_size.max(1),
            // 保证至少能容纳两个分块，避免背压下退化成碎片流。
            ring_capacity: ring_capacity.max(chunk_size.max(1) * 2),
        }
    }

    /// 发送单个文件，返回实际发送字节数。
    pub fn send_file(
        &self,
        transport: &mut dyn TransferTransport,
        path: &Path,
        view: &dyn TransferView,
    ) -> Result<u64> {
        let meta = fs::metadata(path)?;
        if !meta.is_file() {
            return Err(GlowError::Protocol(format!("不是普通文件：{}", path.display())));
        }
        let size = meta.len();
        let name = file_name(path)?;

        transport.send_frame(&Frame::Offer { name, size })?;
        // 等对方拍板。这一段**必须可取消**：对方可能正在等人点「允许」（最长几分钟），
        // 而死等在 `recv_frame` 上时，用户点的 ✕ 要等到对方答复才有机会生效——
        // 表现就是「取消按钮像坏的」。
        let reply = loop {
            if view.canceled() {
                let _ = transport.send_frame(&Frame::Reject { reason: "对方取消了这次发送".into() });
                return Err(GlowError::Canceled("已取消发送".into()));
            }
            if transport.wait_readable(ANSWER_POLL)? {
                break transport.recv_frame()?;
            }
        };
        match reply {
            Frame::Accept => {}
            Frame::Reject { reason } => return Err(GlowError::Rejected(reason)),
            other => return Err(GlowError::Protocol(format!("期望 ACCEPT，收到 {other:?}"))),
        }

        // 生产者：独立线程读盘并写入环形缓冲，写满即阻塞（背压）。
        let ring = Arc::new(SharedRing::new(self.ring_capacity));
        let producer = {
            let ring = Arc::clone(&ring);
            let path = path.to_path_buf();
            let buf_size = self.chunk_size;
            thread::spawn(move || -> std::io::Result<Crc32> {
                let mut file = File::open(&path)?;
                let mut crc = Crc32::new();
                let mut buf = vec![0u8; buf_size];
                loop {
                    if ring.is_closed() {
                        break;
                    }
                    let n = file.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    crc.update(&buf[..n]);
                    if ring.push_all(&buf[..n]) < n {
                        break;
                    }
                }
                ring.close();
                Ok(crc)
            })
        };

        // 消费者：本线程按网络节奏取块发送。
        let mut offset = 0u64;
        let mut buf = vec![0u8; self.chunk_size];
        let mut failure: Result<()> = Ok(());
        loop {
            let n = ring.pop_into(&mut buf);
            if n == 0 {
                break;
            }
            let frame = Frame::Chunk {
                offset,
                data: buf[..n].to_vec(),
                crc: crc32(&buf[..n]),
            };
            if let Err(e) = transport.send_frame(&frame) {
                failure = Err(e);
                break;
            }
            offset += n as u64;
            view.progress("发送", offset, size);
            if view.canceled() {
                // 告诉对端「别等了」再断，比无声无息地掐断干净：
                // 对面会显示「对方取消了这次发送」，而不是一句协议错误。
                let _ = transport.send_frame(&Frame::Reject { reason: "对方取消了这次发送".into() });
                failure = Err(GlowError::Canceled("已取消发送".into()));
                break;
            }
        }

        // 无论成功失败都关闭环并回收读盘线程，避免线程泄漏。
        ring.close();
        let produced = producer
            .join()
            .map_err(|_| GlowError::Protocol("读盘线程异常终止".into()))?;
        failure?;
        let crc = produced?;
        if offset != size {
            return Err(GlowError::Protocol(format!(
                "发送字节数与文件大小不符：{offset} != {size}"
            )));
        }
        transport.send_frame(&Frame::Done { size, crc: crc.finish() })?;
        Ok(size)
    }

    /// 接收单个文件，返回落盘路径。
    pub fn receive_file(
        &self,
        transport: &mut dyn TransferTransport,
        dir: &Path,
        peer: &str,
        view: &dyn TransferView,
    ) -> Result<PathBuf> {
        let (name, size) = match transport.recv_frame()? {
            Frame::Offer { name, size } => (name, size),
            other => return Err(GlowError::Protocol(format!("期望 OFFER，收到 {other:?}"))),
        };
        // 只取文件名部分，杜绝 ../ 路径穿越写入。
        let safe_name = sanitize(&name);

        if !view.confirm_receive(peer, &safe_name, size) {
            transport.send_frame(&Frame::Reject { reason: "用户拒绝接收".into() })?;
            return Err(GlowError::Rejected("用户拒绝接收".into()));
        }
        transport.send_frame(&Frame::Accept)?;

        fs::create_dir_all(dir)?;
        let path = unique_path(dir, &safe_name);
        let mut file = File::create(&path)?;
        let mut crc = Crc32::new();
        let mut written = 0u64;

        let result = (|| -> Result<()> {
            loop {
                match transport.recv_frame()? {
                    Frame::Chunk { offset, data, crc: chunk_crc } => {
                        if offset != written {
                            return Err(GlowError::Protocol(format!(
                                "分块乱序：期望偏移 {written}，收到 {offset}"
                            )));
                        }
                        if crc32(&data) != chunk_crc {
                            return Err(GlowError::Protocol(format!("分块校验失败：偏移 {offset}")));
                        }
                        file.write_all(&data)?;
                        crc.update(&data);
                        written += data.len() as u64;
                        view.progress("接收", written, size);
                        if view.canceled() {
                            return Err(GlowError::Canceled("已取消接收".into()));
                        }
                    }
                    Frame::Done { size: done_size, crc: done_crc } => {
                        if done_size != size {
                            return Err(GlowError::Protocol(format!(
                                "文件大小不符：声明 {size}，结束 {done_size}"
                            )));
                        }
                        if written != size {
                            return Err(GlowError::Protocol(format!(
                                "接收字节数不符：{written} != {size}"
                            )));
                        }
                        if crc.finish() != done_crc {
                            return Err(GlowError::Protocol("文件整体校验失败".into()));
                        }
                        break;
                    }
                    // 对端**中途**取消：这是一条明确结论，不是协议错误
                    //（发件人点 ✕ 时我们会收到它，而不是看到连接被掐断）
                    Frame::Reject { reason } => {
                        return Err(GlowError::Rejected(if reason.is_empty() {
                            "对方取消了这次传输".to_string()
                        } else {
                            reason
                        }))
                    }
                    other => {
                        return Err(GlowError::Protocol(format!(
                            "期望 CHUNK/DONE，收到 {other:?}"
                        )))
                    }
                }
            }
            Ok(())
        })();

        file.flush()?;
        drop(file);
        if result.is_err() {
            let _ = fs::remove_file(&path);
            return result.map(|_| path);
        }
        Ok(path)
    }
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| GlowError::Protocol(format!("文件名非法：{}", path.display())))
}

fn sanitize(name: &str) -> String {
    Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "received.bin".to_string())
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or("received");
    let ext = Path::new(name).extension().and_then(|s| s.to_str());
    for index in 1..10_000 {
        let candidate = match ext {
            Some(ext) => dir.join(format!("{stem} ({index}).{ext}")),
            None => dir.join(format!("{stem} ({index})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    candidate
}
