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
    /// `target` 是**已选中的目标设备名**；`None` 表示还没选。
    ///
    /// 三句话都照抄设计稿（`glowlink.js`）：
    /// 「拖拽文件到这里」/「拖到此处 → 甲机」/「先选择一台设备」。
    /// 注意拒绝时**不显示目标名**：那时的问题不是「发给谁」而是「还没选」，
    /// 写上目标名反而让人以为已经选好了。
    pub fn note(self, target: Option<&str>) -> String {
        match self {
            DropState::Denied(reason) => reason.to_string(),
            DropState::Idle | DropState::Over => match target {
                Some(name) => format!("拖到此处 → {name}"),
                None => "拖拽文件到这里".to_string(),
            },
        }
    }

    /// 是不是「一次失败的投放」——界面据此决定要不要抖一下。
    pub fn is_denied(self) -> bool {
        matches!(self, DropState::Denied(_))
    }
}

/// 这批设备下，往拖拽区扔文件能不能发出去。
///
/// - `Ok(None)`：可以发；目标是**已选中的那台**。
/// - `Ok(Some(名字))`：可以发；只有一台设备，不必先选，直接发给它
///   （用户拍板的语义：一台就免选，多台才要求先点选）。
/// - `Err(原因)`：发不出去，原因是给人看的那句话。
///
/// 这条判定单独抽出来：它是**用户看得见的规则**，而事件处理那段测不动。
pub fn accept_target(
    device_names: &[&str],
    selected: Option<&str>,
) -> std::result::Result<Option<String>, &'static str> {
    // 一台都没有：不是「没选」，是根本没得选——文案要说得出来
    if device_names.is_empty() {
        return Err("附近还没有设备");
    }
    match selected {
        // 选中的还在：就用它
        Some(name) if device_names.contains(&name) => Ok(Some(name.to_string())),
        // 选了但它已经下线：**不许悄悄换一台发出去**——
        // 用户以为发给了甲、实际发给了乙，是最难发现的那种错。
        Some(_) => Err("选中的设备已不在了"),
        // 没选：只有一台就免选（用户拍板的语义），多台要求先点选（设计稿文案）
        None if device_names.len() == 1 => Ok(Some(device_names[0].to_string())),
        None => Err("先选择一台设备"),
    }
}
