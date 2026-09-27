//! 守护进程的平台层：分离进程、终止、以及「守护进程是否在跑」的判定。
//!
//! 状态文件（`state.rs`）与 IPC 协议（`ipc.rs`）刻意分开，这里只放与
//! 具体操作系统打交道的东西；Step 3 的加密要插在 IPC 身份验证处，
//! 放在 `ipc.rs` 里即可，不必动本文件。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::ipc::IpcClient;
use crate::state::{open_log, read_state, State};
use glowlink_core::model::error::Result;

/// 内部子命令名：由 `glowlink on` 拉起的守护进程本体，不面向用户。
pub const DAEMON_COMMAND: &str = "__daemon";

/// 让当前进程脱离父进程的会话（Unix 用 `setsid`；Windows 靠 spawn 时的标志）。
///
/// 直接声明 libc 符号，避免为一个函数引入依赖——与 `main.rs` 里恢复 SIGPIPE 的做法一致。
#[cfg(unix)]
pub fn detach_session() {
    unsafe extern "C" {
        fn setsid() -> i32;
    }
    // SAFETY: 只在守护进程启动、创建线程之前调用一次；失败（已是会话首进程）也不影响运行。
    unsafe {
        setsid();
    }
}

#[cfg(not(unix))]
pub fn detach_session() {}

/// 以分离方式拉起守护进程：独立会话/进程组，stdin 关闭，输出进日志。
pub fn spawn_detached(exe: &Path, args: &[String]) -> Result<Child> {
    let log = open_log()?;
    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP：不继承控制台，父进程退出后不自杀
        const FLAGS: u32 = 0x0000_0008 | 0x0000_0200;
        command.creation_flags(FLAGS);
    }

    Ok(command.spawn()?)
}

/// 强制终止指定 PID（IPC 的 `STOP` 已足够优雅，这里只作为兜底）。
pub fn terminate(pid: u32) {
    #[cfg(unix)]
    let mut command = {
        let mut c = Command::new("kill");
        c.arg(pid.to_string());
        c
    };
    #[cfg(windows)]
    let mut command = {
        let mut c = Command::new("taskkill");
        c.args(["/PID", &pid.to_string(), "/F"]);
        c
    };
    let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// 当前可执行文件路径（用于拉起守护进程本体）。
pub fn current_exe() -> Result<PathBuf> {
    Ok(std::env::current_exe()?)
}

/// 读取状态并确认守护进程真的在响应；不在则返回 `None`。
///
/// 判定标准是「能连上并 PING 通」而不是「文件存在」：
/// 崩溃残留的状态文件不该让客户端连到不存在的端口。
pub fn running_state() -> Option<State> {
    let state = read_state().ok().flatten()?;
    let mut client = IpcClient::connect(&state).ok()?;
    client.call("PING").ok()?;
    Some(state)
}
