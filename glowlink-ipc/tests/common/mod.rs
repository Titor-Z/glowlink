//! 测试共用的替身与样例（不是测试目标本身，只被各测试文件 `mod common;` 引入）。
//!
//! 替身只实现 `IpcHost`——协议分支因此可以逐条覆盖，不必真的起一个守护进程。

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use glowlink_core::model::error::{GlowError, Result};
use glowlink_core::model::packet::DeviceKind;
use glowlink_core::{Device, DeviceId};
use glowlink_ipc::row::{
    PairingRow, PairingState, PendingRow, SendTicket, TransferRow, TransferState,
};
use glowlink_ipc::server::IpcHost;
use glowlink_ipc::state::State;

pub fn sample_state() -> State {
    State {
        version: "0.3.0".to_string(),
        pid: 4242,
        ipc_port: 10_000,
        transfer_port: 51_234,
        discovery_port: 9267,
        receive_dir: "/tmp/接收 目录".into(),
        started_at: 1_700_000_000,
    }
}

pub fn a_device(name: &str, addr: SocketAddr) -> Device {
    Device {
        id: DeviceId::new("abc123def4567890"),
        name: name.to_string(),
        addr,
        last_seen: Instant::now(),
        kind: DeviceKind::Desktop,
    }
}

pub fn pending_row(token: &str, peer: &str, name: &str, size: u64) -> PendingRow {
    PendingRow {
        token: token.to_string(),
        peer: peer.into(),
        name: name.into(),
        size,
        fingerprint: "abcdef0123456789".into(),
        code: "482913".into(),
    }
}

pub fn pairing_row(token: &str, peer: &str, state: PairingState, remaining: u8) -> PairingRow {
    PairingRow {
        token: token.to_string(),
        peer: peer.to_string(),
        code: "482913".to_string(),
        state,
        remaining,
    }
}

/// 测试替身：把守护进程状态塞进内存，覆盖协议各分支。
#[derive(Default)]
pub struct FakeHost {
    pub devices: Mutex<Vec<Device>>,
    pub pending: Mutex<Vec<PendingRow>>,
    pub decided: Mutex<Vec<(String, bool)>>,
    pub stopped: AtomicBool,
    /// 本机发起的配对槽位。
    pub pairings: Mutex<Vec<PairingRow>>,
    /// 对端发起、等本机决定的配对请求。
    pub pair_pending: Mutex<Vec<PairingRow>>,
    /// 用户落下的配对决定：`(编号, 用户输入的码；None = 拒绝)`。
    pub pair_decisions: Mutex<Vec<(String, Option<String>)>>,
    /// 已信任的设备 ID（指纹）。
    pub trusted: Mutex<Vec<String>>,
    /// 被请求发送的 `(设备 ID, 路径)`。
    pub sent: Mutex<Vec<(String, String)>>,
    /// 要展示的传输行。
    pub transfers: Mutex<Vec<TransferRow>>,
    /// 被取消的传输编号（测 `CANCEL` 打到宿主上）。
    pub canceled: Mutex<Vec<String>>,
    /// 被放弃的配对编号（测 `PAIR_CANCEL`）。
    pub pair_canceled: Mutex<Vec<String>>,
    /// 让 `send_file` 报错（测错误分支）。
    pub send_fails: AtomicBool,
}

impl IpcHost for FakeHost {
    fn status(&self) -> State {
        sample_state()
    }

    fn is_trusted(&self, device_id: &str) -> bool {
        self.trusted
            .lock()
            .unwrap()
            .iter()
            .any(|id| id == device_id)
    }

    fn devices(&self) -> Vec<Device> {
        self.devices.lock().unwrap().clone()
    }

    fn find(&self, key: &str) -> Option<Device> {
        self.devices
            .lock()
            .unwrap()
            .iter()
            .find(|d| d.name == key || d.id.as_str() == key)
            .cloned()
    }

    fn pending(&self) -> Vec<PendingRow> {
        self.pending.lock().unwrap().clone()
    }

    fn decide(&self, token: Option<&str>, accept: bool) -> Result<String> {
        let mut pending = self.pending.lock().unwrap();
        let token = match token {
            Some(token) => token.to_string(),
            None if pending.len() == 1 => pending[0].token.clone(),
            None if pending.is_empty() => {
                return Err(GlowError::Usage("当前没有待确认的接收请求".into()))
            }
            None => {
                return Err(GlowError::Usage(format!(
                    "有 {} 个待确认请求，请指定编号",
                    pending.len()
                )))
            }
        };
        if !pending.iter().any(|row| row.token == token) {
            return Err(GlowError::Usage(format!("没有编号为 {token} 的待确认请求")));
        }
        self.decided.lock().unwrap().push((token.clone(), accept));
        pending.retain(|row| row.token != token);
        Ok(token)
    }

    fn request_stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    fn send_file(&self, device_id: &str, path: &str) -> Result<SendTicket> {
        self.sent.lock().unwrap().push((device_id.to_string(), path.to_string()));
        if self.send_fails.load(Ordering::SeqCst) {
            // 网络类错误：客户端要靠这个种类还原退出码
            return Err(GlowError::NotFound(device_id.to_string()));
        }
        Ok(SendTicket { token: "001".to_string(), code: "513904".to_string() })
    }

    fn cancel(&self, token: &str) -> Result<String> {
        let mut rows = self.transfers.lock().unwrap();
        let Some(index) = rows
            .iter()
            .position(|row| row.token == token && row.state == TransferState::Running)
        else {
            return Err(GlowError::Missing(format!("没有正在进行的传输 #{token}")));
        };
        rows[index].state = TransferState::Canceled;
        self.canceled.lock().unwrap().push(token.to_string());
        Ok(token.to_string())
    }

    fn transfers(&self) -> Vec<TransferRow> {
        self.transfers.lock().unwrap().clone()
    }

    fn pair(&self, key: &str) -> Result<(String, String)> {
        if key == "幽灵机" {
            return Err(GlowError::NotFound(key.to_string()));
        }
        Ok(("001".to_string(), "482913".to_string()))
    }

    fn pairings(&self) -> Vec<PairingRow> {
        self.pairings.lock().unwrap().clone()
    }

    fn pair_pending(&self) -> Vec<PairingRow> {
        self.pair_pending.lock().unwrap().clone()
    }

    fn pair_decide(&self, token: &str, code: Option<String>) -> Result<String> {
        if !self
            .pair_pending
            .lock()
            .unwrap()
            .iter()
            .any(|row| row.token == token)
        {
            return Err(GlowError::Usage(format!("没有编号为 {token} 的配对请求")));
        }
        self.pair_decisions
            .lock()
            .unwrap()
            .push((token.to_string(), code));
        Ok(token.to_string())
    }

    fn pair_cancel(&self, token: &str) -> Result<String> {
        let mut canceled = self.pair_canceled.lock().unwrap();
        if canceled.contains(&token.to_string()) {
            return Err(GlowError::Missing(format!("配对 #{token} 已经结束了")));
        }
        canceled.push(token.to_string());
        Ok(token.to_string())
    }
}
