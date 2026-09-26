//! 萤光 GlowLink CLI：Phase 1 概念验证。
//!
//! 这里是唯一做装配（依赖注入）的地方：配置来自配置文件 + 命令行覆盖，
//! 控制器由核心库提供，通道实现由工厂提供，视图由本 crate 的库部分提供。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glowlink_cli::args::{flag_secs, flag_u16, flag_value, has_flag, positional_args, truncate};
use glowlink_cli::config;
use glowlink_cli::view::cli::human_size;
use glowlink_cli::CliView;
use glowlink_core::controller::transfer::TransferListener;
use glowlink_core::patterns::factory::{
    DiscoveryTransportFactory, NativeTransportFactory, TransferTransportFactory,
};
use glowlink_core::{
    Config, DeviceIdentity, DiscoveryController, GlowError, Result, TransferController,
};

fn main() {
    if let Err(e) = run() {
        eprintln!("错误：{e}");
        std::process::exit(1);
    }
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
    let discovery_port = discovery_port(config, args)?;
    let query_every = query_interval(config);
    let factory = NativeTransportFactory;
    let identity = DeviceIdentity::global().clone();
    let transport = factory.create(discovery_port)?;
    let mut discovery = DiscoveryController::new(
        transport,
        identity.clone(),
        0,
        device_timeout(config),
        query_every,
    );

    println!("本机：{}（{}）", identity.name, truncate(&identity.id, 8));
    if discovery.degraded() {
        println!(
            "提示：发现端口 {discovery_port} 已被占用，已降级为主动查询模式（本地端口 {}）。",
            discovery.local_port()?
        );
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        discovery.tick()?;
    }

    let devices = discovery.devices();
    if devices.is_empty() {
        println!("未发现设备（已等待 {} 秒）", timeout.as_secs());
        return Ok(());
    }
    println!("{:<20} {:<10} {:<22} {}", "名称", "ID", "地址", "最后出现");
    for device in devices {
        println!(
            "{:<20} {:<10} {:<22} {}s",
            truncate(&device.name, 18),
            device.short_id(),
            device.addr,
            device.last_seen.elapsed().as_secs()
        );
    }
    Ok(())
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
    let transport = factory.create(discovery_port)?;
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
    if discovery.degraded() {
        println!(
            "提示：发现端口 {discovery_port} 已被占用，已降级为主动查询模式（本地端口 {}）。",
            discovery.local_port()?
        );
    }
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
         \x20 glowlink list  [--discovery-port 端口] [--timeout 秒]\n\
         \x20                                           扫描局域网内的设备\n\
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
