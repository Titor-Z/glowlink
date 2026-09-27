//! 资源：图标 svg 的**唯一来源**。
//!
//! `svg().path(..)` 只给路径，真正取字节靠 `AssetSource`。**不注册源，图标是静默不画的**
//! （不报错、不警告），所以这里必须注册——否则标题栏按钮与节点的图标全都消失，
//! 而界面上看不出「加载失败」，只会以为「设计本来就没画」。
//!
//! ## 图标房规（自己画或从别处拿图标都按这个改）
//!
//! 从别处找来的图标不能直接用：线条粗细、画布留白、颜色写法都不同，
//! 混在一起就是「手机和显示器一个分量、笔记本看着更重」。改法就四条：
//! 1. **画布 18×18 配 `stroke-width: 1.4`**（14 画布配 1.3，本机的 28 画布配 1.9）；
//! 2. 色写 **`currentColor`**（写死 `#F7F7F7` 之类就染不上了，`text_color` 不生效）；
//! 3. `stroke-linecap/linejoin="round"`、`fill="none"`（实心滑块是唯一例外）；
//! 4. 墨迹**别撑满画布**：设备图标的外轮廓高约 12～13px/18px，
//!    尺寸档位再由 `theme::ICON_*` 按外轮廓宽度补偿，两边别各改一次。

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// 嵌进二进制的图标。键就是 svg 里写的那个路径（`icons/xxx.svg`）。
///
/// 加一个图标要同时出现在两处：`assets/icons/` 下的文件 + 这张表。
/// 不写成「扫描目录」是刻意的：`include_bytes!` 要编译期常量，扫描做不到，
/// 而写错的路径在这里是**编译不过**，不会变成运行时静默丢图标。
const ICONS: &[(&str, &[u8])] = &[
    ("icons/close.svg", include_bytes!("../assets/icons/close.svg")),
    ("icons/desktop.svg", include_bytes!("../assets/icons/desktop.svg")),
    ("icons/laptop.svg", include_bytes!("../assets/icons/laptop.svg")),
    ("icons/lock.svg", include_bytes!("../assets/icons/lock.svg")),
    ("icons/monitor.svg", include_bytes!("../assets/icons/monitor.svg")),
    ("icons/phone.svg", include_bytes!("../assets/icons/phone.svg")),
    ("icons/tablet.svg", include_bytes!("../assets/icons/tablet.svg")),
    ("icons/toggle-3.svg", include_bytes!("../assets/icons/toggle-3.svg")),
    ("icons/tray.svg", include_bytes!("../assets/icons/tray.svg")),
    ("icons/unknown.svg", include_bytes!("../assets/icons/unknown.svg")),
];

/// 图标资源源（`assets/icons/` 全部嵌进二进制）。
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}
