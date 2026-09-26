//! 守护进程支撑：状态文件、本地 IPC、待确认接收槽、平台相关的分离与终止。
//!
//! 设计要点：
//! - 状态文件 `~/.glowlink/state.json` 是唯一的“入口”：CLI 先读它拿到 IPC 端口，
//!   再走回环长连接控制守护进程；端口顺延过也能自动找到实际端口，无需猜。
//! - IPC 用面向行的文本协议而不是 JSON：零依赖、可手敲、可读日志；
//!   每条响应都以 `END` 收尾，单行与多行响应在客户端走同一条读取路径。
//! - 守护进程是分离进程（没有终端），所以「收到文件要人工确认」不能靠 stdin：
//!   改为登记待确认请求并阻塞等待，由 `glowlink accept/reject` 经 IPC 下决定。

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use glowlink_core::model::error::{GlowError, Result};
use glowlink_core::Device;

/// 本地 IPC 首选端口；被占用时在 [`IPC_PORT_SPAN`] 内顺延。
///
/// 与发现端口（9267）无关：IPC 只监听回环，不参与局域网发现，
/// 因此单独占一个号段，避免和发现/传输端口互相干扰。
pub const IPC_PORT: u16 = 10_000;

/// IPC 端口顺延范围。
pub const IPC_PORT_SPAN: u16 = 50;

/// 连接守护进程的等待时长：本机回环，超过这个时间说明它已卡死或正在退出。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// 读一行响应的等待时长。
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// 状态目录名（位于用户主目录下）。
const STATE_DIR: &str = ".glowlink";

/// 内部子命令名：由 `glowlink on` 拉起的守护进程本体，不面向用户。
pub const DAEMON_COMMAND: &str = "__daemon";

// ---------------------------------------------------------------- 路径

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// 状态目录 `~/.glowlink`。
pub fn state_dir() -> PathBuf {
    match home_dir() {
        Some(home) => home.join(STATE_DIR),
        // 拿不到主目录就退到当前目录，至少不 panic
        None => PathBuf::from(STATE_DIR),
    }
}

/// 状态文件 `~/.glowlink/state.json`。
pub fn state_path() -> PathBuf {
    state_dir().join("state.json")
}

/// 守护进程日志 `~/.glowlink/daemon.log`。
pub fn log_path() -> PathBuf {
    state_dir().join("daemon.log")
}

/// 追加一行守护进程日志；写失败时静默——日志本身不该拖垮主流程。
pub fn append_log(line: &str) {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "[{stamp}] {line}");
    }
}

/// 打开守护进程日志用于 `--daemon` 子进程的 stdout/stderr 重定向。
pub fn open_log() -> Result<File> {
    let path = log_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(OpenOptions::new().create(true).append(true).open(path)?)
}

// ---------------------------------------------------------------- 状态文件

/// 守护进程写入的状态记录。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub version: String,
    pub pid: u32,
    pub ipc_port: u16,
    pub transfer_port: u16,
    pub discovery_port: u16,
    pub receive_dir: PathBuf,
    pub started_at: u64,
}

impl State {
    pub fn to_json(&self) -> String {
        format!(
            "{{\n  \"version\": \"{}\",\n  \"pid\": {},\n  \"ipc_port\": {},\n  \
             \"transfer_port\": {},\n  \"discovery_port\": {},\n  \"receive_dir\": \"{}\",\n  \
             \"started_at\": {}\n}}\n",
            escape(&self.version),
            self.pid,
            self.ipc_port,
            self.transfer_port,
            self.discovery_port,
            escape(&self.receive_dir.to_string_lossy()),
            self.started_at
        )
    }

    /// 解析自己写出的状态文件；缺字段或字段类型不对都算损坏。
    pub fn parse(text: &str) -> Result<Self> {
        let bad = |field: &str| GlowError::Protocol(format!("状态文件缺少字段「{field}」"));
        Ok(State {
            version: json_string(text, "version").ok_or_else(|| bad("version"))?,
            pid: json_u64(text, "pid").ok_or_else(|| bad("pid"))? as u32,
            ipc_port: json_u64(text, "ipc_port").ok_or_else(|| bad("ipc_port"))? as u16,
            transfer_port: json_u64(text, "transfer_port")
                .ok_or_else(|| bad("transfer_port"))? as u16,
            discovery_port: json_u64(text, "discovery_port")
                .ok_or_else(|| bad("discovery_port"))? as u16,
            receive_dir: PathBuf::from(json_string(text, "receive_dir").ok_or_else(|| bad("receive_dir"))?),
            started_at: json_u64(text, "started_at").unwrap_or(0),
        })
    }
}

/// 读取状态文件；不存在返回 `None`。
pub fn read_state() -> Result<Option<State>> {
    let path = state_path();
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)?;
    Ok(Some(State::parse(&text)?))
}

/// 原子写入状态文件：先写临时文件再改名，避免客户端读到半截内容。
pub fn write_state(state: &State) -> Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let tmp = dir.join("state.json.tmp");
    fs::write(&tmp, state.to_json())?;
    fs::rename(&tmp, state_path())?;
    Ok(())
}

/// 删除状态文件；不存在也算成功。
pub fn remove_state() -> Result<()> {
    let path = state_path();
    if path.is_file() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// 读取状态并确认守护进程真的在响应；不在则返回 `None`。
///
/// 状态文件可能是崩溃残留，所以判定标准是「能连上并 PING 通」而不是文件存在。
pub fn running_state() -> Option<State> {
    let state = read_state().ok().flatten()?;
    let mut client = IpcClient::connect(&state).ok()?;
    client.call("PING").ok()?;
    Some(state)
}

// ---------------------------------------------------------------- JSON 小工具

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// 定位 `"key"` 之后冒号右侧的原文。
fn value_after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let start = text.find(&needle)? + needle.len();
    let rest = text[start..].trim_start();
    Some(rest.strip_prefix(':')?.trim_start())
}

fn json_string(text: &str, key: &str) -> Option<String> {
    let mut chars = value_after(text, key)?.strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    let code = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(code)?);
                }
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

fn json_u64(text: &str, key: &str) -> Option<u64> {
    let rest = value_after(text, key)?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

// ---------------------------------------------------------------- 待确认接收槽

/// 一个待确认的接收请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingOffer {
    pub peer: String,
    pub name: String,
    pub size: u64,
}

/// 待确认接收槽：同一时刻只允许一个请求。
///
/// 守护进程只有一个接收线程，所以不需要队列；用单槽 + 条件变量，
/// 让接收线程阻塞等待，而 IPC 线程负责落决定。
pub struct PendingSlot {
    inner: Mutex<PendingInner>,
    ready: Condvar,
}

#[derive(Default)]
struct PendingInner {
    offer: Option<PendingOffer>,
    decision: Option<bool>,
}

impl Default for PendingSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingSlot {
    pub fn new() -> Self {
        PendingSlot {
            inner: Mutex::new(PendingInner::default()),
            ready: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PendingInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 登记请求并阻塞等待决定；超时按拒绝处理。
    pub fn ask(&self, offer: PendingOffer, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut inner = self.lock();
        inner.offer = Some(offer);
        inner.decision = None;
        let mut decision = None;
        while decision.is_none() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let (guard, _) = self
                .ready
                .wait_timeout(inner, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            inner = guard;
            decision = inner.decision;
        }
        inner.offer = None;
        inner.decision = None;
        decision.unwrap_or(false)
    }

    /// 当前待确认的请求（供 `PENDING` 查询）。
    pub fn current(&self) -> Option<PendingOffer> {
        self.lock().offer.clone()
    }

    /// 落决定；没有待确认请求时报错，避免误判成“已处理”。
    pub fn decide(&self, accept: bool) -> Result<()> {
        let mut inner = self.lock();
        if inner.offer.is_none() {
            return Err(GlowError::Usage("当前没有待确认的接收请求".into()));
        }
        inner.decision = Some(accept);
        self.ready.notify_all();
        Ok(())
    }
}

// ---------------------------------------------------------------- IPC 协议

/// IPC 指令处理所依赖的守护进程能力；抽成 trait 便于用测试替身覆盖协议分支。
pub trait IpcHost: Send + Sync {
    /// 状态快照（含实际端口与接收目录）。
    fn status(&self) -> State;

    /// 当前已知设备。
    fn devices(&self) -> Vec<Device>;

    /// 按名称或 ID 查设备。
    fn find(&self, key: &str) -> Option<Device>;

    /// 待确认的接收请求。
    fn pending(&self) -> Option<PendingOffer>;

    /// 落定待确认请求。
    fn decide(&self, accept: bool) -> Result<()>;

    /// 请求守护进程退出。
    fn request_stop(&self);
}

/// 处理一行 IPC 指令，返回要写回的行；`true` 表示连接可以继续复用。
///
/// 所有响应都以 [`END`] 收尾：客户端不必区分单行与多行响应。
pub fn handle_line(host: &dyn IpcHost, line: &str) -> (Vec<String>, bool) {
    let line = line.trim();
    let (cmd, arg) = match line.split_once(' ') {
        Some((cmd, arg)) => (cmd, arg.trim()),
        None => (line, ""),
    };
    match cmd {
        "PING" => {
            let state = host.status();
            (
                vec![format!(
                    "OK {} {} {} {}",
                    state.version, state.pid, state.transfer_port, state.discovery_port
                )],
                true,
            )
        }
        "STATUS" => {
            let state = host.status();
            let pending = if host.pending().is_some() { 1 } else { 0 };
            (
                vec![format!(
                    "OK {} {} {}",
                    host.devices().len(),
                    pending,
                    state.receive_dir.display()
                )],
                true,
            )
        }
        "DEVICES" => {
            let devices = host.devices();
            let mut lines = vec![format!("OK {}", devices.len())];
            for device in &devices {
                lines.push(format!(
                    "{}\t{}\t{}\t{}",
                    device.name,
                    device.id.as_str(),
                    device.addr,
                    device.last_seen.elapsed().as_millis()
                ));
            }
            (lines, true)
        }
        "RESOLVE" => match host.find(arg) {
            Some(device) => (
                vec![format!("OK {}\t{}\t{}", device.name, device.id.as_str(), device.addr)],
                true,
            ),
            None => (vec![format!("ERR 未找到设备「{arg}」")], true),
        },
        "PENDING" => match host.pending() {
            Some(offer) => (
                vec![
                    "OK 1".to_string(),
                    format!("{}\t{}\t{}", offer.peer, offer.name, offer.size),
                ],
                true,
            ),
            None => (vec!["OK 0".to_string()], true),
        },
        "ACCEPT" | "REJECT" => match host.decide(cmd == "ACCEPT") {
            Ok(()) => (
                vec![format!("OK {}", if cmd == "ACCEPT" { "已接受" } else { "已拒绝" })],
                true,
            ),
            Err(e) => (vec![format!("ERR {e}")], true),
        },
        "STOP" => {
            host.request_stop();
            (vec!["OK 正在停止".to_string()], false)
        }
        "" => (vec!["ERR 空指令".to_string()], true),
        other => (vec![format!("ERR 未知指令「{other}」")], true),
    }
}

/// 把响应行写进流，并补上结束标记。
fn write_reply(stream: &mut TcpStream, lines: &[String]) -> std::io::Result<()> {
    for line in lines {
        writeln!(stream, "{line}")?;
    }
    writeln!(stream, "END")?;
    stream.flush()
}

/// 服务一次 IPC 连接，直到对端关闭或发出 `STOP`。
pub fn serve_connection(host: &dyn IpcHost, stream: TcpStream) {
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let (reply, keep_open) = handle_line(host, &line);
        if write_reply(&mut writer, &reply).is_err() {
            break;
        }
        if !keep_open {
            break;
        }
    }
}

/// 绑定回环上的 IPC 监听器；首选端口被占用时顺延。
pub fn bind_ipc(preferred: u16) -> Result<(TcpListener, u16)> {
    let mut last_error = None;
    for offset in 0..IPC_PORT_SPAN {
        let port = preferred.saturating_add(offset);
        if port == 0 {
            break;
        }
        match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))) {
            Ok(listener) => return Ok((listener, port)),
            Err(e) => last_error = Some(e),
        }
    }
    Err(match last_error {
        Some(e) => GlowError::Io(e),
        None => GlowError::Usage("没有可用的 IPC 端口".into()),
    })
}

// ---------------------------------------------------------------- IPC 客户端

/// 读取一行；超时或对端关闭都算失败。
fn read_line(reader: &mut BufReader<TcpStream>) -> Result<String> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => Err(GlowError::Protocol("守护进程已断开连接".into())),
        Ok(_) => Ok(line.trim_end().to_string()),
        Err(e) => Err(GlowError::Io(e)),
    }
}

pub struct IpcClient {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
}

impl IpcClient {
    pub fn connect(state: &State) -> Result<Self> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, state.ipc_port));
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.set_write_timeout(Some(READ_TIMEOUT))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(IpcClient { writer: stream, reader })
    }

    /// 发一条指令并读取完整响应（`OK …` / `ERR …` 首行 + 数据行，直到 `END`）。
    pub fn call(&mut self, command: &str) -> Result<Vec<String>> {
        writeln!(self.writer, "{command}")?;
        self.writer.flush()?;
        let mut lines = Vec::new();
        loop {
            let line = read_line(&mut self.reader)?;
            if line == "END" {
                break;
            }
            lines.push(line);
        }
        match lines.first() {
            Some(first) if first.starts_with("ERR ") => {
                Err(GlowError::Protocol(first[4..].to_string()))
            }
            Some(_) => Ok(lines),
            None => Err(GlowError::Protocol("守护进程返回了空响应".into())),
        }
    }

    /// 只取首行（单行指令的常用形态）。
    pub fn call_first(&mut self, command: &str) -> Result<String> {
        Ok(self.call(command)?.remove(0))
    }
}

/// 取首行 `OK ` 之后的正文。
pub fn ok_payload(line: &str) -> &str {
    line.strip_prefix("OK ").unwrap_or(line).trim()
}

/// 把 `RESOLVE` 的响应解析成 `(名称, ID, 地址)`。
pub fn parse_resolve(line: &str) -> Result<(String, String, SocketAddr)> {
    let payload = ok_payload(line);
    let mut parts = payload.split('\t');
    let (name, id, addr) = (parts.next(), parts.next(), parts.next());
    match (name, id, addr.and_then(|a| a.parse::<SocketAddr>().ok())) {
        (Some(name), Some(id), Some(addr)) => Ok((name.to_string(), id.to_string(), addr)),
        _ => Err(GlowError::Protocol(format!("无法解析守护进程应答：{payload}"))),
    }
}

/// 把 `DEVICES` 的数据行解析成设备记录。
pub fn parse_device_line(line: &str) -> Option<(String, String, SocketAddr, u128)> {
    let mut parts = line.split('\t');
    let (name, id, addr, age) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    Some((name.to_string(), id.to_string(), addr.parse().ok()?, age.parse().ok()?))
}

// ---------------------------------------------------------------- 平台相关

/// 让当前进程脱离父进程的会话（Unix 用 `setsid`，Windows 靠 spawn 时的标志）。
///
/// 直接声明 libc 符号，避免为一个函数引入依赖——与 `main.rs` 里恢复 SIGPIPE 的做法一致。
#[cfg(unix)]
pub fn detach_session() {
    unsafe extern "C" {
        fn setsid() -> i32;
    }
    // SAFETY: 只在守护进程启动、创建线程之前调用一次，失败（已有会话首进程）也不影响运行。
    unsafe {
        setsid();
    }
}

#[cfg(not(unix))]
pub fn detach_session() {}

/// 以分离方式拉起守护进程：new session / 独立进程组，stdin 关闭，输出进日志。
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
