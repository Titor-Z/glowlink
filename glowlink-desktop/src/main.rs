//! `glowlink-desktop`：320×500 玻璃小窗。
//!
//! 装配点（依赖注入）只在这里：本机名、数据源、窗口都在这一层决定，
//! 控制器与视图都只认 trait。

// Windows 发布版不要那个黑框控制台：桌面版是给人双击的，
// 弹一个命令行窗口在后面既难看又像是出错了。
// 调试构建**保留**控制台——`RUST_LOG` 要看 stderr，这是唯一的第一手线索。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gpui::App;

use std::sync::Arc;

use glowlink_desktop::controller::{
    DeviceSource, IpcSource, PairingSource, PreviewPairing, PreviewSource, PreviewTransfer,
    PreviewTrusts, ReceiveDirSource, TransferSource, TrustSource,
};
use glowlink_desktop::model::settings;

fn main() {
    // 与 CLI 同一套日志口径：先看 stderr。gpui 的字体/渲染问题会把原因直接打日志，
    // 别先怀疑自己的样式代码。
    if std::env::var_os("RUST_LOG").is_some() {
        env_logger::init();
    }

    let name = glowlink_core::DeviceIdentity::global().name.clone();

    // 预览模式：只给离线比对设计稿用（设备表是写死的假数据，不碰真实服务）
    let preview = std::env::var_os("GLOWLINK_PREVIEW").is_some();
    // 预览变体（`xfer` / `xfer-done` / `pair` / `linked`）
    let variant = std::env::var("GLOWLINK_PREVIEW").unwrap_or_default();
    // `linked` = 一台设备 + 已连接（看拖拽区在「可发」状态下写什么）
    // `privacy` = 一开窗就把隐私浮层打开（看那张卡自己的排版）
    let preview_linked = variant == "linked" || variant == "privacy";
    let preview_privacy = variant == "privacy";
    let preview_settings = variant == "settings";
    // 设置先于一切加载：接收目录要在拉起守护进程时带上（`--dir`），主题决定开窗的样子
    let user_settings = settings::load();
    let ipc = IpcSource::spawn();

    // **打开软件即服务在跑**：没跑就拉起（异步，不阻塞开窗）。
    // 不自动拉起的话，每次测试都得先手敲一遍 `glowlink on` —— 那是把服务当命令行工具的用法。
    let (source, service, pairing, transfers, trusts, receive): (
        Arc<dyn DeviceSource>,
        Option<Arc<IpcSource>>,
        Option<Arc<dyn PairingSource>>,
        Option<Arc<dyn TransferSource>>,
        Option<Arc<dyn TrustSource>>,
        Option<Arc<dyn ReceiveDirSource>>,
    ) = if preview {
        log::info!("预览模式（GLOWLINK_PREVIEW=1）：设备表是设计稿里的假数据");
        // 预览默认**不带**卡片（卡片是真实状态）；想单独看排版用
        // `xfer` / `xfer-done` / `pair`（三张卡各自单独看，免得互相挡）
        let preview_transfer: Option<Arc<dyn TransferSource>> = match variant.as_str() {
            "xfer" => Some(Arc::new(PreviewTransfer::halfway())),
            "xfer-done" => Some(Arc::new(PreviewTransfer::finished_receive())),
            _ => None,
        };
        let preview_pairing: Option<Arc<dyn PairingSource>> = match variant.as_str() {
            "pair" => Some(Arc::new(PreviewPairing::default())),
            _ => None,
        };
        (
            Arc::new(PreviewSource::discovered()),
            None,
            preview_pairing,
            preview_transfer,
            Some(Arc::new(PreviewTrusts)),
            Some(Arc::new(PreviewTrusts)),
        )
    } else {
        if !ipc.running() {
            log::info!("服务没在跑，正在拉起守护进程…");
            ipc.start_service();
        }
        (
            Arc::clone(&ipc) as Arc<dyn DeviceSource>,
            Some(Arc::clone(&ipc)),
            Some(Arc::clone(&ipc) as Arc<dyn PairingSource>),
            Some(Arc::clone(&ipc) as Arc<dyn TransferSource>),
            Some(Arc::clone(&ipc) as Arc<dyn TrustSource>),
            Some(Arc::clone(&ipc) as Arc<dyn ReceiveDirSource>),
        )
    };

    gpui_ce_platform::application()
        // 图标必须注册资源源，否则 `svg()` 静默不画（不报错、不警告）
        .with_assets(glowlink_desktop::assets::Assets)
        .run(move |cx: &mut App| {
            glowlink_desktop::view::app::open(
                name.clone(),
                source,
                service,
                pairing,
                transfers,
                trusts,
                receive,
                user_settings,
                preview_linked,
                preview_privacy,
                preview_settings,
                cx,
            );
            cx.activate(true);
        });
}
