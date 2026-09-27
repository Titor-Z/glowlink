//! IPC 数据行：设备表与待确认表在协议上的表示。
//!
//! 为什么单独一层：`DEVICES` / `PENDING` 的**每一行都是跨进程契约**，
//! 字段顺序与分隔符一改就是改协议，必须与守护进程内部的数据结构分开。
//! 守护进程内部用 `PendingOffer`（含队列语义），协议上用 [`PendingRow`]；
//! 两者靠显式转换相连，内部结构变化不会悄悄改掉线上格式。
//!
//! 行格式（字段间是 `\t`，行首是 `OK <数量>`）：
//! ```text
//! DEVICES  名称 \t ID \t 地址 \t 存活毫秒 \t 类型 \t 是否已信任
//! PENDING  编号 \t 来源 \t 文件名 \t 字节数 \t 指纹 \t 校验码
//! PAIRINGS / PAIRPENDING  编号 \t 对端 \t 校验码 \t 状态 \t 剩余尝试
//! ```

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use glowlink_core::model::packet::DeviceKind;
use glowlink_core::{Device, DeviceId};

/// `DEVICES` 的一行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRow {
    pub name: String,
    pub id: String,
    pub addr: SocketAddr,
    /// 最近一次露面的「多少毫秒之前」。传时长而不是时间戳：
    /// 两端时钟不同源，绝对时间没有可比性。
    pub age_ms: u128,
    /// 设备类型（界面用；同样放在末尾，缺了就是未知）。
    pub kind: DeviceKind,
    /// 是否在信任列表里（界面据此决定「要不要请人核对校验码」）。
    ///
    /// 同样容忍缺失：没有这一列的老服务端一律当**未信任**——
    /// 猜「已信任」等于替用户省掉核对，那正是不能省的。
    pub trusted: bool,
}

impl DeviceRow {
    pub fn from_device(device: &Device) -> Self {
        DeviceRow {
            name: device.name.clone(),
            id: device.id.as_str().to_string(),
            addr: device.addr,
            age_ms: device.last_seen.elapsed().as_millis(),
            kind: device.kind,
            trusted: false,
        }
    }

    /// 还原成 `Device`：用「现在减去已存活时长」反推最近出现时刻。
    pub fn to_device(&self) -> Device {
        Device {
            id: DeviceId::new(self.id.clone()),
            name: self.name.clone(),
            addr: self.addr,
            last_seen: Instant::now() - Duration::from_millis(self.age_ms as u64),
            kind: self.kind,
        }
    }

    pub fn to_line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            self.name,
            self.id,
            self.addr,
            self.age_ms,
            self.kind.as_token(),
            if self.trusted { 1 } else { 0 }
        )
    }

    pub fn from_line(line: &str) -> Option<Self> {
        let mut parts = line.split('\t');
        let name = parts.next()?.to_string();
        let id = parts.next()?.to_string();
        let addr = parts.next()?.parse().ok()?;
        let age_ms = parts.next()?.parse().ok()?;
        // 类型是后加的：老行没有这一列，按未知处理
        let kind = parts.next().map(DeviceKind::from_token).unwrap_or_default();
        let trusted = parts.next().map(|v| v == "1").unwrap_or(false);
        Some(DeviceRow { name, id, addr, age_ms, kind, trusted })
    }
}

/// 配对槽位的状态（IPC 行里的可读名字）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairingState {
    /// 本机发起：已握手、码已可显示，等对方拍板。
    WaitingPeer,
    /// 对端发起：等本机用户决定（允许并输码 / 拒绝）。
    WaitingUser,
    /// 已配对成功（指纹已写入信任列表）。
    Paired,
    /// 被拒绝（用户点了拒绝、或码错到次数用尽）。
    Denied,
    /// 出错（对端断开、对方超时）。
    Failed,
}

impl PairingState {
    pub fn as_token(self) -> &'static str {
        match self {
            PairingState::WaitingPeer => "waiting_peer",
            PairingState::WaitingUser => "waiting_user",
            PairingState::Paired => "paired",
            PairingState::Denied => "denied",
            PairingState::Failed => "failed",
        }
    }

    pub fn from_token(token: &str) -> Self {
        match token {
            "waiting_peer" => PairingState::WaitingPeer,
            "paired" => PairingState::Paired,
            "denied" => PairingState::Denied,
            "failed" => PairingState::Failed,
            _ => PairingState::WaitingUser,
        }
    }
}

/// `PAIRINGS` / `PAIRPENDING` 的一行：
/// `编号 \t 对端 \t 校验码 \t 状态 \t 剩余尝试`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingRow {
    pub token: String,
    /// 对端设备名（对方自报，仅供显示）。
    pub peer: String,
    /// 6 位校验码：本机发起时要显示给人看，对端发起时供人核对。
    pub code: String,
    pub state: PairingState,
    /// 还剩几次输入机会（只对 `WaitingUser` 有意义）。
    pub remaining: u8,
}

impl PairingRow {
    pub fn to_line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}",
            self.token,
            self.peer,
            self.code,
            self.state.as_token(),
            self.remaining
        )
    }

    /// 状态认不出时按 `WaitingUser` 处理：宁可多问一次人，也不要静默当成已配对。
    pub fn from_line(line: &str) -> Option<Self> {
        let mut parts = line.split('\t');
        let token = parts.next()?.to_string();
        let peer = parts.next()?.to_string();
        let code = parts.next()?.to_string();
        let state = PairingState::from_token(parts.next()?);
        let remaining = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        Some(PairingRow { token, peer, code, state, remaining })
    }
}

/// `PENDING` 的一行：一个等待用户决定的接收请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingRow {
    /// 给用户手敲的短编号。
    pub token: String,
    /// 来源设备名。
    pub peer: String,
    pub name: String,
    pub size: u64,
    /// 对端设备指纹。
    pub fingerprint: String,
    /// 6 位校验码（两端相同，供人工比对）。
    pub code: String,
}

impl PendingRow {
    pub fn to_line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            self.token, self.peer, self.name, self.size, self.fingerprint, self.code
        )
    }

    /// 允许指纹与校验码缺字段（老守护进程不带这两列），其余字段缺了就是坏行。
    pub fn from_line(line: &str) -> Option<Self> {
        let mut parts = line.split('\t');
        let token = parts.next()?.to_string();
        let peer = parts.next()?.to_string();
        let name = parts.next()?.to_string();
        let size = parts.next()?.parse().ok()?;
        let fingerprint = parts.next().unwrap_or("").to_string();
        let code = parts.next().unwrap_or("").to_string();
        Some(PendingRow { token, peer, name, size, fingerprint, code })
    }
}
