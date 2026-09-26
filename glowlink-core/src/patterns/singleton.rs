//! 单例：本机设备身份。全局唯一，首次访问时惰性初始化。

use std::process::Command;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub id: String,
    pub name: String,
}

impl DeviceIdentity {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        DeviceIdentity { id: id.into(), name: name.into() }
    }

    /// 全局唯一实例。
    ///
    /// 说明：id 由启动时间与进程号派生，因此每次启动都会变化；
    /// 设备指纹持久化（记住已信任设备）留到 Phase 4 加密配对时实现。
    pub fn global() -> &'static DeviceIdentity {
        static INSTANCE: OnceLock<DeviceIdentity> = OnceLock::new();
        INSTANCE.get_or_init(|| DeviceIdentity::new(format!("{:016x}", seed()), hostname()))
    }
}

fn seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in nanos.to_le_bytes().iter().chain(pid.to_le_bytes().iter()) {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn hostname() -> String {
    for key in ["GLOWLINK_NAME", "HOSTNAME", "COMPUTERNAME"] {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                return value;
            }
        }
    }
    Command::new("hostname")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "glowlink-node".to_string())
}
