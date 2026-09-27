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
    /// ID 由 [`install_id`] 注入（CLI 用持久身份里的指纹），因此**跨运行稳定**；
    /// 若没人注入（例如单元测试、无身份的极简用法），退回进程内临时 ID。
    pub fn global() -> &'static DeviceIdentity {
        static INSTANCE: OnceLock<DeviceIdentity> = OnceLock::new();
        INSTANCE.get_or_init(|| {
            let id = INSTALLED_ID
                .get()
                .cloned()
                .unwrap_or_else(|| format!("{:016x}", seed()));
            DeviceIdentity::new(id, detect_name())
        })
    }
}

/// 由调用方注入的本机 ID（持久身份的指纹）。
static INSTALLED_ID: OnceLock<String> = OnceLock::new();

/// 注入本机 ID。**必须在第一次 [`DeviceIdentity::global`] 之前调用**。
///
/// 为什么要注入而不是自己算：面板上显示的设备 ID 若每次启动都变，
/// 用户会怀疑「这还是不是同一台机器」，对端也会把本机当成新设备。
/// ID 取自持久身份指纹，与信任列表是同一套身份，只是显示长度不同。
///
/// 返回 `false` 表示已经注入过（重复调用无副作用）；若在 `global()` 之后调用，
/// 因为实例已经用临时 ID 初始化，本次注入不会生效。
pub fn install_id(id: impl Into<String>) -> bool {
    INSTALLED_ID.set(id.into()).is_ok()
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
