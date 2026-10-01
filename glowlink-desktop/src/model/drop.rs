//! Model：底部拖拽区的状态、文案与投放判定。
//!
//! 全部是纯函数，所以能单测——**这是这一块唯一测得动的东西**
//! （渲染要真实窗口，拖拽事件要真实 OS 拖放）。

/// 拖拽区的状态（对应设计稿的 `.dropzone` / `.is-over` / `.is-denied`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DropState {
    /// 没有任何拖拽在发生。
    #[default]
    Idle,
    /// 文件正悬在窗口上方。
    Over,
    /// 拖进来了但发不出去；带上原因（原因就是要显示给人看的那句话）。
    Denied(&'static str),
}

impl DropState {
    /// 提示文案。
    ///
    /// 三条规矩（都是「前置条件提前可见」的推论）：
    /// - **一台设备都没有**：写「先打开另一台设备的 GlowLink」——这时候写
    ///   「拖拽文件到这里」是邀请一个必然被拒的动作，用户得白丢一次才知道；
    /// - **多台、没选**：直接写「先选择一台设备」（用户拍板：提前说，别等丢进去才拒）；
    /// - **有明确目标**（已选，或只有一台——一台免选）：写「拖到此处 → 甲机」，
    ///   把「会发给谁」摆在明面上。
    pub fn note(self, zone: Zone<'_>) -> String {
        match self {
            // 拒收时**不显示目标名**：那时的问题不是「发给谁」而是「还没选」，
            // 写上目标名反而让人以为已经选好了
            DropState::Denied(reason) => reason.to_string(),
            DropState::Idle | DropState::Over => {
                // 服务关着：**先说要紧的**——没有服务，设备与文件都无从谈起
                match zone.service {
                    crate::model::state::ServiceState::Off => {
                        return "服务已关闭 · 打开右上角的开关".to_string();
                    }
                    crate::model::state::ServiceState::Starting => {
                        return "正在启动服务…".to_string();
                    }
                    crate::model::state::ServiceState::On => {}
                }
                if zone.device_count == 0 {
                    "先打开另一台设备的 GlowLink".to_string()
                } else if let Some(name) = zone.target() {
                    format!("拖到此处 → {name}")
                } else if zone.device_count > 1 {
                    "先选择一台设备".to_string()
                } else {
                    "拖拽文件到这里".to_string()
                }
            }
        }
    }
    /// 是不是「一次失败的投放」——界面据此决定要不要抖一下。
    pub fn is_denied(self) -> bool {
        matches!(self, DropState::Denied(_))
    }
}

/// 这批设备下，往拖拽区扔文件能不能发出去、发给谁。
///
/// 判定输入用的是**设备 ID**（不是名字）：名字可以重（同一台机器上跑两个守护进程时
/// 就完全一样），而「发给谁」必须是唯一的一台。
///
/// - `Ok(id)`：可以发，发给它。只有一台设备时**免选**（用户拍板的语义：一台还要求先点一下是废话）。
/// - `Err(原因)`：发不出去，原因是给人看的那句话。
///
/// 这条判定单独抽出来：它是**用户看得见的规则**，而事件处理那段测不动。
pub fn accept_target(
    device_ids: &[&str],
    selected: Option<&str>,
) -> std::result::Result<String, &'static str> {
    // 一台都没有：不是「没选」，是根本没得选——文案要说得出来
    if device_ids.is_empty() {
        return Err("附近还没有设备");
    }
    match selected {
        // 选中的还在：就用它
        Some(id) if device_ids.contains(&id) => Ok(id.to_string()),
        // 选了但它已经下线：**不许悄悄换一台发出去**——
        // 用户以为发给了甲、实际发给了乙，是最难发现的那种错。
        Some(_) => Err("选中的设备已不在了"),
        // 没选：只有一台就免选（用户拍板的语义），多台要求先点选（设计稿文案）
        None if device_ids.len() == 1 => Ok(device_ids[0].to_string()),
        None => Err("先选择一台设备"),
    }
}

/// 拖拽区文案要知道的现状（一行几个事实，免得参数列表越加越长）。
#[derive(Clone, Copy, Debug, Default)]
pub struct Zone<'a> {
    /// 后台服务现在是什么状态（关着的时候，说什么设备都是空话）。
    pub service: crate::model::state::ServiceState,
    /// 附近有几台设备。
    pub device_count: usize,
    /// 已选中的那台的名字。
    pub selected: Option<&'a str>,
    /// 附近唯一那台的名字（一台**免选**：它就是目标）。
    pub only: Option<&'a str>,
}

impl<'a> Zone<'a> {
    /// 这批文件实际会发给谁（没选时，只有一台也算有目标）。
    pub fn target(&self) -> Option<&'a str> {
        self.selected.or(self.only)
    }
}
