//! Model：配对码输入（6 位数字）。
//!
//! 为什么是个「纯状态机」而不是直接在界面里 if：这是**认证材料**的输入，
//! 规则必须能被测试钉住——多收一位、少收一位、退格退错、把非数字吃进去，
//! 都会让用户输完却配不上，而且现象只有「配不上」一条，极难查。
//!
//! 口径（与设计稿的 6 格一致）：
//! - 只吃 `0`..`9`（其余按键**明确忽略**，不报错、不计数）；
//! - 满 6 位之后不再吃（不许第 7 位把前面顶掉）；
//! - 退格只在非空时生效；
//! - 输入**不做任何校验**（对不对由对端与守护进程说了算），
//!   这里只管「用户敲了什么」。

/// 校验码位数（协议固定 6 位，见 `PairingSession` 的派生）。
pub const CODE_LEN: usize = 6;

/// 用户正在输入的 6 位码。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeInput {
    digits: String,
}

impl CodeInput {
    pub fn new() -> Self {
        Self::default()
    }

    /// 敲入一个字符：只认 `0`..`9`，且没满。
    ///
    /// 返回是否真的收下了（界面据此决定要不要重绘/给反馈）。
    pub fn push(&mut self, ch: char) -> bool {
        if !ch.is_ascii_digit() || self.is_complete() {
            return false;
        }
        self.digits.push(ch);
        true
    }

    /// 退格（空的时候什么都不做）。
    pub fn backspace(&mut self) -> bool {
        self.digits.pop().is_some()
    }

    /// 清空（换一次配对、或输错之后要重来）。
    pub fn clear(&mut self) {
        self.digits.clear();
    }

    /// 已输入的数字。
    pub fn digits(&self) -> &str {
        &self.digits
    }

    /// 六格都填满了吗（填满才允许提交）。
    pub fn is_complete(&self) -> bool {
        self.digits.len() >= CODE_LEN
    }

    /// 当前光标所在格（0..6，6 = 已填满；界面用它画「正在这一格」）。
    pub fn cursor(&self) -> usize {
        self.digits.len().min(CODE_LEN)
    }

    /// 六格的显示内容：填过的给数字，没填的给空串。
    pub fn cells(&self) -> [String; CODE_LEN] {
        let mut cells: [String; CODE_LEN] = Default::default();
        for (index, ch) in self.digits.chars().enumerate().take(CODE_LEN) {
            cells[index] = ch.to_string();
        }
        cells
    }
}
