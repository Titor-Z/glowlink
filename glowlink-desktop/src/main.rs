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

    // 装配点（依赖注入）：有守护进程就用真数据；没有就**显示空表**。
    //
    // 「连不上就回落预览假数据」曾经很方便（离线比对设计稿），但它把假设备
    // 当真设备画了出来——用户看到 `Pixel 9` / `iPad mini` 会以为附近真有这些机器。
    // 所以预览改成显式开关：`GLOWLINK_PREVIEW=1`，只给设计比对用。
    let source: Arc<dyn DeviceSource> = match IpcSource::attach() {
        Some(ipc) => {
            log::info!("已连上守护进程，设备表来自 IPC");
            ipc
        }
        None if std::env::var_os("GLOWLINK_PREVIEW").is_some() => {
            log::info!("预览模式（GLOWLINK_PREVIEW=1）：设备表是设计稿里的假数据");
            Arc::new(PreviewSource::discovered())
        }
        None => {
            log::info!("守护进程没在跑：设备表为空（先 `glowlink on` 才能看到设备）");
            Arc::new(PreviewSource::empty())
        }
    };

    gpui_ce_platform::application()
        // 图标必须注册资源源，否则 `svg()` 静默不画（不报错、不警告）
        .with_assets(glowlink_desktop::assets::Assets)
        .run(move |cx: &mut App| {
            glowlink_desktop::view::app::open(name.clone(), source, cx);
            cx.activate(true);
        });
}
