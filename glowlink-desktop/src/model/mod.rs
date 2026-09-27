//! Model：纯数据与算法，**零 IO、零 GPUI**。
//!
//! 三个模块各管一件事，互不依赖：
//! - `layout`：节点几何与命中判定（全窗坐标一套基准）
//! - `state`：界面状态机（四个状态互斥）
//! - `format`：字节/时间/百分比 → 界面字符
//! - `drop`：底部拖拽区的状态、文案与投放判定
//! - `drag`：拖动设备节点去配对的载荷、拖影比例与状态文案
//! - `connect`：拖到一起之后的连接态（缩小比例、连接环半径、方位分角）

pub mod connect;
pub mod drag;
pub mod drop;
pub mod format;
pub mod layout;
pub mod state;
