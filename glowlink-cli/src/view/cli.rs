//! 视图层：终端交互实现。
//!
//! 核心库只定义 [`TransferView`] 接口，终端表现全部收敛在这里。

use std::io::{self, Write};

use glowlink_core::controller::transfer::TransferView;

pub struct CliView {
    auto_accept: bool,
}

impl CliView {
    pub fn new(auto_accept: bool) -> Self {
        CliView { auto_accept }
    }
}

impl TransferView for CliView {
    fn confirm_receive(&self, peer: &str, name: &str, size: u64) -> bool {
        if self.auto_accept {
            return true;
        }
        print!(
            "\n来自 {peer} 的文件「{name}」（{}），接收？[y/N] ",
            human_size(size)
        );
        let _ = io::stdout().flush();
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
    }

    fn progress(&self, label: &str, done: u64, total: u64) {
        let percent = if total == 0 {
            100
        } else {
            (done.saturating_mul(100) / total).min(100)
        };
        print!(
            "\r{label}：{percent:>3}%  {} / {}",
            human_size(done),
            human_size(total)
        );
        let _ = io::stdout().flush();
        if done >= total {
            println!();
        }
    }
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
