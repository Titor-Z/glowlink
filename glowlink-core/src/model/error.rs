use std::fmt;
use std::io;

/// 核心库统一错误类型。
#[derive(Debug)]
pub enum GlowError {
    Io(io::Error),
    Protocol(String),
    Rejected(String),
    NotFound(String),
    Usage(String),
    Config(String),
    /// 对端（守护进程）返回的错误：直接展示原文，不再叠加本机前缀。
    Remote(String),
}

impl fmt::Display for GlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GlowError::Io(e) => write!(f, "IO 错误：{e}"),
            GlowError::Protocol(m) => write!(f, "协议错误：{m}"),
            GlowError::Rejected(m) => write!(f, "传输被拒绝：{m}"),
            GlowError::NotFound(m) => write!(f, "未找到设备：{m}"),
            GlowError::Usage(m) => write!(f, "用法错误：{m}"),
            GlowError::Config(m) => write!(f, "配置错误：{m}"),
            GlowError::Remote(m) => write!(f, "{m}"),
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
