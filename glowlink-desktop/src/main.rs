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

    // 装配点：有守护进程就用真数据，没有就退回预览
    // （界面照样起得来，便于离线比对设计稿）。
    let source: Arc<dyn DeviceSource> = match IpcSource::attach() {
        Some(ipc) => {
            log::info!("已连上守护进程，设备表来自 IPC");
            ipc
        }
        None => {
            log::info!("守护进程没在跑，设备表用预览数据（先 `glowlink on` 可看真设备）");
            Arc::new(PreviewSource::discovered())
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
