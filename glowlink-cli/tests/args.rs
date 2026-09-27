use glowlink_cli::args::{flag_secs, flag_u16, flag_value, has_flag, positional_args};

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn value_flags_swallow_their_value() {
    let a = args(&["--peer", "127.0.0.1:8888", "./a.bin"]);
    assert_eq!(positional_args(&a), args(&["./a.bin"]), "--peer 的取值不应被当成文件");
}

#[test]
fn every_value_flag_is_registered() {
    // 这条用例防止新增开关时忘记登记，导致取值混进位置参数。
    for flag in [
        "--timeout",
        "--port",
        "--dir",
        "--discovery-port",
        "--discovery-port-span",
        "--peer",
    ] {
        let a = args(&[flag, "取值", "文件"]);
        assert_eq!(positional_args(&a), args(&["文件"]), "{flag} 未登记为取值开关");
    }
}

#[test]
fn boolean_flags_are_dropped_without_swallowing() {
    let a = args(&["--yes", "./a.bin"]);
    assert_eq!(positional_args(&a), args(&["./a.bin"]));
}

#[test]
fn keeps_device_name_and_files() {
    let a = args(&["客厅的 Mac", "./a.bin", "./b.bin"]);
    assert_eq!(positional_args(&a), args(&["客厅的 Mac", "./a.bin", "./b.bin"]));
}

#[test]
fn flag_at_end_without_value_is_dropped() {
    let a = args(&["--yes", "./a.bin", "--timeout"]);
    assert_eq!(positional_args(&a), args(&["./a.bin"]));
    assert_eq!(flag_secs(&a, "--timeout"), None);
}

#[test]
fn reads_flag_values() {
    let a = args(&["--timeout", "7", "--port", "0", "--yes"]);
    assert_eq!(flag_secs(&a, "--timeout"), Some(std::time::Duration::from_secs(7)));
    assert_eq!(flag_value(&a, "--timeout").as_deref(), Some("7"));
    assert_eq!(flag_u16(&a, "--port").unwrap(), Some(0));
    assert!(has_flag(&a, "--yes"));
    assert!(!has_flag(&a, "--no"));
}

#[test]
fn rejects_non_numeric_port() {
    let a = args(&["--port", "abc"]);
    let error = flag_u16(&a, "--port").unwrap_err();
    assert!(matches!(error, glowlink_core::GlowError::Usage(_)), "应为用法错误");
}
