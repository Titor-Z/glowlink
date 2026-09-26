use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use glowlink_cli::config::load_from;
use glowlink_core::model::error::GlowError;

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "glowlink-config-{tag}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn missing_files_fall_back_to_defaults() {
    let dir = temp_dir("missing");
    let config = load_from(&[dir.join("不存在.toml")]).unwrap();
    assert_eq!(config, glowlink_core::Config::default());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn later_file_overrides_earlier_one() {
    let dir = temp_dir("overlay");
    let project = dir.join("glowlink.toml");
    let user = dir.join("user.toml");
    fs::write(&project, "discovery_port = 9000\ntransfer_port = 4000\n").unwrap();
    fs::write(&user, "transfer_port = 5000\nreceive_dir = \"/tmp/user-recv\"\n").unwrap();

    let config = load_from(&[project, user]).unwrap();
    assert_eq!(config.discovery_port, 9000, "用户配置未提及的键应保留工程值");
    assert_eq!(config.transfer_port, 5000, "用户配置应覆盖工程值");
    assert_eq!(config.receive_dir.to_string_lossy(), "/tmp/user-recv");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn broken_file_reports_its_path() {
    let dir = temp_dir("broken");
    let path = dir.join("glowlink.toml");
    fs::write(&path, "unknown_key = 1\n").unwrap();

    let error = load_from(&[path.clone()]).unwrap_err();
    let message = error.to_string();
    assert!(matches!(error, GlowError::Config(_)));
    assert!(message.contains("glowlink.toml"), "应指出出错的配置文件：{message}");
    assert!(message.contains("第 1 行"), "应指出行号：{message}");
    fs::remove_dir_all(&dir).unwrap();
}
