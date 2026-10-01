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
use glowlink_cli::exit;
use glowlink_ipc::client::IpcClient;
use glowlink_ipc::codec;
use glowlink_ipc::daemon;
use glowlink_ipc::row::{
    DeviceRow, PairingRow, PendingRow, SendTicket, TransferRow,
};
use glowlink_ipc::server::{self as ipc_server, IpcHost};
use glowlink_ipc::state;
use glowlink_cli::keys::{install_device_id, load_or_create_identity, TrustFile};
use glowlink_cli::pairing::PairingHub;
use glowlink_cli::pending::PendingQueue;
use glowlink_cli::sendqueue;
use glowlink_cli::transfers::{Job, Transfers};
use glowlink_ipc::state::State;
use glowlink_cli::view::cli::human_size;
use glowlink_cli::view::tui::{self, Header, TuiAction};
use glowlink_cli::view::daemon as daemon_view;
use glowlink_cli::view::{panel, style};
use glowlink_cli::DaemonView;
use glowlink_cli::CliView;
use glowlink_core::controller::transfer::TransferListener;
use glowlink_core::model::probe::{compare_versions, identify, PortOwner, VersionVerdict};
use glowlink_core::patterns::secure::{session_slot, SecureTransport, SessionSlot, TrustList};
use glowlink_core::patterns::factory::{
    DiscoveryTransportFactory, NativeTransportFactory, TransferTransportFactory,
};
use glowlink_core::{
    detect, Config, DeviceIdentity, DiscoveryController, GlowError, Identity, Intent, PairingSession,
    Preloaded, Result, TransferController,
};

fn main() {
    // **第一件事**：用持久身份里的指纹注入本机 ID。
    // 必须早于任何 `DeviceIdentity::global()`——实例一旦用临时 ID 初始化，再注入就晚了，
    // 表现是「本机 ID 每次运行都变」。集成测试盯着这件事。
    install_device_id();

    reset_sigpipe();
    let code = match run() {
        Ok(()) => exit::OK,
        Err(e) => {
            // 用法错误自己就是完整的一句话（含命令名与用法），不再套「错误：用法错误：」两层
            match &e {
                GlowError::Usage(message) => eprintln!("{}", style().error(message)),
                other => eprintln!("{}", style().error(&format!("错误：{other}"))),
            }
            exit::code_of(&e)
        }
    };
    std::process::exit(code);
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
    // `<命令> --help` / `-h`：一律给这条命令自己的说明
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return cmd_help(&args);
    }

    match args.first().map(String::as_str) {
        Some("list") => cmd_list(&config, &args[1..]),
        Some("serve") => cmd_serve(&config, &args[1..]),
        // 守护进程本体：只由 `glowlink on` 拉起，不面向用户直接调用。
        Some(daemon::DAEMON_COMMAND) => cmd_daemon(&config, &args[1..]),
        Some("on") => cmd_on(&config, &args[1..]),
        Some("off") => cmd_off(&config, &args[1..]),
        Some("trust") => cmd_trust(&args[1..]),
        Some("ui") => cmd_ui(&config, &args[1..]),
        Some("accept") => cmd_decide(true, &args[1..]),
        Some("reject") => cmd_decide(false, &args[1..]),
        Some("send") => cmd_send(&config, &args[1..]),
        Some("config") => cmd_config(&config),
        // 敲 `glowlink` 回车：终端里进交互界面，管道/重定向里给纯文本面板
        None => {
            if io::stdout().is_terminal() {
                cmd_ui(&config, &[])
            } else {
                cmd_panel()
            }
        }
        Some("help") => cmd_help(&args[1..]),
        Some("--version") | Some("-V") => {
            println!("glowlink {VERSION}");
            Ok(())
        }
        Some(other) => Err(GlowError::Usage(format!(
            "glowlink: 未知命令「{other}」\n\n可用命令见 glowlink --help"
        ))),
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 打印一条提示语：`[警告]` 走黄、`[信息]`/`[提示]` 走青、其余原样。
///
/// 前缀本身就在文案里（核心库也产出这种文案），所以这里只负责上色，不改文本。
fn print_notice(line: &str) {
    if line.is_empty() {
        return;
    }
    println!(
        "{}",
        style().colorize(glowlink_cli::view::Level::from_prefix(line), line)
    );
}

/// 敲 `glowlink` 回车：只回答「现在什么状态 + 最常做的事」。
/// 问后台服务「现在有几个待确认的传输」。
///
/// 查不到就返回 `None`：状态面板与交互界面都不该因为这一项而失败。
fn daemon_pending(state: &state::State) -> Option<usize> {
    IpcClient::connect(state)
        .ok()
        .and_then(|mut client| client.call_first("STATUS").ok())
        .and_then(|line| {
            codec::ok_payload(&line)
                .split(' ')
                .nth(1)
                .and_then(|n| n.parse::<usize>().ok())
        })
}

fn cmd_panel() -> Result<()> {
    let identity = DeviceIdentity::global();
    // 先绑定到局部变量：面板要借用 receive_dir，临时值活不到打印那一刻
    let running = daemon::running_state();
    let daemon_status = match running.as_ref() {
        Some(state) => {
            let pending = daemon_pending(state);
            Some(panel::DaemonStatus {
                pid: state.pid,
                receive_dir: &state.receive_dir,
                pending,
            })
        }
        None => None,
    };

    println!(
        "{}",
        panel::status_panel(
            style(),
            VERSION,
            &identity.name,
            &truncate(&identity.id, 8),
            daemon_status.as_ref(),
        )
    );
    Ok(())
}

/// 设备表与待确认表的来源间隔：界面每 200ms 重绘，这里 300ms 取一次就够。
const DEVICE_POLL: Duration = Duration::from_millis(300);

/// 起一个后台线程持续产出界面快照（设备表 + 待确认表），界面只读快照。
///
/// 后台服务在跑时**必须走它的 IPC**：发现端口同一时刻只能有一个主人，
/// 界面再去绑一次只会降级成临时端口、看不到主动找上门的设备；
/// 待确认请求也只在后台服务里（界面只是它的前端）。
/// 查表失败时保留上一份结果，别让列表闪空。
fn spawn_snapshot_source(
    config: &Config,
    discovery_port: u16,
    span: u16,
) -> Result<Arc<Mutex<tui::Snapshot>>> {
    let snapshot: Arc<Mutex<tui::Snapshot>> = Arc::new(Mutex::new(tui::Snapshot::default()));

    if let Some(state) = daemon::running_state() {
        let sink = Arc::clone(&snapshot);
        std::thread::spawn(move || loop {
            if let Some(fresh) = daemon_snapshot(&state) {
                *sink.lock().unwrap_or_else(|e| e.into_inner()) = fresh;
            }
            std::thread::sleep(DEVICE_POLL);
        });
        return Ok(snapshot);
    }

    // 没有后台服务：自己起一个发现线程（UDP 收包会阻塞，不能放在界面线程里）；
    // 此时没有待确认请求可显示——接受/拒绝必须有后台服务来执行。
    let snapshot_for_thread = Arc::clone(&snapshot);
    let identity = DeviceIdentity::global().clone();
    let transport = NativeTransportFactory.create(discovery_port, span)?;
    let mut discovery = DiscoveryController::new(
        transport,
        identity,
        0,
        device_timeout(config),
        query_interval(config),
    );
    std::thread::spawn(move || loop {
        if discovery.tick().is_err() {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        let mut fresh = snapshot_for_thread.lock().unwrap_or_else(|e| e.into_inner());
        fresh.devices = discovery.devices();
    });
    Ok(snapshot)
}

/// 从后台服务取一份完整快照（设备 + 待确认）；取不到返回 `None`（保留上一份）。
fn daemon_snapshot(state: &State) -> Option<tui::Snapshot> {
    let mut client = IpcClient::connect(state).ok()?;
    let devices = ipc_devices(&client.call("DEVICES").ok()?);
    let pending = client
        .call("PENDING")
        .ok()
        .map(|lines| {
            lines
                .iter()
                .skip(1)
                .filter_map(|line| PendingRow::from_line(line))
                .map(|row| tui::PendingItem {
                    token: row.token,
                    peer: row.peer,
                    name: row.name,
                    size: row.size,
                    fingerprint: row.fingerprint,
                    code: row.code,
                })
                .collect()
        })
        .unwrap_or_default();
    Some(tui::Snapshot { devices, pending })
}

/// 交互式设备列表：选中设备后输入文件路径，回车即发送。
///
/// 界面只负责「选谁来收、发哪个文件」，真正的传输仍然走原来的发送流程
/// （退出界面、恢复终端之后再传），所以不会有两套进度渲染。
fn cmd_ui(config: &Config, args: &[String]) -> Result<()> {
    if !io::stdout().is_terminal() {
        return Err(GlowError::Usage(
            "glowlink ui: 需要终端环境\n\n\
             提示：脚本里请用 glowlink list（一次性列表）。"
                .into(),
        ));
    }
    let discovery_port = discovery_port(config, args)?;
    let span = discovery_port_span(config, args)?;
    let snapshot = spawn_snapshot_source(config, discovery_port, span)?;
    // 接受/拒绝只能由后台服务执行：它才是持有套接字与待确认队列的那一方
    let daemon_state = daemon::running_state();
    let daemon = daemon_state.as_ref().map(|state| state.pid);

    let identity = DeviceIdentity::global();
    let header = Header {
        version: VERSION.to_string(),
        local_name: identity.name.clone(),
        local_id: truncate(&identity.id, 8),
        daemon,
    };

    let decisions_allowed = daemon_state.is_some();
    let decide = |verb: &str, token: &str| -> std::result::Result<String, String> {
        let Some(state) = daemon_state.as_ref() else {
            return Err("需要后台服务才能接受/拒绝（先跑 glowlink on）".to_string());
        };
        let mut client = IpcClient::connect(state).map_err(|e| e.to_string())?;
        let lines = client
            .call(&format!("{verb} {token}"))
            .map_err(|e| e.to_string())?;
        Ok(lines
            .first()
            .map(|line| codec::ok_payload(line).to_string())
            .unwrap_or_default())
    };
    match tui::run(snapshot, header, decisions_allowed, decide)? {
        TuiAction::Quit => Ok(()),
        // 接受/拒绝在界面内完成，不会走到这里；走到说明逻辑有变，按退出处理
        TuiAction::Accept { .. } | TuiAction::Reject { .. } => Ok(()),
        TuiAction::Send { addr, name, path } => {
            println!("发送到「{name}」（{addr}）：{}", path.display());
            // 地址已经有了，直接直连发送，不再重新搜索一遍
            cmd_send(
                config,
                &[
                    "--peer".to_string(),
                    addr.to_string(),
                    path.to_string_lossy().to_string(),
                ],
            )
        }
    }
}

/// `glowlink help [命令]` 与 `glowlink <命令> --help` 的实现。
///
/// 不认识的命令名按用法错误处理（退出码 2）并指路总览——比默默打总览更容易发现问题。
fn cmd_help(args: &[String]) -> Result<()> {
    // 过滤掉开关：`glowlink help send --yes` 也应当给 send 的说明
    let name = args.iter().find(|arg| !arg.starts_with('-'));
    let Some(name) = name else {
        println!("{}", panel::help_text(style(), VERSION));
        return Ok(());
    };
    match panel::command_help(style(), name) {
        Some(text) => {
            println!("{text}");
            Ok(())
        }
        None => Err(GlowError::Usage(format!(
            "glowlink help: 没有「{name}」这个命令\n\n可用命令见 glowlink --help"
        ))),
    }
}

fn cmd_config(config: &Config) -> Result<()> {
    println!("生效配置（按 内置默认 → {} → 用户配置 顺序叠加）", config::PROJECT_CONFIG_SOURCE);
    println!("  已嵌入  {}", config::PROJECT_CONFIG_SOURCE);
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
    let span = discovery_port_span(config, args)?;

    // 守护进程在跑就走 IPC：既快（设备表是累积的，不用等一轮扫描），
    // 也避免和守护进程抢发现端口。
    if let Some(state) = daemon::running_state() {
        return list_via_daemon(&state, timeout, watch);
    }

    let factory = NativeTransportFactory;
    let identity = DeviceIdentity::global().clone();
    let transport = factory.create(discovery_port, span)?;
    let mut discovery = DiscoveryController::new(
        transport,
        identity.clone(),
        0,
        device_timeout(config),
        query_interval(config),
    );

    println!(
        "本机：{}（{}）",
        style().strong(&identity.name),
        truncate(&identity.id, 8)
    );
    if discovery.degraded() {
        print_notice(&degraded_notice(discovery_port));
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
            "glowlink list: --watch 需要终端环境（输出被重定向了）\n\n\
             提示：脚本里请用不带 --watch 的 glowlink list。"
                .into(),
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
    println!(
        "本机：{}（{}）",
        style().strong(&identity.name),
        truncate(&identity.id, 8)
    );
    print_notice(&format!(
        "[信息] 结果来自守护进程（PID {}，IPC 端口 {}），接收目录 {}。",
        state.pid,
        state.ipc_port,
        state.receive_dir.display()
    ));

    let mut client = IpcClient::connect(state)?;
    if !watch {
        let devices = ipc_devices(&client.call("DEVICES")?);
        print_devices(&devices, timeout);
        print_pending(&mut client)?;
        return Ok(());
    }

    if !io::stdout().is_terminal() {
        return Err(GlowError::Usage(
            "glowlink list: --watch 需要终端环境（输出被重定向了）\n\n\
             提示：脚本里请用不带 --watch 的 glowlink list。"
                .into(),
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
        .filter_map(|line| DeviceRow::from_line(line))
        // 存活时长→最近出现时刻的换算在 DeviceRow 里，两处各写一份就会漂
        .map(|row| row.to_device())
        .collect()
}

/// 有待确认的接收请求时逐条列出（守护进程不会自己弹窗）。
fn print_pending(client: &mut IpcClient) -> Result<()> {
    let lines = client.call("PENDING")?;
    let count = lines
        .first()
        .map(|line| codec::ok_payload(line).parse::<usize>().unwrap_or(0))
        .unwrap_or(0);
    if count == 0 {
        return Ok(());
    }
    println!();
    for line in lines.iter().skip(1) {
        let Some(row) = PendingRow::from_line(line)
        else {
            continue;
        };
        let trust_hint =
            daemon_view::pending_hint(&row.fingerprint, &row.code, daemon_trusts(&row.fingerprint));
        println!(
            "[等待确认 {}] {peer} 想发来「{name}」（{size}）",
            style().strong(&format!("#{}", row.token)),
            peer = row.peer,
            name = row.name,
            size = human_size(row.size),
        );
        if !trust_hint.is_empty() {
            println!("             {trust_hint}");
        }
        println!(
            "             确认：glowlink accept {token} / glowlink reject {token}",
            token = row.token
        );
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
            "[信息] 端口 {port} 被其它程序占用，本次改用临时端口：仍能发现设备，\
             但对端无法主动找到本机。可用 --discovery-port 换一个端口。"
        ),
        // 无人应答但绑不上：内核残留绑定或防火墙，不能说成“被其它程序占用”
        Ok(PortOwner::Stale) => format!(
            "[信息] 端口 {port} 被残留占用（需重启系统释放），本次改用临时端口：\
             仍能发现设备，但对端无法主动找到本机。"
        ),
        Ok(PortOwner::Free) => String::new(),
        Err(e) => format!("[信息] 端口 {port} 的占用情况探测失败：{e}"),
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
    style().dim(&format!("{:<20} {:<10} {:<22} {}", "名称", "ID", "地址", "最后出现"))
}

/// 数据窗口固定为 18 列：留 2 列给省略号，避免中文名撞坏表格。
fn device_row(device: &glowlink_core::Device, fresh: bool) -> String {
    let name = truncate(&device.name, 18);
    let age = device.last_seen.elapsed().as_secs();
    let marker = if fresh && age < 2 { "●" } else { " " };
    // 名称是用户要找的东西，加粗（去掉颜色后仍是原来的文本）
    let label = style().strong(&format!("{name:<19}"));
    format!(
        "{marker}{label} {:<10} {:<22} {}s",
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
    let span = discovery_port_span(config, args)?;
    let decision = glowlink_core::model::probe::decide_port_within(discovery_port, span)?;
    if let Some(notice) = decision.notice() {
        print_notice(&notice);
    }
    let transport = factory.from_bound_socket(decision, span)?;
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
                            Ok(path) => println!(
                                "\n{}",
                                style().ok(&format!("已接收：{}", path.display()))
                            ),
                            Err(e) => eprintln!("{}", style().error(&format!("\n接收失败：{e}"))),
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

/// `send` 的用法说明（缺参数与缺文件时共用，避免两处文案漂移）。
const SEND_USAGE: &str = "glowlink send: 缺少参数\n\n\
     用法：\n\
     \x20 glowlink send <设备名|设备ID> <文件>…\n\
     \x20 glowlink send --peer ip:端口 <文件>…\n\n\
     提示：先用 glowlink list 看看附近有哪些设备。";

fn cmd_send(config: &Config, args: &[String]) -> Result<()> {
    let timeout = search_timeout(config, args);
    let discovery_port = discovery_port(config, args)?;
    let span = discovery_port_span(config, args)?;
    let peer = flag_value(args, "--peer");
    let positional = positional_args(args);

    // 直连模式不需要设备名，位置参数全部视为文件。
    let (peer_key, file_args) = if peer.is_some() {
        (None, &positional[..])
    } else {
        if positional.len() < 2 {
            return Err(GlowError::Usage(SEND_USAGE.into()));
        }
        (Some(positional[0].clone()), &positional[1..])
    };
    if file_args.is_empty() {
        return Err(GlowError::Usage(SEND_USAGE.into()));
    }
    let files: Vec<PathBuf> = file_args.iter().map(PathBuf::from).collect();

    let factory = NativeTransportFactory;
    let direct = peer.is_some();
    let target = match peer {
        // 绕过发现层，直接单播连接，便于本机联调与跨网段排查。
        Some(value) => value
            .parse::<SocketAddr>()
            .map_err(|_| {
                GlowError::Usage(format!(
                    "glowlink send: --peer 需要写成 ip:端口，收到「{value}」"
                ))
            })?,
        None => {
            let peer_key = peer_key.expect("非直连模式必有设备名");
            // 守护进程在跑就直接向它要地址：省一轮扫描，也不和它抢发现端口。
            if let Some(state) = daemon::running_state() {
                let mut client = IpcClient::connect(&state)?;
                let (name, id, addr) = codec::parse_resolve(&client.call_first(&format!("RESOLVE {peer_key}"))?)?;
                println!("目标：{}（{}）@ {}", name, truncate(&id, 8), addr);
                addr
            } else {
                let identity = DeviceIdentity::global().clone();
                let transport = factory.create(discovery_port, span)?;
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
    let total_files = files.len();
    for (index, file) in files.iter().enumerate() {
        // 多文件时给出序号，用户才知道还要等几个（单文件不加，省得啰嗦）
        if let Some(label) = glowlink_cli::view::cli::file_index_label(
            index,
            total_files,
            &file.display().to_string(),
        ) {
            println!("{label}");
        }
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
                "{}",
                glowlink_cli::view::cli::session_banner(
                    &target.to_string(),
                    &info.peer_fingerprint,
                    &info.code,
                    info.trusted,
                )
            );
        }
        let view = CliView::new(true);
        let sent = transfer.send_file(&mut secure, file, &view)?;
        println!(
            "\n{}",
            style().ok(&format!("已发送：{}（{}）", file.display(), human_size(sent)))
        );
    }
    Ok(())
}

// ================================================================ 守护进程

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

    let ipc_port = flag_u16(args, "--ipc-port")?.unwrap_or(ipc_server::IPC_PORT);
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

    let state = daemon::ensure_running(&child_args, STARTUP_TIMEOUT)?;
    print_on_summary(&state, "已启动");
    Ok(())
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

/// `glowlink off`：优先优雅停止，IPC 不通时兜底强杀（机制在 `glowlink-ipc::daemon`）。
fn cmd_off(_config: &Config, _args: &[String]) -> Result<()> {
    match daemon::stop(Duration::from_secs(3))? {
        Some(state) => println!("萤光守护进程（PID {}）已停止。", state.pid),
        // 停止一个没在跑的服务不是错误（与既有行为一致）
        None => println!("萤光守护进程未在运行。"),
    }
    Ok(())
}

/// `glowlink trust [forget <指纹|名称>]`：查看/取消已配对设备的免确认。
fn cmd_trust(args: &[String]) -> Result<()> {
    let trust = TrustFile::open_default()?;
    let positional = positional_args(args);
    match positional.first().map(String::as_str) {
        None => {
            // 本机指纹永远打印：列表为空时用户更需要它来和对方核对
            let me = load_or_create_identity()?;
            println!("本机设备指纹 {}", me.fingerprint());
            let devices = trust.devices();
            if devices.is_empty() {
                println!("信任列表为空（还没有配对过任何设备）。");
                println!("配对方式：收到陌生设备的文件时核对 6 位校验码，然后 glowlink accept <编号>。");
                return Ok(());
            }
            println!("{:<18} {:<16} {}", "设备指纹", "名称", "配对时间");
            for device in &devices {
                println!(
                    "{:<18} {:<16} {}",
                    device.fingerprint,
                    truncate(&device.name, 14),
                    relative_time(device.paired_at)
                );
            }
            println!("\n取消某台设备的免确认：glowlink trust forget <指纹|名称>");
            Ok(())
        }
        Some("forget") => {
            let key = positional.get(1).ok_or_else(|| {
                GlowError::Usage(
                    "glowlink trust: 缺少要取消的设备\n\n\
                     用法：glowlink trust forget <指纹|名称>"
                        .into(),
                )
            })?;
            // 指纹和名称都允许，用户手上可能只有其中一个
            let target = trust
                .devices()
                .into_iter()
                .find(|d| d.fingerprint == *key || d.name == *key)
                .ok_or_else(|| {
                    GlowError::Missing(format!(
                        "信任列表里没有「{key}」，用 glowlink trust 看看有哪些设备"
                    ))
                })?;
            trust.forget(&target.fingerprint)?;
            println!(
                "已取消 {}（{}）的免确认，下次它发文件会重新要求人工确认。",
                target.fingerprint, target.name
            );
            Ok(())
        }
        Some(other) => Err(GlowError::Usage(format!(
            "glowlink trust: 未知子命令「{other}」\n\n\
             用法：glowlink trust [forget <指纹|名称>]"
        ))),
    }
}

/// 把配对时间说成人话（不引入日期库）。
fn relative_time(paired_at: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let elapsed = now.saturating_sub(paired_at);
    match elapsed {
        0..=59 => "刚刚".to_string(),
        60..=3599 => format!("{} 分钟前", elapsed / 60),
        3600..=86_399 => format!("{} 小时前", elapsed / 3600),
        _ => format!("{} 天前", elapsed / 86_400),
    }
}

/// `glowlink accept [编号]` / `glowlink reject [编号]`：
/// 为守护进程里挂起的接收请求落决定；有多个请求时必须给编号。
fn cmd_decide(accept: bool, args: &[String]) -> Result<()> {
    // 命令本身没写错、只是环境没准备好，所以算通用错误（1）而不是用法错误（2）
    let state = daemon::running_state().ok_or_else(|| {
        GlowError::Missing(format!(
            "glowlink {}: 后台服务未在运行\n\n提示：先跑 glowlink on；前台接收请用 glowlink serve。",
            if accept { "accept" } else { "reject" }
        ))
    })?;
    let token = positional_args(args).first().cloned();
    let command = match (&token, accept) {
        (Some(token), true) => format!("ACCEPT {token}"),
        (Some(token), false) => format!("REJECT {token}"),
        (None, true) => "ACCEPT".to_string(),
        (None, false) => "REJECT".to_string(),
    };
    let mut client = IpcClient::connect(&state)?;
    let line = client.call_first(&command)?;
    println!("{}", codec::ok_payload(&line));
    Ok(())
}

/// 发送一个文件所需的依赖快照（从 [`DaemonShared`] 搬出来给线程用）。
struct SendWorker {
    identity: Arc<Identity>,
    trust: Arc<TrustFile>,
    chunk_size: usize,
    ring_capacity: usize,
    connect_timeout: Duration,
}

impl SendWorker {
    /// 连上 → 握手 → 上报校验码 → 传文件。
    ///
    /// “被对方拒绝”和“传输失败”是两种结果：前者是明确结论（`job.reject`），
    /// 后者才是错误（`job.fail`）—— 界面的文案不同，不能搔成一团。
    fn send(
        &self,
        addr: SocketAddr,
        path: &std::path::Path,
        peer_name: &str,
        job: &Job,
    ) -> Result<()> {
        // 轮到这个文件了（前面那个已经传完）：从「等待中」转成「正在传」
        job.mark_running();
        let factory = NativeTransportFactory;
        let stream = factory.connect(addr, self.connect_timeout)?;
        let session: SessionSlot = session_slot();
        let mut secure = SecureTransport::initiator(
            stream,
            Arc::clone(&self.identity),
            Arc::clone(&self.trust) as Arc<dyn TrustList>,
            Arc::clone(&session),
        );
        secure.handshake()?;
        // 校验码必须**在传文件之前**交给调用方：接收侧要拿它核对，
        // 发送侧屏幕上没有码，核对就成了空话（这个坑踩过一次）。
        let code = secure.session_info().map(|info| info.code.clone()).unwrap_or_default();
        // 码记进**这一行**（不是 `SEND` 的回执）：批次是排队的，第 3 个文件的码
        // 要等它真的轮到才有，界面从 `TRANSFERS` 里拿。
        job.set_code(code.clone());
        state::append_log(&format!(
            "正在向 {peer_name} 发送「{}」（校验码 {code}）",
            path.display()
        ));

        let transfer = TransferController::new(self.chunk_size, self.ring_capacity);
        match transfer.send_file(&mut secure, path, job) {
            Ok(sent) => {
                job.finish();
                state::append_log(&format!(
                    "已发送给 {peer_name}：{}（{sent} 字节）",
                    path.display()
                ));
                Ok(())
            }
            // 用户取消（桌面版的 ✕ 经 IPC 打进来）：明确结论，不是故障
            Err(GlowError::Canceled(_)) => {
                job.cancel();
                state::append_log(&format!("已取消向 {peer_name} 发送：{}", path.display()));
                Ok(())
            }
            // 对方拒收：这是一条明确结论，不是错误（界面文案也不一样）
            Err(GlowError::Rejected(reason)) => {
                job.reject(reason.clone());
                state::append_log(&format!("向 {peer_name} 发送被拒绝：{reason}"));
                Ok(())
            }
            Err(e) => {
                state::append_log(&format!("向 {peer_name} 发送失败：{e}"));
                Err(e)
            }
        }
    }
}

/// 从会话槽里取对端指纹（握手已完成时才有）。
fn session_info_fingerprint(session: &SessionSlot) -> String {
    session
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|info| info.peer_fingerprint.clone())
        .unwrap_or_default()
}

/// 按指纹在设备表里找对端名字（找不到返回 `None`）。
///
/// 设备 ID 就是它的身份指纹，所以这里能对上；对不上说明它不在当前设备表里
/// （例如刚上线、或设备表被 TTL 清掉）——那时由调用方决定退成什么。
fn peer_label(devices: &Mutex<Vec<glowlink_core::Device>>, fingerprint: &str) -> Option<String> {
    if fingerprint.is_empty() {
        return None;
    }
    devices
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|device| device.id.as_str() == fingerprint)
        .map(|device| device.name.clone())
}

/// 设备名对不上时的显示名：短指纹（与 `glowlink trust` 里那串字符一致）。
fn short_fingerprint(fingerprint: &str) -> String {
    if fingerprint.is_empty() {
        return "未知设备".to_string();
    }
    truncate(fingerprint, 8)
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
    /// 配对：两个方向的槽位表 + 两条流程都住在这里。
    pairing: Arc<PairingHub>,
    /// 传输登记表：界面靠它画进度。
    transfers: Arc<Transfers>,
    /// 发送队列：`SEND` 只入队，工作线程按顺序一个文件一个文件地发。
    /// 住在这里（而不是界面里）＝「窗口关掉，剩下的照样发出去」。
    send_queue: sendqueue::SharedQueue,
    /// 发送要用的长期身份与信任列表（与接收侧同一份）。
    identity: Arc<Identity>,
    trust: Arc<TrustFile>,
    chunk_size: usize,
    ring_capacity: usize,
    connect_timeout: Duration,
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

    fn is_trusted(&self, device_id: &str) -> bool {
        // 设备 ID 就是它的身份指纹（两侧同一套派生），信任列表按指纹存
        self.pairing.is_trusted(device_id)
    }

    fn find(&self, key: &str) -> Option<glowlink_core::Device> {
        self.devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|device| device.short_id() == key || device.name == key || device.id.as_str() == key)
            .cloned()
    }

    fn pending(&self) -> Vec<PendingRow> {
        self.pending.rows()
    }

    fn decide(&self, token: Option<&str>, accept: bool) -> Result<String> {
        self.pending.decide(token, accept)
    }

    fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    fn pair(&self, key: &str) -> Result<(String, String)> {
        // 先按名称/ID 找到它——配对必须先知道连谁（发现层是唯一的地址来源）
        let device = self.find(key).ok_or_else(|| GlowError::NotFound(key.to_string()))?;
        self.pairing.start(&device)
    }

    fn pairings(&self) -> Vec<PairingRow> {
        self.pairing.queue().outgoing_rows()
    }

    fn pair_pending(&self) -> Vec<PairingRow> {
        self.pairing.queue().pending_rows()
    }

    fn pair_decide(&self, token: &str, code: Option<String>) -> Result<String> {
        self.pairing.queue().decide(token, code)
    }
    fn pair_cancel(&self, token: &str) -> Result<String> {
        let token = self.pairing.queue().cancel_outgoing(token)?;
        state::append_log(&format!("已放弃配对 #{token}"));
        Ok(token)
    }

    fn transfers(&self) -> Vec<TransferRow> {
        self.transfers.rows()
    }

    /// 取消一次传输（桌面版的 ✕）。
    ///
    /// 只**置标志**：真正的收尾由传输线程做（它才知道传到哪儿了）。
    /// 编号不存在或已经结束就如实报错，不假装取消成功。
    fn cancel(&self, token: &str) -> Result<String> {
        self.transfers
            .cancel(token)
            .ok_or_else(|| GlowError::Missing(format!("没有正在进行的传输 #{token}")))
    }

    /// 把文件发给某台设备：握手**在这里等**（校验码要立刻能显示），传输丢给线程。
    fn send_file(&self, device_id: &str, path: &str) -> Result<SendTicket> {
        let device = self.find(device_id).ok_or_else(|| GlowError::NotFound(device_id.to_string()))?;

        let path = PathBuf::from(path);
        let meta = fs::metadata(&path)?;
        if !meta.is_file() {
            return Err(GlowError::Protocol(format!("不是普通文件：{}", path.display())));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        // 登记 + 入队，**立刻**返回：队列跑在守护进程里，界面不必等
        // （窗口关掉，后面几个文件照样会发出去）。
        let job = self.transfers.start_queued(name, device.name.clone(), meta.len());
        let token = job.token();
        self.send_queue.push(sendqueue::Item {
            addr: device.addr,
            peer: device.name.clone(),
            path,
            job,
        });
        state::append_log(&format!(
            "已排入发送队列 #{token}（队列 {} 个，对端 {}）",
            self.send_queue.len(),
            device.name
        ));
        Ok(SendTicket { token })
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
    let ipc_port = flag_u16(args, "--ipc-port")?.unwrap_or(ipc_server::IPC_PORT);
    let auto_accept = has_flag(args, "--yes");
    let dir = match flag_value(args, "--dir") {
        Some(value) => PathBuf::from(value),
        None => config.receive_dir.clone(),
    };
    fs::create_dir_all(&dir)?;

    // 长期身份与信任列表：守护进程全程复用同一份
    let identity_key = load_or_create_identity()?;
    let trust = TrustFile::open_default()?;
    let pairing = PairingHub::new(
        Arc::clone(&identity_key),
        Arc::clone(&trust),
        Duration::from_secs(config.pending_timeout_secs),
    );

    let factory = NativeTransportFactory;
    let listener = factory.listen(port)?;
    let port = listener.local_port()?;
    // 与 serve 相同：发现端口被占时顺延，但已绑好的套接字不能重探
    let span = discovery_port_span(config, args)?;
    let decision = glowlink_core::model::probe::decide_port_within(discovery_port, span)?;
    if let Some(notice) = decision.notice() {
        state::append_log(&notice);
    }
    // 顺延后要记实际端口：状态文件是客户端了解真实端口的唯一来源
    let bound_discovery_port = decision.port;
    let transport = factory.from_bound_socket(decision, span)?;
    let mut discovery = DiscoveryController::new(
        transport,
        DeviceIdentity::global().clone(),
        port,
        device_timeout(config),
        query_interval(config),
    );
    let (ipc_listener, ipc_port) = ipc_server::bind_ipc(ipc_port)?;

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
        pairing,
        transfers: Arc::new(Transfers::new()),
        send_queue: Arc::new(sendqueue::SendQueue::new()),
        identity: Arc::clone(&identity_key),
        trust: Arc::clone(&trust),
        chunk_size: config.chunk_size(),
        ring_capacity: config.ring_capacity(),
        connect_timeout: Duration::from_secs(config.connect_timeout_secs),
        auto_accept,
        stop: AtomicBool::new(false),
    });

    // 发送工作线程：**一条**，按队列顺序发（同一时刻只有一个文件在传）。
    // 队列里每件的连接/握手都在这里做——所以 `SEND` 能立刻返回。
    {
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            // 与发送同一个依赖快照（身份、信任、超时、分块大小）
            let worker = SendWorker {
                identity: Arc::clone(&shared.identity),
                trust: Arc::clone(&shared.trust),
                chunk_size: shared.chunk_size,
                ring_capacity: shared.ring_capacity,
                connect_timeout: shared.connect_timeout,
            };
            loop {
                let item = shared.send_queue.wait();
                // 用户在排队期间取消了它：不发，状态已经是「已取消」
                if item.job.is_canceled() {
                    state::append_log(&format!(
                        "队列中的「{}」已被取消，跳过",
                        item.path.display()
                    ));
                    continue;
                }
                if let Err(e) = worker.send(item.addr, &item.path, &item.peer, &item.job) {
                    item.job.fail(e.to_string());
                }
            }
        });
    }

    // IPC 线程：TCP 回环，一条连接可连发多条指令
    {
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            for stream in ipc_listener.incoming() {
                match stream {
                    Ok(stream) => ipc_server::serve_connection(shared.as_ref(), stream),
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
        let pairing = Arc::clone(&shared.pairing);
        let transfers_for_recv = Arc::clone(&shared.transfers);
        // 接连接时按指纹查设备名（把 IP 挡在用户视线之外）
        let shared_for_recv = Arc::clone(&shared);
        let config_pending_timeout = config.pending_timeout_secs;
        std::thread::spawn(move || loop {
            match listener.accept() {
                // 每个到达的连接单开一个线程：等待确认期间不能挡住后面来的设备，
                // 否则第二个发送方只会静默挂在 backlog 里（就是单槽版本的毛病）。
                Ok((stream, _peer)) => {
                    let trust = Arc::clone(&trust);
                    let identity_key = Arc::clone(&identity_key);
                    let dir = dir.clone();
                    let pending = Arc::clone(&pending);
                    let pairing = Arc::clone(&pairing);
                    let transfers = Arc::clone(&transfers_for_recv);
                    let shared_conn = Arc::clone(&shared_for_recv);
                    std::thread::spawn(move || {
                        // 每个连接各自握手，会话信息（指纹 + 校验码）交给本连接的视图
                        let session: SessionSlot = session_slot();
                        let mut secure = SecureTransport::responder(
                            stream,
                            identity_key,
                            Arc::clone(&trust) as Arc<dyn TrustList>,
                            Arc::clone(&session),
                        );
                        // 配对与传输**共用同一个端口**：握手后的第一帧决定这次连接干什么
                        let (intent, first) = match detect(&mut secure) {
                            Ok(value) => value,
                            Err(e) => {
                                state::append_log(&format!("握手失败：{e}"));
                                return;
                            }
                        };
                        if let Intent::Pair { name } = intent {
                            match PairingSession::over(secure) {
                                Ok(session) => {
                                    if let Err(e) = pairing.handle_incoming(session, name) {
                                        state::append_log(&format!("配对处理失败：{e}"));
                                    }
                                }
                                Err(e) => state::append_log(&format!("配对会话建立失败：{e}")),
                            }
                            return;
                        }
                        // 传输：把已读到的那一帧放回去，transfer 那边看不出差别
                        let transfer = TransferController::new(chunk_size, ring_capacity);
                        // 对端显示名：按指纹从设备表里找（先取，`session` 要移进视图）。
                        // **不用 IP**：那东西在桌面版里属于「不该给用户看」的技术细节，
                        // 而卡片上写「来自 127.0.0.1」对用户毫无意义。
                        // 找不到就退成 IP（CLI 侧可以看，桌面版不展示它）。
                        let fingerprint = session_info_fingerprint(&session);
                        // 兜底不用 IP：那是技术细节，桌面版一概不展示。
                        // 短指纹与信任列表里看到的是同一串字符，用户对得上。
                        let label = peer_label(&shared_conn.devices, &fingerprint)
                            .unwrap_or_else(|| short_fingerprint(&fingerprint));
                        let view = DaemonView::new(
                            pending,
                            Duration::from_secs(config_pending_timeout),
                            session,
                            trust,
                            auto_accept,
                            Arc::clone(&transfers),
                        );
                        let mut stream = Preloaded::new(secure, first);
                        match transfer.receive_file(&mut stream, &dir, &label, &view) {
                            Ok(path) => {
                                view.finish();
                                state::append_log(&format!("已接收：{}", path.display()));
                            }
                            Err(e) => {
                                view.fail(e.to_string());
                                state::append_log(&format!("接收失败：{e}"));
                            }
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
            Err(e) => {
                // 兜底退让：错误分支若不睡，任何持续性错误都会变成忙等（CPU 空转 + 写爆日志）
                state::append_log(&format!("发现循环出错：{e}"));
                std::thread::sleep(Duration::from_secs(1));
            }
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

/// 候选端口个数：命令行 > 配置文件 > 内置默认（协议常量）。
///
/// 命名为 `span`（跨度/个数）而不是 `range`：后者容易读成「起止两端」，
/// 与既有 `--discovery-port`（起点）配对更清楚。
fn discovery_port_span(config: &Config, args: &[String]) -> Result<u16> {
    let span = flag_u16(args, "--discovery-port-span")?.unwrap_or(config.discovery_port_span);
    if !(1..=glowlink_core::model::config::MAX_PORT_SPAN).contains(&span) {
        return Err(GlowError::Usage(format!(
            "glowlink: --discovery-port-span 需在 1..={} 之间，收到 {span}",
            glowlink_core::model::config::MAX_PORT_SPAN
        )));
    }
    Ok(span)
}

