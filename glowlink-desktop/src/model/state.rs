//! Model：界面状态机。
//!
//! 设计稿的原话是**一次只强调一件事**：空态只有自机 + spinner，发现态只有节点，
//! 传输态才出现连线与卡片，配对态才出现码卡片——四个状态**互斥**，不叠加。
//! 所以这里用一个枚举而不是一堆 bool：`is_empty && is_transfer` 这种组合
//! 在类型上就不存在，界面也就画不出自相矛盾的画面。
//!
//! 零 IO、零 GPUI，全部可单测。

use std::time::Duration;

/// 界面状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// 启动后先「搜索中」：发现要花几百毫秒，直接跳到发现态会闪一下空图。
    Searching,
    /// 已发现设备（或确认附近确实没有设备）。
    Discover,
    /// 正在传输（连线 + 进度卡）。
    Transfer,
    /// 等待对方允许配对，或正在出示校验码。
    Pairing,
}

impl Phase {
    /// 设备节点是否可见。空态不显示设备（设计稿 `--dev-opacity: 0`）。
    pub fn shows_devices(self) -> bool {
        matches!(self, Phase::Discover | Phase::Transfer)
    }

    /// 是否显示「正在搜索附近设备…」。
    pub fn shows_searching(self) -> bool {
        matches!(self, Phase::Searching)
    }

    /// 是否画连线（只有传输态）。
    pub fn shows_wire(self) -> bool {
        matches!(self, Phase::Transfer)
    }

    /// 中心盘是否呼吸（设计稿：本机常驻呼吸；配对/传输时加快，见渲染层）。
    pub fn self_breathes(self) -> bool {
        true
    }
}

/// 界面状态机。
#[derive(Clone, Debug)]
pub struct UiState {
    phase: Phase,
    /// 用户选中的设备下标（投递文件的目标）。
    selected: Option<usize>,
    /// 已知设备台数（用于槽位分配与「+N」提示）。
    device_count: usize,
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

impl UiState {
    pub fn new() -> Self {
        UiState { phase: Phase::Searching, selected: None, device_count: 0 }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn device_count(&self) -> usize {
        self.device_count
    }

    /// 搜索结束（无论有没有找到设备都进入发现态：空图也是结果）。
    /// 由 [`Self::observe_devices`] 在设备表变化时调用。
    pub fn finish_search(&mut self) {
        if self.phase == Phase::Searching {
            self.phase = Phase::Discover;
        }
    }

    /// 更新设备台数，并据此决定「搜索中 / 发现态」。
    ///
    /// 为什么状态要跟着设备数走：启动后永远是 `Searching`，而 `Searching` 下
    /// **设备节点不画**（设计稿：搜索中只有自机 + spinner）。
    /// 实测就是这里漏了——真设备早就拿到了，屏幕上却只有中心那枚盘。
    /// 相反，设备全部掉线时不回到搜索态的话，空图会挂着上一次的残留。
    ///
    /// 只动 `Searching` / `Discover` 两态：传输、配对态由用户动作驱动，
    /// 不能被一次刷新掳走。
    pub fn observe_devices(&mut self, count: usize) {
        self.device_count = count;
        // 选中的设备下线了，选中必须跟着失效，否则界面指着空气
        if let Some(index) = self.selected {
            if index >= count {
                self.selected = None;
            }
        }
        match self.phase {
            Phase::Searching if count > 0 => self.finish_search(),
            Phase::Discover if count == 0 => self.phase = Phase::Searching,
            _ => {}
        }
    }

    /// 点选设备：再点一次取消选中。
    pub fn select(&mut self, index: usize) {
        if index >= self.device_count {
            return;
        }
        self.selected = if self.selected == Some(index) { None } else { Some(index) };
    }

    /// 开始传输。没有选中设备时**不进入传输态**——没有目标就没有传输。
    pub fn begin_transfer(&mut self) -> bool {
        if self.selected.is_none() {
            return false;
        }
        self.phase = Phase::Transfer;
        true
    }

    /// 传输结束，回到发现态（选中保留，便于「再发一个」）。
    pub fn end_transfer(&mut self) {
        if self.phase == Phase::Transfer {
            self.phase = Phase::Discover;
        }
    }

    /// 进入配对态（拖动设备到中心）。
    pub fn begin_pairing(&mut self) -> bool {
        if self.selected.is_none() {
            return false;
        }
        self.phase = Phase::Pairing;
        true
    }

    /// 配对结束，回到发现态。
    pub fn end_pairing(&mut self) {
        if self.phase == Phase::Pairing {
            self.phase = Phase::Discover;
        }
    }
}

/// 搜了多少秒之后就**不再假装还在搜**。
///
/// 「搜索中」超过这个时长，事实就是「附近没有设备」——继续转 spinner 是演给用户看，
/// 而且会一直吃 60fps 的重绘（空转 spinner 也是每帧都在动）。
pub const SEARCH_PATIENCE: Duration = Duration::from_secs(10);

/// 空态那句话：刚开始是「正在搜索」，久了就如实说没有。
pub fn search_note(waited: Duration) -> &'static str {
    if still_looking(waited) {
        "正在搜索附近设备…"
    } else {
        // 只报结论不够：用户盯着这句话时唯一的疑问是「我该怎么办」
        "附近还没有设备 · 让对方也打开 GlowLink"
    }
}

/// spinner 还转不转（不转的时候 tick 降到慢档，省掉每帧重绘）。
pub fn still_looking(waited: Duration) -> bool {
    waited < SEARCH_PATIENCE
}
