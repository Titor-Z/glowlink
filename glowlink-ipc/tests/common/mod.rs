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
use glowlink_ipc::row::{PairingRow, PairingState, PendingRow};
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
}

impl IpcHost for FakeHost {
    fn status(&self) -> State {
        sample_state()
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
}
