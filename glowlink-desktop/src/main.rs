//! `glowlink-desktop`：320×500 玻璃小窗。
//!
//! 装配点（依赖注入）只在这里：本机名、数据源、窗口都在这一层决定，
//! 控制器与视图都只认 trait。

use gpui::App;

use std::sync::Arc;

use glowlink_desktop::controller::{DeviceSource, IpcSource, PreviewSource};

fn main() {
    // 与 CLI 同一套日志口径：先看 stderr。gpui 的字体/渲染问题会把原因直接打日志，
    // 别先怀疑自己的样式代码。
    if std::env::var_os("RUST_LOG").is_some() {
        env_logger::init();
    }

    let name = glowlink_core::DeviceIdentity::global().name.clone();

    // 预览模式：只给离线比对设计稿用（设备表是写死的假数据，不碰真实服务）
    let preview = std::env::var_os("GLOWLINK_PREVIEW").is_some();
    let ipc = IpcSource::spawn();

    // **打开软件即服务在跑**：没跑就拉起（异步，不阻塞开窗）。
    // 不自动拉起的话，每次测试都得先手敲一遍 `glowlink on` —— 那是把服务当命令行工具的用法。
    let (source, service): (Arc<dyn DeviceSource>, Option<Arc<IpcSource>>) = if preview {
        log::info!("预览模式（GLOWLINK_PREVIEW=1）：设备表是设计稿里的假数据");
        (Arc::new(PreviewSource::discovered()), None)
    } else {
        if !ipc.running() {
            log::info!("服务没在跑，正在拉起守护进程…");
            ipc.start_service();
        }
        (Arc::clone(&ipc) as Arc<dyn DeviceSource>, Some(Arc::clone(&ipc)))
    };

    gpui_ce_platform::application()
        // 图标必须注册资源源，否则 `svg()` 静默不画（不报错、不警告）
        .with_assets(glowlink_desktop::assets::Assets)
        .run(move |cx: &mut App| {
            glowlink_desktop::view::app::open(name.clone(), source, service, cx);
            cx.activate(true);
        });
}
