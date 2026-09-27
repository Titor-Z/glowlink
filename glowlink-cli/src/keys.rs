//! 身份密钥文件与信任列表（IO 层）。
//!
//! 两个文件都放在 `~/.glowlink/` 下，**行式文本**、原子写入（临时文件 + rename）：
//! - `identity.txt`：本机长期身份私钥 + 公钥（Unix 上 0600）；
//!   路径可用 `$GLOWLINK_IDENTITY` 覆盖（同机模拟两台设备时用）
//! - `trusted.txt`：已配对设备（每行 `指纹\t名称\t配对时间`）
//!
//! 为什么不用 JSON：这两个文件都是「一行一条记录」的平面结构，
//! 手写行解析比手写 JSON 数组解析更短、更好核对，也符合本项目「可读、可手改」的风格。
//! 密钥是秘密，所以初始化时把权限收紧到 0600（Windows 依赖用户目录 ACL）。

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use glowlink_core::model::error::{GlowError, Result};
use glowlink_core::patterns::secure::TrustList;
use glowlink_core::Identity;

use crate::state::state_dir;

/// 身份密钥文件名。
const IDENTITY_FILE: &str = "identity.txt";
/// 信任列表文件名。
const TRUST_FILE: &str = "trusted.txt";

/// 身份密钥文件路径。
///
/// 可用 `$GLOWLINK_IDENTITY` 直接指定文件，便于**同一台机器模拟两台设备**
/// （否则同机同用户的进程共享同一份身份，两端指纹会一样）。
pub fn identity_path() -> PathBuf {
    match std::env::var_os("GLOWLINK_IDENTITY") {
        Some(path) => PathBuf::from(path),
        None => state_dir().join(IDENTITY_FILE),
    }
}

/// 信任列表文件路径。
pub fn trust_path() -> PathBuf {
    state_dir().join(TRUST_FILE)
}

// ---------------------------------------------------------------- 身份密钥

/// 读取本机长期身份；不存在就现场生成并落盘。
pub fn load_or_create_identity() -> Result<Arc<Identity>> {
    load_or_create_identity_at(&identity_path())
}

/// 指定路径版本（便于测试）。
pub fn load_or_create_identity_at(path: &PathBuf) -> Result<Arc<Identity>> {
    if path.is_file() {
        let text = fs::read_to_string(path)?;
        let secret = field(&text, "secret")
            .ok_or_else(|| GlowError::Config(format!("{} 缺少 secret 字段", path.display())))?;
        let bytes = from_hex(&secret).ok_or_else(|| {
            GlowError::Config(format!("{} 的 secret 不是合法的 64 位十六进制", path.display()))
        })?;
        let secret_bytes: [u8; 32] = bytes.try_into().map_err(|_| {
            GlowError::Config(format!("{} 的 secret 长度不是 32 字节", path.display()))
        })?;
        let identity = Identity::from_secret_bytes(secret_bytes);
        // 公钥与私钥不一致 ⇒ 文件被改坏，直接报错而不是默默用错身份
        if let Some(stored) = field(&text, "public") {
            if stored != hex(&identity.public_bytes()) {
                return Err(GlowError::Config(format!(
                    "{} 的公钥与私钥不匹配（文件可能被改动）",
                    path.display()
                )));
            }
        }
        return Ok(Arc::new(identity));
    }

    let identity = Identity::generate();
    let secret = hex(&identity.secret_bytes()[..]);
    let body = format!(
        "# 萤光 GlowLink 设备身份：私钥请勿外传，删除本文件即视为换了一台设备\n\
         secret={secret}\n\
         public={}\n",
        hex(&identity.public_bytes())
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_private(path, body.as_bytes())?;
    Ok(Arc::new(identity))
}

/// 取 `key=value` 行里的值（`#` 开头是注释）。
fn field(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    text.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&prefix).map(|v| v.trim().to_string()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// 原子写入 + 收紧权限（Unix 0600；Windows 依赖用户目录 ACL）。
fn write_private(path: &PathBuf, body: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

// ---------------------------------------------------------------- 信任列表

/// 一台已配对的设备。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedDevice {
    pub fingerprint: String,
    pub name: String,
    /// 配对时间（Unix 秒）。
    pub paired_at: u64,
}

/// 信任列表：既是 [`TrustList`] 的读取实现，也负责把新配对写回磁盘。
pub struct TrustFile {
    path: PathBuf,
    devices: Mutex<Vec<TrustedDevice>>,
}

impl TrustFile {
    /// 打开（不存在则视为空列表）。
    pub fn open(path: PathBuf) -> Result<Arc<Self>> {
        let devices = if path.is_file() {
            parse(&fs::read_to_string(&path)?)
        } else {
            Vec::new()
        };
        Ok(Arc::new(TrustFile { path, devices: Mutex::new(devices) }))
    }

    /// 默认位置 `~/.glowlink/trusted.txt`。
    pub fn open_default() -> Result<Arc<Self>> {
        Self::open(trust_path())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<TrustedDevice>> {
        self.devices.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn devices(&self) -> Vec<TrustedDevice> {
        self.lock().clone()
    }

    pub fn contains(&self, fingerprint: &str) -> bool {
        self.lock().iter().any(|d| d.fingerprint == fingerprint)
    }

    /// 记住一台设备（已存在则只更新名称）。用户确认配对后调用。
    pub fn trust(&self, fingerprint: &str, name: &str) -> Result<()> {
        let mut devices = self.lock();
        if let Some(existing) = devices.iter_mut().find(|d| d.fingerprint == fingerprint) {
            existing.name = name.to_string();
        } else {
            devices.push(TrustedDevice {
                fingerprint: fingerprint.to_string(),
                name: name.to_string(),
                paired_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            });
        }
        self.save(&devices)
    }

    /// 忘记一台设备；返回是否真的删掉了。
    pub fn forget(&self, fingerprint: &str) -> Result<bool> {
        let mut devices = self.lock();
        let before = devices.len();
        devices.retain(|d| d.fingerprint != fingerprint);
        let removed = devices.len() != before;
        if removed {
            self.save(&devices)?;
        }
        Ok(removed)
    }

    fn save(&self, devices: &[TrustedDevice]) -> Result<()> {
        let mut body = String::from(
            "# 萤光 GlowLink 已配对设备：每行「指纹\\t名称\\t配对时间」，删掉一行即取消该设备的免确认\n",
        );
        for device in devices {
            body.push_str(&format!(
                "{}\t{}\t{}\n",
                device.fingerprint, device.name, device.paired_at
            ));
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_private(&self.path, body.as_bytes())
    }
}

impl TrustList for TrustFile {
    fn is_trusted(&self, fingerprint: &str) -> bool {
        self.contains(fingerprint)
    }

    fn name_of(&self, fingerprint: &str) -> Option<String> {
        self.lock()
            .iter()
            .find(|d| d.fingerprint == fingerprint)
            .map(|d| d.name.clone())
    }
}

/// 指纹是否合法：固定 16 位十六进制（与 `fingerprint_of` 的输出一致）。
///
/// 这一层校验是必要的：手改坏的行（比如只写了设备名）若被当成指纹收下，
/// 就等于凭空多出一台「已信任设备」。
fn is_fingerprint(text: &str) -> bool {
    text.len() == 16 && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// 解析信任列表；坏行直接跳过（列表不是秘密，容错比报错更实用）。
fn parse(text: &str) -> Vec<TrustedDevice> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let fingerprint = parts.next()?.trim();
            if !is_fingerprint(fingerprint) {
                return None;
            }
            let name = parts.next().unwrap_or("").trim();
            let paired_at = parts.next().unwrap_or("0").trim().parse().unwrap_or(0);
            Some(TrustedDevice {
                fingerprint: fingerprint.to_string(),
                name: name.to_string(),
                paired_at,
            })
        })
        .collect()
}
