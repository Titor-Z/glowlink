//! 设备模型与设备簿（超时自动下线）。

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use crate::model::packet::Hello;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DeviceId(String);

impl DeviceId {
    pub fn new(id: impl Into<String>) -> Self {
        DeviceId(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub addr: SocketAddr,
    pub last_seen: Instant,
}

impl Device {
    pub fn is_alive(&self, timeout: Duration) -> bool {
        self.last_seen.elapsed() < timeout
    }

    /// UI 展示用的短 id。
    pub fn short_id(&self) -> &str {
        let s = self.id.as_str();
        if s.len() > 8 {
            &s[..8]
        } else {
            s
        }
    }
}

/// 在线设备簿：只负责合并、裁剪与查找，不涉及网络。
pub struct DeviceBook {
    devices: Vec<Device>,
    timeout: Duration,
}

impl DeviceBook {
    pub fn new(timeout: Duration) -> Self {
        DeviceBook { devices: Vec::new(), timeout }
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// 收到一次广播：已存在则刷新，不存在则新增。
    pub fn observe(&mut self, hello: &Hello, from: IpAddr) -> &Device {
        let addr = SocketAddr::new(from, hello.port);
        let index = match self.devices.iter().position(|d| d.id.as_str() == hello.id) {
            Some(index) => index,
            None => {
                self.devices.push(Device {
                    id: DeviceId::new(hello.id.clone()),
                    name: hello.name.clone(),
                    addr,
                    last_seen: Instant::now(),
                });
                self.devices.len() - 1
            }
        };
        let device = &mut self.devices[index];
        device.name = hello.name.clone();
        device.addr = addr;
        device.last_seen = Instant::now();
        device
    }

    /// 移除超时设备，返回移除数量。
    pub fn prune(&mut self) -> usize {
        let timeout = self.timeout;
        let before = self.devices.len();
        self.devices.retain(|d| d.is_alive(timeout));
        before - self.devices.len()
    }

    pub fn devices(&self) -> Vec<Device> {
        self.devices.clone()
    }

    pub fn len(&self) -> usize {
        self.devices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// 按名称（精确 → 忽略大小写 → 包含）或 id（精确 → 前缀）查找。
    pub fn find(&self, key: &str) -> Option<Device> {
        let lower = key.to_lowercase();
        let by = |pred: &dyn Fn(&Device) -> bool| self.devices.iter().find(|d| pred(d)).cloned();
        by(&|d| d.name == key)
            .or_else(|| by(&|d| d.name.to_lowercase() == lower))
            .or_else(|| by(&|d| d.id.as_str() == key))
            .or_else(|| by(&|d| d.id.as_str().starts_with(key)))
            .or_else(|| by(&|d| d.name.to_lowercase().contains(&lower)))
    }
}
