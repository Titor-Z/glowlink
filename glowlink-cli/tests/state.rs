//! `glowlink-cli/src/state.rs` 的单元测试：状态文件读写与路径。

use std::env;
use std::sync::{Mutex, MutexGuard};

use glowlink_cli::state::{remove_state, state_path, write_state, State};

/// 这些用例改的是**进程级**环境变量 `HOME`，而测试默认并行跑：
/// 不串行化就会互相把 HOME 改到别的目录，写出「写 A 读 B」的假故障。
static HOME_LOCK: Mutex<()> = Mutex::new(());

/// 把 HOME 指到临时目录，避免测试碰用户真实的 ~/.glowlink。
///
/// 返回的守卫必须持有到用例结束（`let _guard = isolate_home(...)`）。
fn isolate_home(tag: &str) -> MutexGuard<'static, ()> {
    let guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = env::temp_dir().join(format!("glowlink-state-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    env::set_var("HOME", &dir);
    guard
}

fn sample() -> State {
    State {
        version: "0.2.0".to_string(),
        pid: 4242,
        ipc_port: 10_000,
        transfer_port: 51_234,
        discovery_port: 9267,
        receive_dir: "/tmp/接收 目录".into(),
        started_at: 1_700_000_000,
    }
}

#[test]
fn json_roundtrip_preserves_every_field() {
    let state = sample();
    assert_eq!(State::parse(&state.to_json()).unwrap(), state);
}

#[test]
fn json_escapes_quotes_and_backslashes() {
    let mut state = sample();
    state.receive_dir = r#"/tmp/带"引号"\与反斜杠"#.into();
    assert_eq!(State::parse(&state.to_json()).unwrap().receive_dir, state.receive_dir);
}

#[test]
fn parse_rejects_corrupt_file() {
    // 缺字段必须报错：不能默默补默认值让客户端连到错误的端口
    assert!(State::parse(r#"{"version":"0.2.0","pid":1}"#).is_err());
    assert!(State::parse("完全不是 JSON").is_err());
}

#[test]
fn write_then_read_is_atomic_and_leaves_no_temp_file() {
    let _guard = isolate_home("atomic");
    let state = sample();
    write_state(&state).unwrap();

    assert!(state_path().is_file(), "状态文件应已写入");
    let tmp = state_path().with_extension("json.tmp");
    assert!(!tmp.exists(), "临时文件必须已被 rename 掉，不能残留");

    let text = std::fs::read_to_string(state_path()).unwrap();
    assert_eq!(State::parse(&text).unwrap(), state);
}

#[test]
fn write_is_idempotent_and_overwrites() {
    let _guard = isolate_home("overwrite");
    write_state(&sample()).unwrap();
    let mut second = sample();
    second.pid = 777;
    write_state(&second).unwrap();

    let text = std::fs::read_to_string(state_path()).unwrap();
    assert_eq!(State::parse(&text).unwrap().pid, 777);
}

#[test]
fn remove_state_is_ok_when_missing_and_then_effective() {
    let _guard = isolate_home("remove");
    // 不存在时也应成功（off 在没跑过守护进程的机器上不该报错）
    remove_state().unwrap();
    write_state(&sample()).unwrap();
    assert!(state_path().is_file());
    remove_state().unwrap();
    assert!(!state_path().exists(), "off 之后状态文件必须消失，否则会误判成还在跑");
    // 再删一次仍然成功
    remove_state().unwrap();
}

#[test]
fn paths_live_under_dot_glowlink() {
    let _guard = isolate_home("paths");
    let dir = glowlink_cli::state::state_dir();
    assert!(dir.ends_with(".glowlink"), "实际：{}", dir.display());
    assert_eq!(glowlink_cli::state::state_path(), dir.join("state.json"));
    assert_eq!(glowlink_cli::state::log_path(), dir.join("daemon.log"));
}
