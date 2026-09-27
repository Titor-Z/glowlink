//! 视图层：终端交互实现。
//!
//! 核心库只定义 [`TransferView`] 接口，终端表现全部收敛在这里。

use std::io::{self, IsTerminal, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glowlink_core::controller::transfer::TransferView;
use glowlink_core::patterns::secure::SessionSlot;

use crate::keys::TrustFile;
use crate::view::style;

/// 进度条宽度（字符数）。
const BAR_WIDTH: usize = 24;
/// 进度刷新间隔：太密会刷屏，太疏会显得卡住。
const REFRESH: Duration = Duration::from_millis(100);

/// 发送方握手成功后要打印的一行会话信息。
///
/// 校验码**任何情况下都要打印**：本机信任对方，不代表对方也信任本机——
/// 对方可能正因为「不认识本机」而要求核对，藏起来人就无从对照了。
pub fn session_banner(target: &str, fingerprint: &str, code: &str, trusted: bool) -> String {
    let trust_note = if trusted {
        "（已信任）"
    } else {
        ""
    };
    let code_note = if trusted {
        "（本机已信任对方，若对方要求核对请出示此码）"
    } else {
        ""
    };
    format!(
        "已与 {target} 建立加密连接：对端指纹 {fingerprint}{trust_note}\n\
         本次会话校验码 **{code}** ← 请与接收方屏幕上显示的核对{code_note}"
    )
}

/// 接收方展示的「来源」行：指纹与校验码。
///
/// 同样**已信任也要显示校验码**（对称的核对需求）。
pub fn source_line(fingerprint: &str, code: &str, trusted: bool) -> String {
    if trusted {
        format!(
            "  已信任设备（指纹 {fingerprint}）    校验码 **{code}**\
             （本机无需核对；若发送方要求核对请出示此码）"
        )
    } else {
        format!("  设备指纹 {fingerprint}    校验码 **{code}**（请与发送方屏幕核对）")
    }
}

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
    /// 上一次绘制时的进度，用于识别「新的一轮传输」。
    last_done: u64,
    last_draw: Instant,
    drawn: bool,
    /// 滑动窗口测速。
    meter: SpeedMeter,
    /// 非终端下已经打过的百分比档（每 10% 一行，日志不至于刷屏又能看出进度）。
    bucket: u32,
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
                println!("{}", source_line(&info.peer_fingerprint, &info.code, false));
                true
            }
            Some(info) => {
                // 已信任也要把码显示出来：发送方可能正要求核对（本机信任对方 ≠ 对方信任本机）
                println!("{}", source_line(&info.peer_fingerprint, &info.code, true));
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
                drawn: false,
                meter: SpeedMeter::new(now),
                bucket: 0,
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

        let speed = progress.meter.observe(done, now);
        let eta_text = match eta_seconds(done, total, speed) {
            Some(seconds) => human_duration(seconds),
            None => "--".to_string(),
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
        } else {
            // 非终端（重定向 / CI / 后台）：整行输出，避免日志里塞满 \r；
            // 频率压到「每跨一个 10% 档 + 第一次 + 完成」——既能看出进度，又不刷屏。
            let bucket = percent(done, total) / 10;
            if finished || !progress.drawn || bucket > progress.bucket {
                println!("{line}");
                progress.drawn = true;
                progress.bucket = bucket;
            }
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
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(BAR_WIDTH - filled));
    // 上色只是装饰：管道里（非 TTY）拿到的是同一串字符，宽度也不变
    style().ok(&bar)
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
/// 测速窗口：每秒重算一次。
///
/// 用「窗口内的增量」而不是「全程平均」：传到一半掉速（对方磁盘慢、Wi-Fi 抖动）时，
/// 全程平均会长时间停在旧的高值上，ETA 也就跟着骗人。
pub const SPEED_WINDOW: Duration = Duration::from_secs(1);

/// 速度采样器（纯状态 + 纯算术，可单测）。
pub struct SpeedMeter {
    started: Instant,
    /// 上一次窗口采样的进度与时刻。
    sampled_done: u64,
    sampled_at: Instant,
    /// 最近一次窗口测得的速率。
    measured: f64,
    /// 是否已经完成过至少一次窗口采样。
    ///
    /// 有它才能区分「还没测过（退回全程平均）」与「测出来就是 0（卡住了）」——
    /// 少了这个标记，卡住的传输会被全程平均悄悄粉饰成一个好看的旧速度。
    sampled: bool,
}

impl SpeedMeter {
    pub fn new(now: Instant) -> Self {
        SpeedMeter { started: now, sampled_done: 0, sampled_at: now, measured: 0.0, sampled: false }
    }

    /// 记录一次进度，返回当前速率（字节/秒）。
    ///
    /// 窗口未满时退回全程平均（否则传输刚开始那几秒会显示 0，看着像卡住）；
    /// 窗口满了就以窗口为准，**包括掉到 0 的情况**——那正是要如实报出来的。
    pub fn observe(&mut self, done: u64, now: Instant) -> f64 {
        let window = now.duration_since(self.sampled_at).as_secs_f64();
        if window >= SPEED_WINDOW.as_secs_f64() {
            let delta = done.saturating_sub(self.sampled_done) as f64;
            self.measured = delta / window;
            self.sampled_done = done;
            self.sampled_at = now;
            self.sampled = true;
        }
        if self.sampled {
            self.measured
        } else {
            let elapsed = now.duration_since(self.started).as_secs_f64();
            if elapsed > 0.0 {
                done as f64 / elapsed
            } else {
                0.0
            }
        }
    }
}

/// 剩余秒数；无法估算时 `None`。
pub fn eta_seconds(done: u64, total: u64, speed: f64) -> Option<u64> {
    // 总量未知（例如 0 字节文件）时不给估计：说「剩 0 秒」是假话
    if total == 0 {
        return None;
    }
    if done >= total {
        return Some(0);
    }
    if speed <= 0.0 {
        return None;
    }
    Some(((total - done) as f64 / speed).ceil() as u64)
}

/// 速度：小数值不硬套 MB/s，否则「0.0 MB/s」看着像卡死。
pub fn human_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1024.0 * 1024.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.1} MB/s", bytes_per_sec / (1024.0 * 1024.0))
    }
}

/// 多文件发送时给每个文件加个序号；只有一个文件时不加（省得啰嗦）。
pub fn file_index_label(index: usize, total: usize, name: &str) -> Option<String> {
    (total > 1).then(|| format!("[{}/{}] {name}", index + 1, total))
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
