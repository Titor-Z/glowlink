//! Model：界面状态机。
//!
//! 设计稿的原话是**一次只强调一件事**：空态只有自机 + spinner，发现态只有节点，
//! 传输态才出现连线与卡片，配对态才出现码卡片——四个状态**互斥**，不叠加。
//! 所以这里用一个枚举而不是一堆 bool：`is_empty && is_transfer` 这种组合
//! 在类型上就不存在，界面也就画不出自相矛盾的画面。
//!
//! 零 IO、零 GPUI，全部可单测。

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
    pub fn finish_search(&mut self) {
        if self.phase == Phase::Searching {
            self.phase = Phase::Discover;
        }
    }

    /// 更新设备台数。**不自动切换状态**：状态由用户动作驱动，
    /// 否则设备一多一少界面就会自己跳来跳去。
    pub fn set_devices(&mut self, count: usize) {
        self.device_count = count;
        // 选中的设备下线了，选中必须跟着失效，否则界面指着空气
        if let Some(index) = self.selected {
            if index >= count {
                self.selected = None;
            }
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
