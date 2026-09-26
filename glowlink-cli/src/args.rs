//! 命令行参数解析：无第三方依赖的手写实现。
//!
//! 约定：`VALUE_FLAGS` 中的开关会吃掉紧随其后的一整个参数作为取值，
//! 其余 `--` 开头的参数视为布尔开关，非 `--` 开头的参数视为位置参数。

use std::time::Duration;

use glowlink_core::{GlowError, Result};

/// 会吃掉一个值的开关（带前导 `--`，比对时使用完整开关名）。
pub const VALUE_FLAGS: [&str; 5] = ["--timeout", "--port", "--dir", "--discovery-port", "--peer"];

pub fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

pub fn flag_value(args: &[String], name: &str) -> Option<String> {
    let position = args.iter().position(|a| a == name)?;
    args.get(position + 1).cloned()
}

pub fn flag_secs(args: &[String], name: &str) -> Option<Duration> {
    flag_value(args, name)
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
}

pub fn flag_u16(args: &[String], name: &str) -> Result<Option<u16>> {
    match flag_value(args, name) {
        None => Ok(None),
        Some(value) => value
            .parse::<u16>()
            .map(Some)
            .map_err(|_| GlowError::Usage(format!("{name} 需要 0-65535 的端口号，收到「{value}」"))),
    }
}

/// 取出位置参数，跳过开关及其取值。
pub fn positional_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_value = false;
    for arg in args {
        if skip_value {
            skip_value = false;
            continue;
        }
        if arg.starts_with("--") {
            if VALUE_FLAGS.contains(&arg.as_str()) {
                skip_value = true;
            }
            continue;
        }
        out.push(arg.clone());
    }
    out
}

pub fn truncate(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}
