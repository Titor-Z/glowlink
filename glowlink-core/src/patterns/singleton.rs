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
        INSTANCE.get_or_init(|| DeviceIdentity::new(format!("{:016x}", seed()), detect_name()))
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

/// 按优先级探测本机可读名称：环境变量 → 系统设置里的电脑名 → hostname。
///
/// 直接用 `hostname` 往往拿到一串自动生成的名字（例如 macOS 默认的 `xxxxxdeMac-mini`），
/// 用户根本认不出来，所以先问系统要“电脑名称”。
fn detect_name() -> String {
    // 显式指定优先级最高，便于一台机器上跑多个节点或做测试。
    if let Ok(value) = std::env::var("GLOWLINK_NAME") {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    for candidate in platform_names() {
        let candidate = candidate.trim();
        if candidate.is_empty() || is_placeholder(candidate) {
            continue;
        }
        return candidate.to_string();
    }
    system_hostname().unwrap_or_else(|| "glowlink-node".to_string())
}

/// 平台提供的可读电脑名（取不到就返回空）。
fn platform_names() -> Vec<String> {
    let mut names = Vec::new();
    let commands: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[
            // 系统设置 → 通用 → 关于本机 → 名称
            ("scutil", &["--get", "ComputerName"]),
            // 更完整的本地主机名，例如 "titor-z的Mac mini.local"
            ("scutil", &["--get", "LocalHostName"]),
        ]
    } else if cfg!(target_os = "windows") {
        &[("powershell", &["-NoProfile", "-Command", "$env:COMPUTERNAME"])]
    } else {
        &[]
    };

    for (program, args) in commands {
        if let Some(value) = run(program, args) {
            names.push(value);
        }
    }

    // 环境变量兜底：Windows 的 COMPUTERNAME、类 Unix 的 HOSTNAME。
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(value) = std::env::var(key) {
            names.push(value);
        }
    }
    names
}

fn system_hostname() -> Option<String> {
    run("hostname", &[]).or_else(|| {
        std::env::var("HOSTNAME")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    })
}

/// 这些名字没有任何辨识度，宁可继续往后找。
fn is_placeholder(name: &str) -> bool {
    matches!(name.to_lowercase().as_str(), "localhost" | "localhost.localdomain")
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}
