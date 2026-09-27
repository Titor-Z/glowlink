//! 进程退出码。
//!
//! 这是**对外契约**：脚本与 CI 会据此分支（重试、换设备、报错给人看）。
//! 改动等于改契约，必须同步 AGENTS.md 与 `glowlink --help` 里的那张表。
//!
//! ```text
//! 0  成功
//! 1  通用错误（本地文件读写、协议不符等）
//! 2  用法错误（参数写错、配置项非法、命令不存在）
//! 3  网络不可达（连不上、连断了、超时、找不到设备）
//! 4  加密/信任失败（握手失败、校验码/指纹不符）
//! 5  被拒绝或超时（对方拒绝接收、待确认超时）
//! ```

use std::io::ErrorKind;

use glowlink_core::GlowError;

pub const OK: i32 = 0;
pub const FAILURE: i32 = 1;
pub const USAGE: i32 = 2;
pub const NETWORK: i32 = 3;
pub const TRUST: i32 = 4;
pub const REJECTED: i32 = 5;

/// 错误 → 退出码。
pub fn code_of(error: &GlowError) -> i32 {
    match error {
        // 用户把参数/配置写错了：脚本该改命令，而不是重试
        GlowError::Usage(_) | GlowError::Config(_) => USAGE,
        // 加密/信任：可能是中间人，也可能是对端重置了身份；绝不能当成「重试就好」
        GlowError::Crypto(_) => TRUST,
        // 对方明确拒绝（含待确认超时后被拒）
        GlowError::Rejected(_) => REJECTED,
        // 「未找到设备」：对端不在线或还没被发现，脚本可以稍后重试
        GlowError::NotFound(_) => NETWORK,
        GlowError::Io(e) if is_network_io(e.kind()) => NETWORK,
        GlowError::Io(_)
        | GlowError::Protocol(_)
        | GlowError::Missing(_)
        // 对端（守护进程）回报的错误：种类已在 IPC 层还原，走到这里说明是它自己的问题
        | GlowError::Remote(_) => FAILURE,
    }
}

/// 网络类 IO 错误：连不上 / 连断了 / 超时 / 地址用不了。
///
/// 同一套标准也被 IPC 用来决定「错误种类名」，见 `ipc::error_kind`。
///
/// 不在此列的（文件不存在、权限不足、磁盘满）都算通用错误——它们跟网络无关，
/// 混在一起会让脚本「重试三次」重试一个永远不会成功的操作。
pub fn is_network_io(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::NotConnected
            | ErrorKind::TimedOut
            | ErrorKind::BrokenPipe
            | ErrorKind::NetworkUnreachable
            | ErrorKind::HostUnreachable
            | ErrorKind::AddrNotAvailable
    )
}
