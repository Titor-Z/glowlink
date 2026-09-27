//! 守护进程的平台层与生命周期：分离进程、终止、「是否在跑」的判定、拉起与停止。
//!
//! 放在 `glowlink-ipc` 而不是 CLI 里：**服务生命周期不是 CLI 的表现层**。
//! 桌面版要能「打开软件即服务在跑」、还要能用标题栏开关停掉它，
//! 而桌面版不许依赖 cli（那是另一个进程的表现层），所以这层必须下沉到这里。
//!
//! 代价：拉起守护进程需要一个可执行文件。桌面版与 CLI 一起构建/分发（自用工具），
//! 所以按「先看同目录、再查 PATH」找它；找不到就如实报错，不去猜路径。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use glowlink_core::model::error::{GlowError, Result};

use crate::client::IpcClient;
use crate::state::{open_log, read_state, remove_state, State};

/// 内部子命令名：由宿主（`glowlink on` / 桌面版）拉起的守护进程本体，不面向用户。
pub const DAEMON_COMMAND: &str = "__daemon";

/// 守护进程可执行文件的候选名字（Windows 带 `.exe`）。
#[cfg(windows)]
const HOST_BINARY: &str = "glowlink.exe";
#[cfg(not(windows))]
const HOST_BINARY: &str = "glowlink";

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

/// 当前可执行文件路径（CLI 自己拉起守护进程本体时用）。
pub fn current_exe() -> Result<PathBuf> {
    Ok(std::env::current_exe()?)
}

/// 找守护进程可执行文件：**先看自己旁边**（两个二进制一起构建/拷贝时就挨着），再查 PATH。
///
/// 不猜安装路径、不写死 `/usr/local/bin`——找不到就让调用方如实报错。
pub fn find_host_binary() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside = dir.join(HOST_BINARY);
            if beside.is_file() {
                return Some(beside);
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(HOST_BINARY))
        .find(|candidate| candidate.is_file())
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

/// 确保守护进程在跑：没跑就按 `args` 拉起，并等它就绪。
///
/// 已经跑着就直接返回它（**不**按 args 重新拉起：一个机器上只应有一个发现端口的主人）。
pub fn ensure_running(args: &[String], timeout: Duration) -> Result<State> {
    if let Some(state) = running_state() {
        return Ok(state);
    }
    // 残留状态文件先清掉，免得客户端读到不存在的 IPC 端口
    remove_state()?;
    let exe = find_host_binary().ok_or_else(|| {
        GlowError::Missing(format!("找不到 {HOST_BINARY}（拉起守护进程需要它，请与本程序放在同一目录）"))
    })?;
    let child = spawn_detached(&exe, args)?;

    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(state) = running_state() {
            return Ok(state);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // 沿用 `Usage` 种类：`glowlink on` 起不来一直是退出码 2，改它等于改对外契约
    Err(GlowError::Usage(format!(
        "守护进程未能就绪（PID {}）\n\n日志：{}",
        child.id(),
        crate::state::log_path().display()
    )))
}

/// 停掉守护进程：先走 IPC `STOP`（它会自己退出并清掉状态文件），超时则按 PID 兜底强杀。
///
/// 返回被停掉的状态（调用方拿它去打印）。没在跑时返回 `Ok(None)`——**停止一个没跑的服务
/// 不是错误**，这一点与 CLI `off` 的既有行为一致。
pub fn stop(timeout: Duration) -> Result<Option<State>> {
    let Some(state) = read_state()? else {
        return Ok(None);
    };

    let stopped_gracefully = match IpcClient::connect(&state) {
        Ok(mut client) => client.call("STOP").is_ok(),
        Err(_) => false,
    };
    if !stopped_gracefully {
        // 连不上（进程卡死或状态文件过期）：直接按 PID 兜底
        terminate(state.pid);
    }

    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline && running_state().is_some() {
        std::thread::sleep(Duration::from_millis(100));
    }
    if running_state().is_some() {
        terminate(state.pid);
        std::thread::sleep(Duration::from_millis(200));
    }
    remove_state()?;
    Ok(Some(state))
}
