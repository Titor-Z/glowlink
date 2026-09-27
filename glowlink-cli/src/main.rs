//! 萤光 GlowLink CLI：Phase 1 概念验证。
//!
//! 这里是唯一做装配（依赖注入）的地方：配置来自配置文件 + 命令行覆盖，
//! 控制器由核心库提供，通道实现由工厂提供，视图由本 crate 的库部分提供。

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use glowlink_cli::args::{flag_secs, flag_u16, flag_value, has_flag, positional_args, truncate};
use glowlink_cli::config;
use glowlink_cli::daemon;
use glowlink_cli::ipc::{self, IpcHost};
use glowlink_cli::keys::{load_or_create_identity, TrustFile};
use glowlink_cli::pending::{PendingOffer, PendingQueue};
use glowlink_cli::state::{self, State};
use glowlink_cli::view::cli::human_size;
use glowlink_cli::DaemonView;
use glowlink_cli::CliView;
use glowlink_core::controller::transfer::TransferListener;
use glowlink_core::model::probe::{compare_versions, identify, PortOwner, VersionVerdict};
use glowlink_core::patterns::secure::{session_slot, SecureTransport, SessionSlot, TrustList};
use glowlink_core::patterns::factory::{
    DiscoveryTransportFactory, NativeTransportFactory, TransferTransportFactory,
};
use glowlink_core::{
    Config, DeviceIdentity, DiscoveryController, GlowError, Result, TransferController,
};

fn main() {
    reset_sigpipe();
    if let Err(e) = run() {
        eprintln!("错误：{e}");
        std::process::exit(1);
    }
}

/// 恢复 SIGPIPE 的默认行为，让 `glowlink list | head` 这类下游提前关管道时静默退出。
///
/// Rust 启动时会把 SIGPIPE 设为忽略，于是写入已关闭的管道会返回 EPIPE，
/// 而 `println!` 遇到写入失败会直接 panic（不经过 `Result`），"broken pipe" 就会刷一屏回溯。
#[cfg(unix)]
fn reset_sigpipe() {
    // SAFETY: 在 main 开头、任何线程创建之前调用，只是把信号处理恢复为系统默认。
    unsafe {
        libc_signal(13, 0); // SIGPIPE = 13, SIG_DFL = 0
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

#[cfg(unix)]
unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}

/// 绑定到 libc 的 `signal`，避免为一个函数引入 libc 依赖。
#[cfg(unix)]
unsafe fn libc_signal(signum: i32, handler: usize) {
    // 平台间 `signal` 的符号名一致，直接声明确保链接到 libc
    unsafe { signal(signum, handler) };
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = config::load()?;
    match args.first().map(String::as_str) {
        Some("list") => cmd_list(&config, &args[1..]),
        Some("serve") => cmd_serve(&config, &args[1..]),
        // 守护进程本体：只由 `glowlink on` 拉起，不面向用户直接调用。
        Some(daemon::DAEMON_COMMAND) => cmd_daemon(&config, &args[1..]),
        Some("on") => cmd_on(&config, &args[1..]),
        Some("off") => cmd_off(&config, &args[1..]),
        Some("accept") => cmd_decide(true, &args[1..]),
        Some("reject") => cmd_decide(false, &args[1..]),
        Some("send") => cmd_send(&config, &args[1..]),
        Some("config") => cmd_config(&config),
        Some("help") | None => {
            print_help();
            Ok(())
        }
        Some(other) => Err(GlowError::Usage(format!("未知命令：{other}"))),
    }
}

fn cmd_config(config: &Config) -> Result<()> {
    println!("生效配置（按 {} → 用户配置 顺序叠加）", config::PROJECT_CONFIG);
    for path in config::config_paths() {
        let mark = if path.is_file() { "已加载" } else { "不存在" };
        println!("  {mark}  {}", path.display());
    }
    println!("{config:#?}");
    Ok(())
}

fn cmd_list(config: &Config, args: &[String]) -> Result<()> {
    let timeout = search_timeout(config, args);
    let watch = has_flag(args, "--watch");
    let discovery_port = discovery_port(config, args)?;

    // 守护进程在跑就走 IPC：既快（设备表是累积的，不用等一轮扫描），
    // 也避免和守护进程抢发现端口。
    if let Some(state) = daemon::running_state() {
        return list_via_daemon(&state, timeout, watch);
    }

    let factory = NativeTransportFactory;
    let identity = DeviceIdentity::global().clone();
    let transport = factory.create(discovery_port)?;
    let mut discovery = DiscoveryController::new(
        transport,
        identity.clone(),
        0,
        device_timeout(config),
        query_interval(config),
    );

    println!("本机：{}（{}）", identity.name, truncate(&identity.id, 8));
    if discovery.degraded() {
        println!("{}", degraded_notice(discovery_port));
    }

    if !watch {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            discovery.tick()?;
        }
        print_devices(&discovery.devices(), timeout);
        return Ok(());
    }

    // --watch：持续刷新，直到 Ctrl-C。
    if !io::stdout().is_terminal() {
        return Err(GlowError::Usage(
            "--watch 需要终端环境；重定向输出请改用普通的 list".into(),
        ));
    }
    println!("持续监听中，Ctrl-C 退出。\n");
    loop {
        discovery.tick()?;
        print_devices_live(&discovery.devices());
    }
}

/// 经守护进程查询：设备表由守护进程长期累积，比现场扫一轮更快也更全。
fn list_via_daemon(state: &State, timeout: Duration, watch: bool) -> Result<()> {
    let identity = DeviceIdentity::global().clone();
    println!("本机：{}（{}）", identity.name, truncate(&identity.id, 8));
    println!(
        "[信息] 结果来自守护进程（PID {}，IPC 端口 {}），接收目录 {}。",
        state.pid,
        state.ipc_port,
        state.receive_dir.display()
    );

    let mut client = ipc::IpcClient::connect(state)?;
    if !watch {
        let devices = ipc_devices(&client.call("DEVICES")?);
        print_devices(&devices, timeout);
        print_pending(&mut client)?;
        return Ok(());
    }

    if !io::stdout().is_terminal() {
        return Err(GlowError::Usage(
            "--watch 需要终端环境；重定向输出请改用普通的 list".into(),
        ));
    }
    println!("持续监听中，Ctrl-C 退出。\n");
    loop {
        let devices = ipc_devices(&client.call("DEVICES")?);
        print_devices_live(&devices);
    }
}

/// 客户端侧判断某指纹是否已信任（仅用于显示提示，读失败就当未信任）。
fn daemon_trusts(fingerprint: &str) -> bool {
    TrustFile::open_default()
        .map(|trust| trust.is_trusted(fingerprint))
        .unwrap_or(false)
}

/// 把 `DEVICES` 响应还原成设备列表（首行是 `OK <数量>`）。
fn ipc_devices(lines: &[String]) -> Vec<glowlink_core::Device> {
    lines
        .iter()
        .skip(1)
        .filter_map(|line| ipc::parse_device_line(line))
        .map(|(name, id, addr, age_ms)| glowlink_core::Device {
            id: glowlink_core::DeviceId::new(id),
            name,
            addr,
            // 用“现在减去已存活时长”还原最近出现时刻，复用同一套表格渲染
            last_seen: Instant::now() - Duration::from_millis(age_ms as u64),
        })
        .collect()
}

/// 有待确认的接收请求时逐条列出（守护进程不会自己弹窗）。
fn print_pending(client: &mut ipc::IpcClient) -> Result<()> {
    let lines = client.call("PENDING")?;
    let count = lines
        .first()
        .map(|line| ipc::ok_payload(line).parse::<usize>().unwrap_or(0))
        .unwrap_or(0);
    if count == 0 {
        return Ok(());
    }
    println!();
    for line in lines.iter().skip(1) {
        let Some((token, peer, name, size, fingerprint, code)) = ipc::parse_pending_line(line)
        else {
            continue;
        };
        let trust_hint = if fingerprint.is_empty() {
            String::new()
        } else if daemon_trusts(&fingerprint) {
            format!("设备指纹 {fingerprint}（已信任）")
        } else {
            format!("设备指纹 {fingerprint}  校验码 **{code}**（请与发送方屏幕核对）")
        };
        println!(
            "[等待确认 #{token}] {peer} 想发来「{name}」（{}）",
            human_size(size)
        );
        if !trust_hint.is_empty() {
            println!("             {trust_hint}");
        }
        println!("             确认：glowlink accept {token} / glowlink reject {token}");
    }
    Ok(())
}

/// 降级提示：用 PROBE/PONG 握手区分四种情况。
///
/// 只靠“端口被占”无法给出有用信息，而这几种情况的处理方式完全不同：
/// 自家节点属于正常的多实例场景，别人占用需要用户处理，
/// 残留绑定则是 macOS 内核问题（重启即好）。
fn degraded_notice(port: u16) -> String {
    match identify(port) {
        Ok(PortOwner::Ours(p)) => match compare_versions(env!("CARGO_PKG_VERSION"), &p.version) {
            VersionVerdict::RunningIsNewer => format!(
                "[信息] 发现端口 {port} 上已有更新的节点在运行（v{}，PID {}），本进程以仅查询模式工作。",
                p.version, p.pid
            ),
            VersionVerdict::RunningIsOlder => format!(
                "[信息] 发现端口 {port} 上运行的是旧版本（v{}，PID {}），本进程以仅查询模式工作。",
                p.version, p.pid
            ),
            VersionVerdict::Same => format!(
                "[信息] 端口 {port} 上已有本机节点在运行（v{}，PID {}），本进程以仅查询模式工作（收不到广播，但仍能被对方发现）。",
                p.version, p.pid
            ),
        },
        // 真收到了非协议应答：确实是别的程序
        Ok(PortOwner::Stranger) => format!(
            "[警告] 端口 {port} 被其他程序占用，建议使用 --discovery-port 切换端口。"
        ),
        // 无人应答但绑不上：内核残留绑定或防火墙，不能说成“被其它程序占用”
        Ok(PortOwner::Stale) => format!(
            "[警告] 端口 {port} 绑定异常（可能是内核残留或防火墙拦截），建议使用 --discovery-port 切换端口。"
        ),
        Ok(PortOwner::Free) => String::new(),
        Err(e) => format!("[警告] 端口 {port} 占用情况探测失败：{e}"),
    }
}

fn print_devices(devices: &[glowlink_core::Device], timeout: Duration) {
    if devices.is_empty() {
        println!("未发现设备（已等待 {} 秒）", timeout.as_secs());
        return;
    }
    println!("{} {}", device_table_header(), self_hint());
    for device in devices {
        println!("{}", device_row(device, false));
    }
}

/// `--watch` 模式：表格原地重画，行数变化时不留下残留。
fn print_devices_live(devices: &[glowlink_core::Device]) {
    let mut lines = vec![device_table_header(), self_hint()];
    if devices.is_empty() {
        lines.push("  （搜索中…）".to_string());
    }
    for device in devices {
        lines.push(device_row(device, true));
    }
    // \x1b[2K 清整行、\x1b[1A 上移一行，逐行覆盖上一次的内容
    let mut out = String::from("\x1b[?25l");
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push_str("\x1b[1A");
        }
        out.push_str("\x1b[2K\r");
        out.push_str(line);
        out.push('\n');
    }
    print!("{out}\x1b[?25h");
    let _ = io::stdout().flush();
}

fn device_table_header() -> String {
    format!("{:<20} {:<10} {:<22} {}", "名称", "ID", "地址", "最后出现")
}

/// 数据窗口固定为 18 列：留 2 列给省略号，避免中文名撞坏表格。
fn device_row(device: &glowlink_core::Device, fresh: bool) -> String {
    let name = truncate(&device.name, 18);
    let age = device.last_seen.elapsed().as_secs();
    let marker = if fresh && age < 2 { "●" } else { " " };
    format!(
        "{marker}{:<19} {:<10} {:<22} {}s",
        name,
        device.short_id(),
        device.addr,
        age
    )
}

/// 自我提醒：设备列表里不会出现自己，避免用户疑惑“怎么少了一台”。
fn self_hint() -> String {
    "（本机不在列表中）".to_string()
}

fn cmd_serve(config: &Config, args: &[String]) -> Result<()> {
    let port = match flag_u16(args, "--port")? {
        Some(port) => port,
        None => config.transfer_port,
    };
    let discovery_port = discovery_port(config, args)?;
    let dir = match flag_value(args, "--dir") {
        Some(dir) => PathBuf::from(dir),
        None => config.receive_dir.clone(),
    };
    let auto_accept = has_flag(args, "--yes");

    let factory = NativeTransportFactory;
    let listener = factory.listen(port)?;
    let port = listener.local_port()?;
    let identity = DeviceIdentity::global().clone();
    // serve 必须跑起来：发现端口被占时顺延，只打一句 [信息]。
    // decide_port 已完成探测与绑定，这里**不能再探测一次**：
    // 新端口此刻正被自己持有，重探只会把自己误判成“绑定异常”。
    let decision = glowlink_core::model::probe::decide_port(discovery_port)?;
    if let Some(notice) = decision.notice() {
        println!("{notice}");
    }
    let transport = factory.from_bound_socket(decision)?;
    let mut discovery = DiscoveryController::new(
        transport,
        identity.clone(),
        port,
        device_timeout(config),
        query_interval(config),
    );

    // 线程里不能再借用 config，先把需要的值取出来
    let chunk_size = config.chunk_size();
    let ring_capacity = config.ring_capacity();
    let identity_key = load_or_create_identity()?;
    let trust = TrustFile::open_default()?;

    println!(
        "萤光节点「{}」已启动：传输端口 {port}，接收目录 {}",
        identity.name,
        dir.display()
    );
    println!(
        "本机设备指纹 {}（加密已启用，首次配对需与对端核对 6 位校验码）",
        identity_key.fingerprint()
    );
    if !auto_accept {
        println!("收到文件时会先询问是否接收（--yes 对已信任设备免确认）。");
    }

    // 接收在独立线程中进行，主线程持续查询与刷新设备列表。
    {
        let listener: Arc<Box<dyn TransferListener>> = Arc::from(listener);
        let dir = dir.clone();
        let trust = Arc::clone(&trust);
        std::thread::spawn(move || loop {
            match listener.accept() {
                Ok((stream, peer)) => {
                    // 每个连接各自握手、各自一份会话信息与视图
                    let trust = Arc::clone(&trust);
                    let identity_key = Arc::clone(&identity_key);
                    let dir = dir.clone();
                    std::thread::spawn(move || {
                        let session: SessionSlot = session_slot();
                        let mut secure = SecureTransport::responder(
                            stream,
                            identity_key,
                            Arc::clone(&trust) as Arc<dyn TrustList>,
                            Arc::clone(&session),
                        );
                        let transfer = TransferController::new(chunk_size, ring_capacity);
                        let view = CliView::with_session(auto_accept, session, trust);
                        let label = peer.ip().to_string();
                        match transfer.receive_file(&mut secure, &dir, &label, &view) {
                            Ok(path) => println!("\n已接收：{}", path.display()),
                            Err(e) => eprintln!("\n接收失败：{e}"),
                        }
                    });
                }
                Err(e) => eprintln!("接受连接失败：{e}"),
            }
        });
    }

    loop {
        discovery.tick()?;
    }
}

fn cmd_send(config: &Config, args: &[String]) -> Result<()> {
    let timeout = search_timeout(config, args);
    let discovery_port = discovery_port(config, args)?;
    let peer = flag_value(args, "--peer");
    let positional = positional_args(args);

    // 直连模式不需要设备名，位置参数全部视为文件。
    let (peer_key, file_args) = if peer.is_some() {
        (None, &positional[..])
    } else {
        if positional.len() < 2 {
            return Err(GlowError::Usage(
                "用法：glowlink send <设备名|设备ID> <文件>… 或 glowlink send --peer ip:端口 <文件>…"
                    .into(),
            ));
        }
        (Some(positional[0].clone()), &positional[1..])
    };
    if file_args.is_empty() {
        return Err(GlowError::Usage("至少需要一个待发送的文件".into()));
    }
    let files: Vec<PathBuf> = file_args.iter().map(PathBuf::from).collect();

    let factory = NativeTransportFactory;
    let direct = peer.is_some();
    let target = match peer {
        // 绕过发现层，直接单播连接，便于本机联调与跨网段排查。
        Some(value) => value
            .parse::<SocketAddr>()
            .map_err(|_| GlowError::Usage(format!("--peer 需要 ip:端口，收到「{value}」")))?,
        None => {
            let peer_key = peer_key.expect("非直连模式必有设备名");
            // 守护进程在跑就直接向它要地址：省一轮扫描，也不和它抢发现端口。
            if let Some(state) = daemon::running_state() {
                let mut client = ipc::IpcClient::connect(&state)?;
                let (name, id, addr) = ipc::parse_resolve(&client.call_first(&format!("RESOLVE {peer_key}"))?)?;
                println!("目标：{}（{}）@ {}", name, truncate(&id, 8), addr);
                addr
            } else {
                let identity = DeviceIdentity::global().clone();
                let transport = factory.create(discovery_port)?;
                let mut discovery = DiscoveryController::new(
                    transport,
                    identity,
                    0,
                    device_timeout(config),
                    query_interval(config),
                );
                println!("正在搜索「{peer_key}」…");
                let deadline = Instant::now() + timeout;
                while Instant::now() < deadline {
                    discovery.tick()?;
                }
                let device = discovery
                    .find(&peer_key)
                    .ok_or_else(|| GlowError::NotFound(peer_key.clone()))?;
                println!("目标：{}（{}）@ {}", device.name, device.short_id(), device.addr);
                device.addr
            }
        }
    };
    if direct {
        println!("直连目标：{target}");
    }

    let transfer = TransferController::new(config.chunk_size(), config.ring_capacity());
    let connect_timeout = Duration::from_secs(config.connect_timeout_secs);
    let identity_key = load_or_create_identity()?;
    let trust = TrustFile::open_default()?;
    println!("本机设备指纹 {}", identity_key.fingerprint());
    for file in &files {
        let stream = factory.connect(target, connect_timeout)?;
        let session: SessionSlot = session_slot();
        let mut secure = SecureTransport::initiator(
            stream,
            Arc::clone(&identity_key),
            Arc::clone(&trust) as Arc<dyn TrustList>,
            Arc::clone(&session),
        );
        // 先握手拿到校验码：接收方要靠它核对，所以必须发文件之前就显示
        secure.handshake()?;
        if let Some(info) = secure.session_info() {
            println!(
                "已与 {} 建立加密连接：对端指纹 {}{}",
                target,
                info.peer_fingerprint,
                if info.trusted { "（已信任）" } else { "" }
            );
            if !info.trusted {
                println!("校验码 **{}** ← 请与接收方屏幕上显示的核对", info.code);
            }
        }
        let view = CliView::new(true);
        let sent = transfer.send_file(&mut secure, file, &view)?;
        println!("\n已发送：{}（{}）", file.display(), human_size(sent));
    }
    Ok(())
}

// ================================================================ 守护进程

/// 待确认接收的等待上限：超时按拒绝处理。
const PENDING_TIMEOUT: Duration = Duration::from_secs(60);

/// 拉起守护进程等待其就绪的上限。
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

/// `glowlink on`：后台拉起守护进程（幂等：已在跑就直接报告）。
fn cmd_on(_config: &Config, args: &[String]) -> Result<()> {
    // --yes 已开放，但**只对已信任指纹生效**：陌生设备仍必须人工确认，
    // 否则「无感」等于「谁都能往你机器里塞文件」。
    let auto_accept = has_flag(args, "--yes");
    if let Some(state) = daemon::running_state() {
        print_on_summary(&state, "已在运行");
        return Ok(());
    }

    // 残留状态文件先清掉，免得客户端读到不存在的 IPC 端口。
    state::remove_state()?;

    let ipc_port = flag_u16(args, "--ipc-port")?.unwrap_or(ipc::IPC_PORT);
    let mut child_args = vec![daemon::DAEMON_COMMAND.to_string()];
    if auto_accept {
        child_args.push("--yes".to_string());
    }
    for flag in ["--dir", "--port", "--discovery-port"] {
        if let Some(value) = flag_value(args, flag) {
            child_args.push(flag.to_string());
            child_args.push(value);
        }
    }
    child_args.push("--ipc-port".to_string());
    child_args.push(ipc_port.to_string());

    let exe = daemon::current_exe()?;
    let child = daemon::spawn_detached(&exe, &child_args)?;

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(state) = daemon::running_state() {
            print_on_summary(&state, "已启动");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(GlowError::Usage(format!(
        "守护进程未能就绪（PID {}）。日志：{}",
        child.id(),
        state::log_path().display()
    )))
}

fn print_on_summary(state: &State, prefix: &str) {
    println!(
        "萤光守护进程{prefix}：PID {}，IPC 端口 {}，传输端口 {}（发现端口 {}），接收目录 {}",
        state.pid,
        state.ipc_port,
        state.transfer_port,
        state.discovery_port,
        state.receive_dir.display()
    );
    if let Ok(identity) = load_or_create_identity() {
        println!("本机设备指纹 {}", identity.fingerprint());
    }
    println!(
        "加密已启用；未配对设备需核对校验码后确认：glowlink list / accept <编号> / reject <编号>"
    );
    println!("日志 {}    停止：glowlink off", state::log_path().display());
}

/// `glowlink off`：优先优雅停止，IPC 不通时兜底强杀。
fn cmd_off(_config: &Config, _args: &[String]) -> Result<()> {
    let Some(state) = state::read_state()? else {
        println!("萤光守护进程未在运行。");
        return Ok(());
    };

    let stopping = match ipc::IpcClient::connect(&state) {
        Ok(mut client) => client.call("STOP").is_ok(),
        Err(_) => false,
    };
    if !stopping {
        // 连不上（进程卡死或状态文件过期），直接按 PID 强杀兜底
        daemon::terminate(state.pid);
    }

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && daemon::running_state().is_some() {
        std::thread::sleep(Duration::from_millis(100));
    }
    if daemon::running_state().is_some() {
        daemon::terminate(state.pid);
        std::thread::sleep(Duration::from_millis(200));
    }
    state::remove_state()?;
    println!("萤光守护进程（PID {}）已停止。", state.pid);
    Ok(())
}

/// `glowlink accept [编号]` / `glowlink reject [编号]`：
/// 为守护进程里挂起的接收请求落决定；有多个请求时必须给编号。
fn cmd_decide(accept: bool, args: &[String]) -> Result<()> {
    let state = daemon::running_state().ok_or_else(|| {
        GlowError::Usage("守护进程未在运行；接收确认只对守护进程有效。".into())
    })?;
    let token = positional_args(args).first().cloned();
    let command = match (&token, accept) {
        (Some(token), true) => format!("ACCEPT {token}"),
        (Some(token), false) => format!("REJECT {token}"),
        (None, true) => "ACCEPT".to_string(),
        (None, false) => "REJECT".to_string(),
    };
    let mut client = ipc::IpcClient::connect(&state)?;
    let line = client.call_first(&command)?;
    println!("{}", ipc::ok_payload(&line));
    Ok(())
}

/// 守护进程共享状态：IPC 线程只读这里的快照。
///
/// 发现循环**不能**和 IPC 共用一把锁：`tick()` 里要阻塞等收包（数百毫秒），
/// 持锁期间 IPC 查询会被整个饿死（实测 STATUS/DEVICES 直接超时）。
/// 所以发现循环独占控制器，只把设备快照发到这把锁里。
struct DaemonShared {
    state: Mutex<State>,
    devices: Mutex<Vec<glowlink_core::Device>>,
    pending: Arc<PendingQueue>,
    /// 是否对**已信任**设备免确认自动接收（`on --yes`）。
    auto_accept: bool,
    stop: AtomicBool,
}

impl IpcHost for DaemonShared {
    fn status(&self) -> State {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn devices(&self) -> Vec<glowlink_core::Device> {
        self.devices.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn find(&self, key: &str) -> Option<glowlink_core::Device> {
        self.devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|device| device.short_id() == key || device.name == key || device.id.as_str() == key)
            .cloned()
    }

    fn pending(&self) -> Vec<(String, PendingOffer)> {
        self.pending.list()
    }

    fn decide(&self, token: Option<&str>, accept: bool) -> Result<String> {
        self.pending.decide(token, accept)
    }

    fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// `glowlink __daemon`：守护进程本体（由 `on` 以分离进程拉起）。
fn cmd_daemon(config: &Config, args: &[String]) -> Result<()> {
    daemon::detach_session();

    let port = match flag_u16(args, "--port")? {
        Some(port) => port,
        None => config.transfer_port,
    };
    let discovery_port = discovery_port(config, args)?;
    let ipc_port = flag_u16(args, "--ipc-port")?.unwrap_or(ipc::IPC_PORT);
    let auto_accept = has_flag(args, "--yes");
    let dir = match flag_value(args, "--dir") {
        Some(value) => PathBuf::from(value),
        None => config.receive_dir.clone(),
    };
    fs::create_dir_all(&dir)?;

    // 长期身份与信任列表：守护进程全程复用同一份
    let identity_key = load_or_create_identity()?;
    let trust = TrustFile::open_default()?;

    let factory = NativeTransportFactory;
    let listener = factory.listen(port)?;
    let port = listener.local_port()?;
    // 与 serve 相同：发现端口被占时顺延，但已绑好的套接字不能重探
    let decision = glowlink_core::model::probe::decide_port(discovery_port)?;
    if let Some(notice) = decision.notice() {
        state::append_log(&notice);
    }
    // 顺延后要记实际端口：状态文件是客户端了解真实端口的唯一来源
    let bound_discovery_port = decision.port;
    let transport = factory.from_bound_socket(decision)?;
    let mut discovery = DiscoveryController::new(
        transport,
        DeviceIdentity::global().clone(),
        port,
        device_timeout(config),
        query_interval(config),
    );
    let (ipc_listener, ipc_port) = ipc::bind_ipc(ipc_port)?;

    let state = State {
        version: env!("CARGO_PKG_VERSION").to_string(),
        pid: std::process::id(),
        ipc_port,
        transfer_port: port,
        discovery_port: bound_discovery_port,
        receive_dir: dir.clone(),
        started_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };
    state::write_state(&state)?;
    state::append_log(&format!(
        "守护进程已启动：PID {}，传输端口 {port}，IPC 端口 {ipc_port}，接收目录 {}，\
         本机指纹 {}，{}",
        state.pid,
        dir.display(),
        identity_key.fingerprint(),
        if auto_accept {
            "自动接收：仅限已信任设备"
        } else {
            "自动接收：关闭（全部人工确认）"
        }
    ));

    let shared = Arc::new(DaemonShared {
        state: Mutex::new(state),
        devices: Mutex::new(Vec::new()),
        pending: Arc::new(PendingQueue::new()),
        auto_accept,
        stop: AtomicBool::new(false),
    });

    // IPC 线程：TCP 回环，一条连接可连发多条指令
    {
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            for stream in ipc_listener.incoming() {
                match stream {
                    Ok(stream) => ipc::serve_connection(shared.as_ref(), stream),
                    Err(e) => {
                        state::append_log(&format!("IPC 连接失败：{e}"));
                        continue;
                    }
                }
                if shared.stop.load(Ordering::SeqCst) {
                    break;
                }
            }
            state::append_log("收到停止指令，正在退出");
            let _ = state::remove_state();
            std::process::exit(0);
        });
    }

    // 接收线程：默认人工确认（DaemonView 阻塞等 IPC 落决定）
    {
        let listener: Arc<Box<dyn TransferListener>> = Arc::from(listener);
        let chunk_size = config.chunk_size();
        let ring_capacity = config.ring_capacity();
        let dir = dir.clone();
        let trust = Arc::clone(&trust);
        let identity_key = Arc::clone(&identity_key);
        let auto_accept = shared.auto_accept;
        let pending = Arc::clone(&shared.pending);
        std::thread::spawn(move || loop {
            match listener.accept() {
                // 每个到达的连接单开一个线程：等待确认期间不能挡住后面来的设备，
                // 否则第二个发送方只会静默挂在 backlog 里（就是单槽版本的毛病）。
                Ok((stream, peer)) => {
                    let trust = Arc::clone(&trust);
                    let identity_key = Arc::clone(&identity_key);
                    let dir = dir.clone();
                    let pending = Arc::clone(&pending);
                    std::thread::spawn(move || {
                        // 每个连接各自握手，会话信息（指纹 + 校验码）交给本连接的视图
                        let session: SessionSlot = session_slot();
                        let mut secure = SecureTransport::responder(
                            stream,
                            identity_key,
                            Arc::clone(&trust) as Arc<dyn TrustList>,
                            Arc::clone(&session),
                        );
                        let transfer = TransferController::new(chunk_size, ring_capacity);
                        let view = DaemonView::new(
                            pending,
                            PENDING_TIMEOUT,
                            session,
                            trust,
                            auto_accept,
                        );
                        let label = peer.ip().to_string();
                        match transfer.receive_file(&mut secure, &dir, &label, &view) {
                            Ok(path) => state::append_log(&format!("已接收：{}", path.display())),
                            Err(e) => state::append_log(&format!("接收失败：{e}")),
                        }
                    });
                }
                Err(e) => state::append_log(&format!("接受连接失败：{e}")),
            }
        });
    }

    // 发现主循环（本线程独占控制器，不占共享锁）
    while !shared.stop.load(Ordering::SeqCst) {
        match discovery.tick() {
            Ok(_) => {
                let snapshot = discovery.devices();
                *shared.devices.lock().unwrap_or_else(|e| e.into_inner()) = snapshot;
            }
            Err(e) => state::append_log(&format!("发现循环出错：{e}")),
        }
    }
    state::append_log("主循环退出");
    state::remove_state()?;
    Ok(())
}

fn search_timeout(config: &Config, args: &[String]) -> Duration {
    flag_secs(args, "--timeout").unwrap_or_else(|| Duration::from_secs(config.scan_timeout_secs))
}

fn query_interval(config: &Config) -> Duration {
    Duration::from_secs(config.query_interval_secs)
}

fn device_timeout(config: &Config) -> Duration {
    Duration::from_secs(config.device_timeout_secs)
}

/// 发现端口：命令行 > 配置文件。
fn discovery_port(config: &Config, args: &[String]) -> Result<u16> {
    Ok(flag_u16(args, "--discovery-port")?.unwrap_or(config.discovery_port))
}

fn print_help() {
    println!(
        "萤光 GlowLink {}\n\
         \n\
         用法：\n\
         \x20 glowlink list  [--discovery-port 端口] [--timeout 秒] [--watch]\n\
         \x20                                           扫描局域网内的设备（--watch 持续刷新）\n\
         \x20 glowlink serve [--port 传输端口] [--discovery-port 端口] [--dir 目录] [--yes]\n\
         \x20                                           常驻接收方（端口默认自动分配）\n\
         \x20 glowlink send  <设备名|设备ID> <文件>… [--timeout 秒]\n\
         \x20 glowlink send  --peer ip:端口 <文件>…\n\
         \x20                                           直连发送，绕过发现层\n\
         \x20 glowlink config                          显示生效配置与配置文件位置\n\n\
         守护进程：\n\
         \x20 glowlink on   [--dir 目录] [--yes] [--ipc-port 端口]\n\
         \x20                                           后台常驻；--yes 仅对已信任设备免确认\n\
         \x20 glowlink accept [编号] | reject [编号]   确认/拒绝待接收的文件（编号见 list）\n\
         \x20 glowlink off                           停止守护进程\n\n\
         配置文件：{}（工程默认）→ ~/.config/glowlink/config.toml（用户覆盖）\n\
         也可用 $GLOWLINK_CONFIG 指定用户配置文件路径。\n\
         发现端口默认 {}；被占用时自动顺延到相邻端口。\n\
         守护进程在跑时，list / send 自动改走本地 IPC。\n\
         传输已启用端到端加密（X25519 + ChaCha20-Poly1305）；\n\
         首次配对时请核对两端显示的 6 位校验码，确认后该设备进入信任列表。\n",
        env!("CARGO_PKG_VERSION"),
        config::PROJECT_CONFIG,
        Config::default().discovery_port
    );
}
