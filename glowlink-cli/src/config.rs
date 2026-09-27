//! 配置文件的定位与加载（IO 层，放在 CLI 侧）。
//!
//! 优先级从低到高：
//! 1. `glowlink-core` 里的内置默认（[`Config::default`]）；
//! 2. **编译期嵌入的工程配置** `glowlink.toml`（`include_str!`）；
//! 3. 用户配置 `~/.config/glowlink/config.toml`（或 `$GLOWLINK_CONFIG`）；
//! 4. 命令行开关（由 `main.rs` 覆盖）。
//!
//! 工程配置为何是编译期的：它是**开发者写工程常量**的地方（端口、超时、分块大小），
//! 不是给用户的运行时文件。所以它既不随包分发，也不在运行时按当前目录去摸文件——
//! 想改就改仓库里那份再重新编译，也不用担心「运行目录里躺着一份旧的」。

use std::fs;
use std::path::PathBuf;

use glowlink_core::model::error::{GlowError, Result};
use glowlink_core::Config;

/// 工程配置：**编进二进制**。
///
/// 路径相对本文件，指向仓库根部的 `glowlink.toml`；
/// 改了它必须重新编译才生效（这是刻意的，不是缺陷）。
pub const PROJECT_CONFIG_TEXT: &str = include_str!("../../glowlink.toml");

/// 工程配置在报错/展示时的名字。
pub const PROJECT_CONFIG_SOURCE: &str = "glowlink.toml（编译期嵌入）";

/// 用户配置文件可由环境变量指定，便于多套环境切换与自动化测试。
pub const CONFIG_ENV: &str = "GLOWLINK_CONFIG";

/// 按优先级返回候选的**用户**配置文件路径。
///
/// 工程配置已在编译期生效，不在这里——否则它会被找两次（一次旧的、一次新的）。
pub fn config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = std::env::var_os(CONFIG_ENV) {
        paths.push(PathBuf::from(path));
    } else if let Some(home) = home_dir() {
        paths.push(home.join(".config").join("glowlink").join("config.toml"));
    }
    paths
}

/// 内置默认 + 编译期嵌入的工程配置。
pub fn project_config() -> Result<Config> {
    let mut config = Config::default();
    config
        .apply(PROJECT_CONFIG_TEXT)
        .map_err(|e| annotate(e, PROJECT_CONFIG_SOURCE))?;
    Ok(config)
}

/// 加载生效配置；用户层文件不存在即跳过，只是那层没有覆盖。
pub fn load() -> Result<Config> {
    load_from(&config_paths())
}

/// 在「工程配置」之上按给定顺序逐层叠加用户层。
pub fn load_from(paths: &[PathBuf]) -> Result<Config> {
    let mut config = project_config()?;
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(path)
            .map_err(|e| GlowError::Config(format!("读取 {} 失败：{e}", path.display())))?;
        config
            .apply(&text)
            .map_err(|e| annotate(e, &path.display().to_string()))?;
    }
    Ok(config)
}

/// 给配置错误补上「哪来的这份文本」，否则用户只知道行号、不知道是哪个文件。
fn annotate(error: GlowError, source: &str) -> GlowError {
    match error {
        GlowError::Config(message) => GlowError::Config(format!("{source} {message}")),
        other => other,
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
