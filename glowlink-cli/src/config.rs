//! 配置文件的定位与加载（IO 层，放在 CLI 侧）。
//!
//! 优先级从低到高：
//! 1. `glowlink-core` 里的内置默认（[`Config::default`]）；
//! 2. 当前目录的 `glowlink.toml`（随工程分发的默认配置）；
//! 3. 用户配置 `~/.config/glowlink/config.toml`（或 `$GLOWLINK_CONFIG`）；
//! 4. 命令行开关（由 `main.rs` 覆盖）。

use std::fs;
use std::path::PathBuf;

use glowlink_core::model::error::{GlowError, Result};
use glowlink_core::Config;

/// 工程配置文件（相对当前工作目录）。
pub const PROJECT_CONFIG: &str = "glowlink.toml";

/// 用户配置文件可由环境变量指定，便于多套环境切换与自动化测试。
pub const CONFIG_ENV: &str = "GLOWLINK_CONFIG";

/// 按优先级返回候选配置文件路径。
pub fn config_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from(PROJECT_CONFIG)];
    if let Some(path) = std::env::var_os(CONFIG_ENV) {
        paths.push(PathBuf::from(path));
    } else if let Some(home) = home_dir() {
        paths.push(home.join(".config").join("glowlink").join("config.toml"));
    }
    paths
}

/// 加载生效配置；文件不存在即跳过，只是那层没有覆盖。
pub fn load() -> Result<Config> {
    load_from(&config_paths())
}

/// 按给定顺序逐层叠加配置文件。
pub fn load_from(paths: &[PathBuf]) -> Result<Config> {
    let mut config = Config::default();
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(path).map_err(|e| {
            GlowError::Config(format!("读取 {} 失败：{e}", path.display()))
        })?;
        config.apply(&text).map_err(|e| match e {
            GlowError::Config(message) => {
                GlowError::Config(format!("{} {}", path.display(), message))
            }
            other => other,
        })?;
    }
    Ok(config)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
