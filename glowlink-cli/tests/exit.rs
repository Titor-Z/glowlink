//! `exit.rs` 的测试：退出码是对外契约，脚本会依赖它。
//!
//! 这里既测映射本身，也**真跑二进制**验证几个常见失败场景的退出码——
//! 映射改对了但某条错误路径根本没走到，只有真跑才看得出来。

use std::io::ErrorKind;

use glowlink_cli::exit::{self, code_of};
use glowlink_ipc::codec::{error_from_line, error_reply};
use glowlink_core::GlowError;

#[test]
fn each_error_kind_maps_to_its_exit_code() {
    assert_eq!(code_of(&GlowError::Usage("x".into())), exit::USAGE);
    // 配置写错属于用法错误：脚本该改配置，而不是重试
    assert_eq!(code_of(&GlowError::Config("x".into())), exit::USAGE);
    assert_eq!(code_of(&GlowError::Crypto("x".into())), exit::TRUST);
    assert_eq!(code_of(&GlowError::Rejected("x".into())), exit::REJECTED);
    // 「未找到设备」是对端不在线，脚本可以稍后重试
    assert_eq!(code_of(&GlowError::NotFound("x".into())), exit::NETWORK);
    // 本地找不到条目与网络无关
    assert_eq!(code_of(&GlowError::Missing("x".into())), exit::FAILURE);
    assert_eq!(code_of(&GlowError::Protocol("x".into())), exit::FAILURE);
}

#[test]
fn io_errors_are_split_into_network_and_everything_else() {
    for kind in [
        ErrorKind::ConnectionRefused,
        ErrorKind::ConnectionReset,
        ErrorKind::TimedOut,
        ErrorKind::BrokenPipe,
    ] {
        let error = GlowError::Io(std::io::Error::from(kind));
        assert_eq!(code_of(&error), exit::NETWORK, "{kind:?} 应当算网络错误");
    }
    // 文件类错误不能算网络，否则脚本会去重试一个永远不会成功的操作
    for kind in [ErrorKind::NotFound, ErrorKind::PermissionDenied, ErrorKind::AlreadyExists] {
        let error = GlowError::Io(std::io::Error::from(kind));
        assert_eq!(code_of(&error), exit::FAILURE, "{kind:?} 不该算网络错误");
    }
}

/// 经 IPC 往返后退出码必须不变：否则同一条命令会随「有没有开守护进程」而给出不同退出码。
#[test]
fn exit_code_survives_the_ipc_round_trip() {
    for error in [
        GlowError::Usage("参数写错".into()),
        GlowError::Config("配置项非法".into()),
        GlowError::Crypto("校验码不符".into()),
        GlowError::Rejected("用户拒绝接收".into()),
        GlowError::NotFound("甲机".into()),
        GlowError::Missing("信任列表里没有".into()),
        GlowError::Protocol("坏魔数".into()),
        GlowError::Remote("对方说不".into()),
        GlowError::Io(std::io::Error::from(ErrorKind::ConnectionRefused)),
    ] {
        let line = error_reply(&error);
        assert!(line.starts_with("ERR "), "响应行要以 ERR 开头：{line}");
        let restored = error_from_line(&line);
        assert_eq!(
            code_of(&restored),
            code_of(&error),
            "往返后退出码变了：{line}"
        );
        // 正文也要保留，用户看到的信息不能丢
        let original = error.to_string();
        let text = restored.to_string();
        assert!(
            original.contains(text.trim()) || text.contains(original.trim()),
            "正文丢失：原「{original}」还原成「{text}」"
        );
    }
}

/// 老版本守护进程（ERR 后直接跟正文、没有种类）也要能配合。
#[test]
fn error_line_without_a_kind_still_works() {
    let restored = error_from_line("ERR 未找到设备「甲机」");
    assert!(matches!(restored, GlowError::Remote(_)), "认不出种类就按远端错误处理");
    assert_eq!(restored.to_string(), "未找到设备「甲机」");
}

fn run(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_glowlink"))
        .args(args)
        .output()
        .expect("应当能启动 glowlink")
}

#[test]
fn success_and_usage_errors_have_the_documented_codes() {
    assert_eq!(run(&["--version"]).status.code(), Some(exit::OK));
    // 缺参数 / 未知命令 / 未知配置项都是用法错误
    assert_eq!(run(&["send"]).status.code(), Some(exit::USAGE));
    assert_eq!(run(&["nope"]).status.code(), Some(exit::USAGE));
    assert_eq!(run(&["help", "nope"]).status.code(), Some(exit::USAGE));
}

/// 连不上对端必须是 3（网络），这样脚本才敢稍后重试。
#[test]
fn refused_connection_reports_a_network_failure() {
    let dir = std::env::temp_dir().join(format!("glowlink-exit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.bin");
    std::fs::write(&file, b"payload").unwrap();

    // 借一个空闲端口后立刻释放：那一刻它几乎肯定无人监听
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let output = run(&[
        "send",
        "--peer",
        &format!("127.0.0.1:{port}"),
        &file.display().to_string(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(exit::NETWORK),
        "连不上应当是网络错误：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_file(&file);
}

/// 本地「找不到」不是网络错误：退出码要能区分开。
#[test]
fn missing_local_entry_is_not_a_network_failure() {
    let output = run(&["trust", "forget", "绝对不存在的设备名"]);
    assert_eq!(
        output.status.code(),
        Some(exit::FAILURE),
        "stderr：{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
