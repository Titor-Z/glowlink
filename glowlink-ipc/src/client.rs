//! IPC 客户端：连上守护进程、发一条指令、读完整响应。
//!
//! 指令一览：
//! - `PING`                    → `OK <版本> <PID> <传输端口> <发现端口>`
//! - `STATUS`                  → `OK <设备数> <待确认数> <接收目录>`
//! - `DEVICES`                 → `OK <n>` + n 行（见 [`crate::row::DeviceRow`]）
//! - `RESOLVE <名称|ID>`       → `OK 名称\tID\t地址`
//! - `PENDING`                 → `OK <n>` + n 行（见 [`crate::row::PendingRow`]）
//! - `ACCEPT [编号]` / `REJECT [编号]` → `OK <编号>`
//! - `STOP`                    → `OK …` 并关闭连接
//!
//! 每条响应都以 `END` 收尾，所以单行与多行响应在客户端走同一条读取路径。

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use glowlink_core::model::error::{GlowError, Result};

use crate::codec::{error_from_line, END};
use crate::state::State;

/// 建立连接的等待时长：本机回环，超过这个时间说明守护进程已卡死或正在退出。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// 读一行响应的等待时长。
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// 读取一行；超时或对端关闭都算失败。
fn read_line(reader: &mut BufReader<TcpStream>) -> Result<String> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => Err(GlowError::Protocol("守护进程已断开连接".into())),
        Ok(_) => Ok(line.trim_end().to_string()),
        Err(e) => Err(GlowError::Io(e)),
    }
}

pub struct IpcClient {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}

impl IpcClient {
    pub fn connect(state: &State) -> Result<Self> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, state.ipc_port));
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.set_write_timeout(Some(READ_TIMEOUT))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(IpcClient { writer: stream, reader })
    }

    /// 发一条指令并读取完整响应：`OK …` / `ERR …` 首行 + 数据行，直到 `END`。
    pub fn call(&mut self, command: &str) -> Result<Vec<String>> {
        writeln!(self.writer, "{command}")?;
        self.writer.flush()?;
        let mut lines = Vec::new();
        loop {
            let line = read_line(&mut self.reader)?;
            if line == END {
                break;
            }
            lines.push(line);
        }
        match lines.first() {
            Some(first) if first.starts_with("ERR ") => Err(error_from_line(first)),
            Some(_) => Ok(lines),
            None => Err(GlowError::Protocol("守护进程返回了空响应".into())),
        }
    }

    /// 只取首行（单行指令的常用形态）。
    pub fn call_first(&mut self, command: &str) -> Result<String> {
        Ok(self.call(command)?.remove(0))
    }
}
