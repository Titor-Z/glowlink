//! 萤光 GlowLink CLI：Phase 1 概念验证。
//!
//! 这里是唯一做装配（依赖注入）的地方：控制器由核心库提供，
//! 通道实现由工厂提供，视图与参数解析由本 crate 的库部分提供。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glowlink_cli::args::{flag_secs, flag_u16, flag_value, has_flag, positional_args, truncate};
use glowlink_cli::view::cli::human_size;
use glowlink_cli::CliView;
use glowlink_core::controller::transfer::TransferListener;
use glowlink_core::model::packet::{CHUNK_SIZE, DISCOVERY_PORT};
use glowlink_core::patterns::factory::{
    DiscoveryTransportFactory, NativeTransportFactory, TransferTransportFactory,
};
use glowlink_core::{DeviceIdentity, DiscoveryController, GlowError, Result, TransferController};

const DISCOVERY_INTERVAL: Duration = Duration::from_secs(1);
const DEVICE_TIMEOUT: Duration = Duration::from_secs(6);
const SCAN_TIMEOUT: Duration = Duration::from_secs(3);
const RING_CAPACITY: usize = 1 << 20;

fn main() {
    if let Err(e) = run() {
        eprintln!("错误：{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("list") => cmd_list(&args[1..]),
        Some("serve") => cmd_serve(&args[1..]),
        Some("send") => cmd_send(&args[1..]),
        Some("help") | None => {
            print_help();
            Ok(())
        }
        Some(other) => Err(GlowError::Usage(format!("未知命令：{other}"))),
    }
}

fn cmd_list(args: &[String]) -> Result<()> {
    let timeout = flag_secs(args, "--timeout").unwrap_or(SCAN_TIMEOUT);
    let discovery_port = flag_u16(args, "--discovery-port")?.unwrap_or(DISCOVERY_PORT);
    let factory = NativeTransportFactory;
    let identity = DeviceIdentity::global().clone();
    let transport = factory.create(discovery_port)?;
    let mut discovery =
        DiscoveryController::new(transport, identity.clone(), 0, DEVICE_TIMEOUT, DISCOVERY_INTERVAL);

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

fn cmd_serve(args: &[String]) -> Result<()> {
    let port = flag_u16(args, "--port")?.unwrap_or(0);
    let discovery_port = flag_u16(args, "--discovery-port")?.unwrap_or(DISCOVERY_PORT);
    let dir = PathBuf::from(flag_value(args, "--dir").unwrap_or_else(|| "./received".to_string()));
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
        DEVICE_TIMEOUT,
        DISCOVERY_INTERVAL,
    );

    let transfer = TransferController::new(CHUNK_SIZE, RING_CAPACITY);
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

fn cmd_send(args: &[String]) -> Result<()> {
    let timeout = flag_secs(args, "--timeout").unwrap_or(SCAN_TIMEOUT);
    let discovery_port = flag_u16(args, "--discovery-port")?.unwrap_or(DISCOVERY_PORT);
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
                DEVICE_TIMEOUT,
                DISCOVERY_INTERVAL,
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

    let transfer = TransferController::new(CHUNK_SIZE, RING_CAPACITY);
    let view = CliView::new(true);
    for file in &files {
        let mut stream = factory.connect(target, Duration::from_secs(10))?;
        let sent = transfer.send_file(stream.as_mut(), file, &view)?;
        println!("\n已发送：{}（{}）", file.display(), human_size(sent));
    }
    Ok(())
}

fn print_help() {
    println!(
        "萤光 GlowLink {}\n\
         \n\
         用法：\n\
         \x20 glowlink list  [--discovery-port 端口] [--timeout 秒]\n\
         \x20                                           扫描局域网内的设备\n\
         \x20 glowlink serve [--port 传输端口] [--discovery-port 端口] [--dir 目录] [--yes]\n\
         \x20                                           常驻接收方（传输端口默认自动分配）\n\
         \x20 glowlink send  <设备名|设备ID> <文件>… [--timeout 秒]\n\
         \x20 glowlink send  --peer ip:端口 <文件>…\n\
         \x20                                           直连发送，绕过发现层\n\n\
         发现端口默认 {DISCOVERY_PORT}；被占用时自动降级为主动查询模式。\n",
        env!("CARGO_PKG_VERSION")
    );
}
