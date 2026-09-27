//! 视图层：终端交互实现。
//!
//! 核心库只定义 [`TransferView`] 接口，终端表现全部收敛在这里。

use std::io::{self, IsTerminal, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glowlink_core::controller::transfer::TransferView;
use glowlink_core::patterns::secure::SessionSlot;

use crate::keys::TrustFile;

/// 进度条宽度（字符数）。
const BAR_WIDTH: usize = 24;
/// 进度刷新间隔：太密会刷屏，太疏会显得卡住。
const REFRESH: Duration = Duration::from_millis(100);

pub struct CliView {
    /// 是否对**已信任**设备免确认自动接收。
    auto_accept: bool,
    /// 本次连接的会话信息（指纹 + 校验码）；发起方用于打印校验码。
    session: Option<SessionSlot>,
    /// 用户确认配对后写入这里；`None` 表示这次运行不记信任（例如仅发送）。
    trust: Option<Arc<TrustFile>>,
    /// 进度状态：同一时刻只有一个传输在进行，用一把锁保护即可。
    state: Mutex<Option<Progress>>,
}

struct Progress {
    /// 上一次绘制时的进度，用于计算瞬时速率。
    last_done: u64,
    last_draw: Instant,
    started: Instant,
    drawn: bool,
}

impl CliView {
    /// 简单视图：只用于发送侧与无配对的场景。
    pub fn new(auto_accept: bool) -> Self {
        CliView {
            auto_accept,
            session: None,
            trust: None,
            state: Mutex::new(None),
        }
    }

    /// 带加密会话信息的视图（接收侧用它显示指纹与校验码、并在确认后记住设备）。
    pub fn with_session(
        auto_accept: bool,
        session: SessionSlot,
        trust: Arc<TrustFile>,
    ) -> Self {
        CliView {
            auto_accept,
            session: Some(session),
            trust: Some(trust),
            state: Mutex::new(None),
        }
    }

    fn session_info(&self) -> Option<glowlink_core::patterns::secure::SessionInfo> {
        let slot = self.session.as_ref()?;
        slot.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl TransferView for CliView {
    fn confirm_receive(&self, peer: &str, name: &str, size: u64) -> bool {
        let info = self.session_info();
        // 已信任设备 + 允许自动接收 ⇒ 不问
        if let Some(info) = &info {
            if info.trusted && self.auto_accept {
                println!(
                    "\n已信任设备「{}」（{}）发来「{name}」，自动接收。",
                    info.peer_fingerprint, peer
                );
                return true;
            }
        }

        println!("\n来自 {peer} 的文件「{name}」（{}）", human_size(size));
        let untrusted = match &info {
            Some(info) if !info.trusted => {
                // 用户要把这两个跟发送方屏幕上显示的核对一遍
                println!(
                    "  设备指纹 {}    校验码 **{}**（请与发送方屏幕核对）",
                    info.peer_fingerprint, info.code
                );
                true
            }
            Some(_) => {
                println!("  已信任设备（指纹 {}）", info.as_ref().unwrap().peer_fingerprint);
                false
            }
            None => {
                println!("  [警告] 没有加密会话信息，无法核对来源");
                true
            }
        };
        print!("接收？[y/N] ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        let accepted = matches!(line.trim().to_lowercase().as_str(), "y" | "yes");

        // 用户既然确认了这台设备，就记住它，下次免确认
        if accepted && untrusted {
            if let (Some(info), Some(trust)) = (&info, self.trust.as_ref()) {
                match trust.trust(&info.peer_fingerprint, peer) {
                    Ok(()) => println!(
                        "已记住设备 {}（{}），下次免确认。",
                        info.peer_fingerprint, peer
                    ),
                    Err(e) => eprintln!("写入信任列表失败：{e}"),
                }
            }
        }
        accepted
    }

    fn progress(&self, label: &str, done: u64, total: u64) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());

        // 新的一轮传输（偏移回退或首次进入）：重建状态并另起一行。
        let fresh = match state.as_ref() {
            None => true,
            Some(p) => done < p.last_done,
        };
        let progress = if fresh {
            if state.as_ref().is_some_and(|p| p.drawn) {
                println!();
            }
            *state = Some(Progress {
                last_done: 0,
                last_draw: now,
                started: now,
                drawn: false,
            });
            state.as_mut().expect("刚写入")
        } else {
            state.as_mut().expect("已存在")
        };

        let finished = done >= total;
        // 限流：未结束且距上次绘制不足间隔就跳过；结束那一次一定要画。
        if !finished && progress.drawn && now.duration_since(progress.last_draw) < REFRESH {
            return;
        }

        let elapsed = now.duration_since(progress.started).as_secs_f64();
        let speed = if elapsed > 0.0 { done as f64 / elapsed } else { 0.0 };
        let eta = if finished {
            0
        } else if speed > 0.0 {
            ((total.saturating_sub(done) as f64 / speed).ceil()) as u64
        } else {
            0
        };
        let eta_text = if finished {
            "0s".to_string()
        } else if speed > 0.0 {
            human_duration(eta)
        } else {
            "--".to_string()
        };

        let line = format!(
            "{} {} {:>3}%  {} / {}  {}  ETA {}{}",
            label,
            bar(done, total),
            percent(done, total),
            human_size(done),
            human_size(total),
            human_speed(speed),
            eta_text,
            if finished { "  完成" } else { "" }
        );

        if io::stdout().is_terminal() {
            // 终端：原地刷新。用 \x1b[K 清掉上一次的残留字符。
            print!("\r{line}\x1b[K");
            progress.drawn = true;
        } else if finished || !progress.drawn {
            // 非终端（重定向 / CI / 后台）：只输出整行文本，避免日志里塞满 \r
            println!("{line}");
            progress.drawn = true;
        }
        progress.last_done = done;
        progress.last_draw = now;
        if finished {
            if io::stdout().is_terminal() {
                println!();
            }
            *state = None;
        }
        let _ = io::stdout().flush();
    }
}

fn percent(done: u64, total: u64) -> u32 {
    if total == 0 {
        100
    } else {
        ((done.saturating_mul(100) / total) as u32).min(100)
    }
}

/// 进度条：`████████░░░░░░░░`，总宽固定，不随百分比抖动。
fn bar(done: u64, total: u64) -> String {
    let filled = if total == 0 {
        BAR_WIDTH
    } else {
        let ratio = (done as f64 / total as f64).clamp(0.0, 1.0);
        // 未完成时至少留一格空格，让进度条看起来是在推进而不是已满
        let raw = (ratio * BAR_WIDTH as f64).round() as usize;
        if ratio < 1.0 && raw >= BAR_WIDTH {
            BAR_WIDTH - 1
        } else {
            raw
        }
    };
    let filled = filled.min(BAR_WIDTH);
    format!("{}{}", "█".repeat(filled), "░".repeat(BAR_WIDTH - filled))
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

/// 速率：固定 MB/s 单位，便于不同量级之间直接比较。
pub fn human_speed(bytes_per_sec: f64) -> String {
    format!("{:.1} MB/s", bytes_per_sec / (1024.0 * 1024.0))
}

/// 格式化剩余时间（秒）。
pub fn human_duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h{:02}m", seconds / 3600, (seconds % 3600) / 60)
    }
}
