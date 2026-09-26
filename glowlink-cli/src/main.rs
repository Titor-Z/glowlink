//! 萤光 GlowLink CLI：Phase 1 概念验证。
//!
//! 这里是唯一做装配（依赖注入）的地方：配置来自配置文件 + 命令行覆盖，
//! 控制器由核心库提供，通道实现由工厂提供，视图由本 crate 的库部分提供。

use std::io::{self, IsTerminal, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glowlink_cli::args::{flag_secs, flag_u16, flag_value, has_flag, positional_args, truncate};
use glowlink_cli::config;
use glowlink_cli::view::cli::human_size;
use glowlink_cli::CliView;
use glowlink_core::controller::transfer::TransferListener;
use glowlink_core::model::probe::{compare_versions, identify, PortOwner, VersionVerdict};
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
        print_devices(&discovery, timeout);
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
        print_devices_live(&discovery);
    }
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

fn print_devices(discovery: &DiscoveryController, timeout: Duration) {
    let devices = discovery.devices();
    if devices.is_empty() {
        println!("未发现设备（已等待 {} 秒）", timeout.as_secs());
        return;
    }
    println!("{} {}", device_table_header(), self_hint());
    for device in devices {
        println!("{}", device_row(&device, false));
    }
}

/// `--watch` 模式：表格原地重画，行数变化时不留下残留。
fn print_devices_live(discovery: &DiscoveryController) {
    let devices = discovery.devices();
    let mut lines = vec![device_table_header(), self_hint()];
    if devices.is_empty() {
        lines.push("  （搜索中…）".to_string());
    }
    for device in &devices {
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

    let transfer = TransferController::new(config.chunk_size(), config.ring_capacity());
    let view = Arc::new(CliView::new(auto_accept));

    println!(
        "萤光节点「{}」已启动：传输端口 {port}，接收目录 {}",
        identity.name,
        dir.display()
    );
    if !auto_accept {
        println!("收到文件时会先询问是否接收（--yes 可自动接收）。");
    }

    // 接收在独立线程中进行，主线程持续查询与刷新设备列表。
    {
        let listener: Arc<Box<dyn TransferListener>> = Arc::from(listener);
        let view = Arc::clone(&view);
        let dir = dir.clone();
        std::thread::spawn(move || loop {
            match listener.accept() {
                Ok((mut stream, peer)) => {
                    let label = peer.ip().to_string();
                    match transfer.receive_file(stream.as_mut(), &dir, &label, view.as_ref()) {
                        Ok(path) => println!("\n已接收：{}", path.display()),
                        Err(e) => eprintln!("\n接收失败：{e}"),
                    }
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
    };
    if direct {
        println!("直连目标：{target}");
    }

    let transfer = TransferController::new(config.chunk_size(), config.ring_capacity());
    let connect_timeout = Duration::from_secs(config.connect_timeout_secs);
    let view = CliView::new(true);
    for file in &files {
        let mut stream = factory.connect(target, connect_timeout)?;
        let sent = transfer.send_file(stream.as_mut(), file, &view)?;
        println!("\n已发送：{}（{}）", file.display(), human_size(sent));
    }
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
         配置文件：{}（工程默认）→ ~/.config/glowlink/config.toml（用户覆盖）\n\
         也可用 $GLOWLINK_CONFIG 指定用户配置文件路径。\n\
         发现端口默认 {}；被占用时自动降级为主动查询模式。\n",
        env!("CARGO_PKG_VERSION"),
        config::PROJECT_CONFIG,
        Config::default().discovery_port
    );
}
