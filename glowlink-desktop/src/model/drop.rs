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
    /// 四条规矩（都是「前置条件提前可见」的推论）：
    /// - **一台设备都没有**：写「暂无可发送的设备」——说清**这台软件现在的状态**，
    ///   而不是去指使对端（早先写的「先打开另一台设备的 GlowLink」是把实现细节
    ///   说成了给用户的命令；产品口吻只描述用户看得见的事）；
    /// - **有设备但一台都没连接**：写「把设备拖到中间即可发送」（用户拍板：**发送的前提是已连接**）。
    ///   发现 ≠ 可以发：刚看到的陌生设备还没经过配对，写「拖到此处 → 甲机」等于承诺一件
    ///   流程不允许的事，用户丢进来才发现被拒；
    /// - **已连接多台、没选**：直接写「先选择一台设备」（提前说，别等丢进去才拒）；
    /// - **有明确目标**（已选，或只有一台已连接——一台免选）：写「拖到此处 → 甲机」，
    ///   把「会发给谁」摆在明面上。
    pub fn note(self, zone: Zone<'_>) -> String {
        match self {
            // 拒收时**不显示目标名**：那时的问题不是「发给谁」而是「还没选」，
            // 写上目标名反而让人以为已经选好了
            DropState::Denied(reason) => reason.to_string(),
            DropState::Idle | DropState::Over => {
                // 没在找设备：**先说要紧的**——这时候说设备与文件都是空话
                match zone.service {
                    crate::model::state::ServiceState::Off => {
                        return "已暂停查找设备".to_string();
                    }
                    crate::model::state::ServiceState::Starting => {
                        return "正在启动…".to_string();
                    }
                    crate::model::state::ServiceState::On => {}
                }
                if zone.device_count == 0 {
                    // 只描述状态：没有可发的对象。指使对端「去打开另一台设备的 GlowLink」
                    // 是开发者视角（我们清楚要有个进程在跑），不是用户视角
                    NO_DEVICES.to_string()
                } else if zone.connected == 0 {
                    // 有设备、但一台都没连上：**先连接**。这一条必须在
                    // 「拖到此处」之前判——否则就是在承诺一件流程不允许的事。
                    // 这里说的是**收益**（拖过去就能发），比一句命令更像产品
                    CONNECT_HINT.to_string()
                } else if let Some(name) = zone.target() {
                    format!("拖到此处 → {name}")
                } else if zone.connected > 1 {
                    "先选择一台设备".to_string()
                } else {
                    // 只有一台已连接时上面就会给出目标名；走到这里说明调用方漏填 `only`
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

/// 拒收时的纠正话（短、命令式）：投放被拒、或点了没连的设备时用它。
pub const CONNECT_FIRST: &str = "先把设备拖到中间连接";
/// 常驻的提示话（说收益）：拖拽区在「有设备但没连」时显示它。
///
/// 与 [`CONNECT_FIRST`] 同一件事、两种角色：常驻的要说**这么做能得到什么**
/// （用户看它一眼就知道下一步），拒收时的要说**这次为什么没成**（已经动手了，要短）。
pub const CONNECT_HINT: &str = "把设备拖到中间即可发送";
/// 附近没有设备时的说法（状态，不是对别人的要求）。判定与提示共用。
pub const NO_DEVICES: &str = "附近没有设备";

/// 这批设备下，往拖拽区扔文件能不能发出去、发给谁。
///
/// 判定输入用的是**设备 ID**（不是名字）：名字可以重（同一台机器上跑两个守护进程时
/// 就完全一样），而「发给谁」必须是唯一的一台。
///
/// **发送的前提是「已连接」**（用户拍板）：`discovered` 里发现到的设备只是「看得见」，
/// 要往它发文件得先拖到中心连上（配对）。发现 ≠ 可发，所以这里两张名单都要：
/// `discovered` 只用来区分「附近没有设备」与「有但没连」，**可发名单是 `connected`**。
///
/// - `Ok(id)`：可以发，发给它。只有一台已连接时**免选**（一台还要求先点一下是废话）。
/// - `Err(原因)`：发不出去，原因是给人看的那句话。
///
/// 这条判定单独抽出来：它是**用户看得见的规则**，而事件处理那段测不动。
pub fn accept_target(
    discovered: &[&str],
    connected: &[&str],
    selected: Option<&str>,
) -> std::result::Result<String, &'static str> {
    // 一台都没有：不是「没选」，是根本没得选——文案要说得出来
    if discovered.is_empty() {
        return Err(NO_DEVICES);
    }
    // 有设备但都没连：不许发（流程规则）。这一条要排在选中之前判，
    // 否则「选了一台没连的」会被当成正常路径走出去
    if connected.is_empty() {
        return Err(CONNECT_FIRST);
    }
    match selected {
        // 选中的已连接：就用它
        Some(id) if connected.contains(&id) => Ok(id.to_string()),
        // 选中的还在附近但没连：实话实说（不是「不在了」，别让用户去找设备）
        Some(id) if discovered.contains(&id) => Err(CONNECT_FIRST),
        // 选了但它已经下线：**不许悄悄换一台发出去**——
        // 用户以为发给了甲、实际发给了乙，是最难发现的那种错。
        Some(_) => Err("那台设备已不在了"),
        // 没选：只有一台已连接就免选（用户拍板的语义），多台要求先点选（设计稿文案）
        None if connected.len() == 1 => Ok(connected[0].to_string()),
        None => Err("先选择一台设备"),
    }
}

/// 拖拽区文案要知道的现状（一行几个事实，免得参数列表越加越长）。
#[derive(Clone, Copy, Debug, Default)]
pub struct Zone<'a> {
    /// 后台服务现在是什么状态（关着的时候，说什么设备都是空话）。
    pub service: crate::model::state::ServiceState,
    /// 附近发现了几台设备（**不等于可以发**：还要连上）。
    pub device_count: usize,
    /// **已连接**（环上）的台数——只有它是可发名单的规模。
    pub connected: usize,
    /// 已选中的那台名字（只可能是已连接的）。
    pub selected: Option<&'a str>,
    /// 唯一那台**已连接**的名字（一台免选：它就是目标）。
    pub only: Option<&'a str>,
}

impl<'a> Zone<'a> {
    /// 这批文件实际会发给谁（没选时，只有一台已连接也算有目标）。
    pub fn target(&self) -> Option<&'a str> {
        self.selected.or(self.only)
    }
}
