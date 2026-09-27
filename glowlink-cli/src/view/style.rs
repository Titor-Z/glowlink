//! 终端着色：只在 TTY 下启用，并尊重 `NO_COLOR`。
//!
//! 三条硬规则（见 AGENTS.md「信息暴露与输出规范」）：
//! 1. **只有 TTY 才上色**：管道、重定向、CI 里自动降级为纯文本；
//! 2. **尊重 `NO_COLOR`**：这个环境变量存在即不上色（社区约定，老手会设它）；
//! 3. **颜色不携带语义**：颜色只是装饰，去掉颜色后信息依然完整、可 grep、可被脚本解析。
//!
//! 因此这里的所有方法都只是「把文本包上转义序列」，绝不改变文本本身。

use std::io::IsTerminal;
use std::sync::OnceLock;

/// 前景色与样式（ANSI SGR）。
const RESET: &str = "\x1b[0m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

/// 输出级别。级别只决定「前缀与颜色」，不改变信息内容。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// 普通信息（状态、列表、结果）：无前缀。
    Plain,
    /// 提示用户做什么：`[提示]`。
    Notice,
    /// 后台行为说明（自动顺延端口之类）：`[信息]`。
    Info,
    /// 功能可能受限但可继续：`[警告]`。
    Warn,
}

impl Level {
    /// 按已带前缀的文案反推级别。
    ///
    /// 提示语散落在核心库（`decide_port` 的顺延说明等）与 CLI 各处，让它们继续
    /// 只产出文本，由这里统一决定用哪个级别上色，避免为了上色去改一堆返回类型。
    pub fn from_prefix(line: &str) -> Level {
        if line.starts_with("[警告]") {
            Level::Warn
        } else if line.starts_with("[信息]") {
            Level::Info
        } else if line.starts_with("[提示]") {
            Level::Notice
        } else {
            Level::Plain
        }
    }

    /// 级别前缀。**即使不上色也要有前缀**，否则信息会因环境而丢失。
    pub fn prefix(self) -> &'static str {
        match self {
            Level::Plain => "",
            Level::Notice => "[提示] ",
            Level::Info => "[信息] ",
            Level::Warn => "[警告] ",
        }
    }
}

pub struct Style {
    colored: bool,
}

impl Style {
    /// 按「是否 TTY」「是否设了 NO_COLOR」决定要不要上色（纯函数，便于测试）。
    pub fn from_flags(is_tty: bool, no_color: bool) -> Self {
        Style { colored: is_tty && !no_color }
    }

    /// 按当前进程的真实环境判断。
    pub fn detect() -> Self {
        Style::from_flags(
            std::io::stdout().is_terminal(),
            std::env::var_os("NO_COLOR").is_some(),
        )
    }

    /// 明确不上色（测试与 `--no-color` 用）。
    pub fn plain() -> Self {
        Style::from_flags(false, false)
    }

    pub fn colored(&self) -> bool {
        self.colored
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.colored && !text.is_empty() {
            format!("{code}{text}{RESET}")
        } else {
            text.to_string()
        }
    }

    /// 错误（红）。
    pub fn error(&self, text: &str) -> String {
        self.wrap(RED, text)
    }

    /// 成功（绿）。
    pub fn ok(&self, text: &str) -> String {
        self.wrap(GREEN, text)
    }

    /// 警告（黄）。
    pub fn warn(&self, text: &str) -> String {
        self.wrap(YELLOW, text)
    }

    /// 信息（青）。
    pub fn info(&self, text: &str) -> String {
        self.wrap(CYAN, text)
    }

    /// 强调（粗体），用于设备名、校验码这类用户要盯着看的内容。
    pub fn strong(&self, text: &str) -> String {
        self.wrap(BOLD, text)
    }

    /// 次要内容（暗色），用于表头、提示尾巴。
    pub fn dim(&self, text: &str) -> String {
        self.wrap(DIM, text)
    }

    /// 只上色，**不改动一个字符**：文本已经带前缀时用它。
    pub fn colorize(&self, level: Level, text: &str) -> String {
        match level {
            Level::Warn => self.warn(text),
            Level::Info | Level::Notice => self.info(text),
            Level::Plain => text.to_string(),
        }
    }

    /// 上色，并在**缺失时**补上级别前缀（已经有了就不重复加）。
    ///
    /// 前缀是信息的一部分、颜色只是装饰，所以补前缀这件事与是否上色无关。
    pub fn labeled(&self, level: Level, text: &str) -> String {
        let line = if text.starts_with(level.prefix()) {
            text.to_string()
        } else {
            format!("{}{text}", level.prefix())
        };
        self.colorize(level, &line)
    }
}

static GLOBAL: OnceLock<Style> = OnceLock::new();

/// 进程级样式（首次调用时按环境判定并缓存）。
pub fn style() -> &'static Style {
    GLOBAL.get_or_init(Style::detect)
}
