use std::fmt;
use std::io;

/// 核心库统一错误类型。
#[derive(Debug)]
pub enum GlowError {
    Io(io::Error),
    Protocol(String),
    Rejected(String),
    /// 找不到**设备**（对端不在线 / 还没被发现），属于网络侧问题。
    NotFound(String),
    /// 本地可预期的失败（例如信任列表里没有这个名字、后台服务没跑）：与网络无关，
    /// 文案由调用方组织，**原样展示**，退出码为通用错误。
    Missing(String),
    Usage(String),
    Config(String),
    /// 对端（守护进程）返回的错误：直接展示原文，不再叠加本机前缀。
    Remote(String),
    /// 加密/认证失败。
    Crypto(String),
    /// 用户自己取消（不是故障，也不是对端拒绝）：界面据此显示「已取消」。
    Canceled(String),
}

impl fmt::Display for GlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GlowError::Io(e) => write!(f, "IO 错误：{e}"),
            GlowError::Protocol(m) => write!(f, "协议错误：{m}"),
            GlowError::Rejected(m) => write!(f, "传输被拒绝：{m}"),
            GlowError::NotFound(m) => write!(f, "未找到设备：{m}"),
            GlowError::Missing(m) => write!(f, "{m}"),
            GlowError::Usage(m) => write!(f, "用法错误：{m}"),
            GlowError::Config(m) => write!(f, "配置错误：{m}"),
            GlowError::Remote(m) => write!(f, "{m}"),
            GlowError::Crypto(m) => write!(f, "加密错误：{m}"),
            GlowError::Canceled(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for GlowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GlowError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for GlowError {
    fn from(e: io::Error) -> Self {
        GlowError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, GlowError>;
