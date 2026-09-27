//! Model：传输态的界面逻辑（纯函数 + 纯状态，零 IO 零渲染）。
//!
//! 传输数据来自守护进程的 `TRANSFERS` 行（[`glowlink_ipc::row::TransferRow`]），
//! 这里只做三件事：
//! 1. **速度**：1 秒滑动窗口（与 CLI 同一口径，见 `SpeedMeter`）；
//! 2. **该显示哪一条**：正在跑的优先，其次最近一条（完成的卡片不自动消失，用户自己关）；
//! 3. **卡片上写什么**：全部数字来自真实字节与耗时，绝不编造指标。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use glowlink_ipc::row::{TransferDirection, TransferRow, TransferState};

use crate::model::format;

/// 速度窗口长度（秒）。用滑动窗口而不是全程平均：中途掉速时全程平均会长时间停在旧值上。
pub const SPEED_WINDOW: Duration = Duration::from_secs(1);

/// 采样最小间隔：快照 300ms 一次，界面 60fps——没必要每帧都塞样本。
pub const SPEED_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

/// 1 秒滑动窗口速度计。
///
/// 口径：`(窗口内最新字节 − 窗口内最早字节) / 两点之间的真实耗时`。
/// 窗口未满时用已有的全部样本（短传输不会算出荒唐的速度）。
#[derive(Default, Debug)]
pub struct SpeedMeter {
    samples: VecDeque<(Instant, u64)>,
    /// 上一次采样的时刻（用于最小间隔节流）。
    last: Option<Instant>,
}

impl SpeedMeter {
    /// 采一格。`bytes` 是**累计已传字节**（不是增量）。
    ///
    /// 字节数没变也要采：窗口滑走后速度该掉下来，否则会一直显示旧的峰值。
    pub fn sample(&mut self, bytes: u64, now: Instant) -> f64 {
        let fresh = match self.last {
            Some(last) => now.saturating_duration_since(last) >= SPEED_SAMPLE_INTERVAL,
            None => true,
        };
        if !fresh {
            return self.speed();
        }
        self.last = Some(now);
        self.samples.push_back((now, bytes));
        while let Some((then, _)) = self.samples.front() {
            if now.saturating_duration_since(*then) > SPEED_WINDOW {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        self.speed()
    }

    /// 当前速度（字节/秒）。样本不足时给 0，**不编造**。
    pub fn speed(&self) -> f64 {
        let (Some((first_at, first_bytes)), Some((last_at, last_bytes))) =
            (self.samples.front(), self.samples.back())
        else {
            return 0.0;
        };
        let elapsed = last_at.saturating_duration_since(*first_at).as_secs_f64();
        if elapsed <= 0.0 {
            return 0.0;
        }
        last_bytes.saturating_sub(*first_bytes) as f64 / elapsed
    }

    /// 换了一条传输（或者传输结束）就清空：两条传输的字节数不能相加。
    pub fn reset(&mut self) {
        self.samples.clear();
        self.last = None;
    }
}

/// 该显示哪一条传输（`None` = 卡片不出现）。
///
/// 优先级：**正在跑的**（最近开始的那条）> 最近结束的。
/// 完成的卡片留在屏幕上是有意的（设计稿：「完成后不自动消失，用户自己关」）。
pub fn pick(rows: &[TransferRow]) -> Option<usize> {
    let running = rows
        .iter()
        .rposition(|row| row.state == TransferState::Running);
    running.or_else(|| rows.len().checked_sub(1))
}

/// 卡片上的一行元信息要用的数字。
#[derive(Clone, Debug, PartialEq)]
pub struct CardNumbers {
    /// 进度条目标值 0..1。
    pub fraction: f64,
    /// 「42%」或结束时的替代文案。
    pub percent: String,
    /// 9px 元信息行：方向 + 大小 + 速度 + 剩余。
    pub meta: String,
}

/// 把一行传输变成卡片上的三样东西。
///
/// `speed` 由调用方用 [`SpeedMeter`] 算好传进来（模型不碰时钟）。
pub fn numbers(row: &TransferRow, speed: f64) -> CardNumbers {
    let fraction = if row.total == 0 {
        0.0
    } else {
        (row.done as f64 / row.total as f64).clamp(0.0, 1.0)
    };
    let direction = match row.direction {
        TransferDirection::Send => "发送",
        TransferDirection::Receive => "接收",
    };
    let percent = match row.state {
        TransferState::Running => format!("{}%", format::percent(row.done, row.total)),
        TransferState::Done => "已完成".to_string(),
        TransferState::Rejected => "被拒绝".to_string(),
        TransferState::Failed => "失败".to_string(),
    };
    let meta = match row.state {
        TransferState::Running => {
            let mut text = format!(
                "{direction}中 · {} / {}",
                format::human_size(row.done),
                format::human_size(row.total)
            );
            if speed > 0.0 {
                text.push_str(&format!(" · {}", format::human_speed(speed)));
                if let Some(eta) = format::eta_seconds(row.total.saturating_sub(row.done), speed) {
                    text.push_str(&format!(" · 剩 {}", format::human_eta(eta)));
                }
            }
            text
        }
        TransferState::Done => {
            // 平均速度（完成时用它）：这是「真实字节 / 真实耗时」，不是编出来的倍数
            format!("{direction}完成 · {}", format::human_size(row.total))
        }
        TransferState::Rejected => {
            if row.note.is_empty() {
                "对方拒绝了这次传输".to_string()
            } else {
                row.note.clone()
            }
        }
        TransferState::Failed => {
            if row.note.is_empty() {
                "传输失败".to_string()
            } else {
                row.note.clone()
            }
        }
    };
    CardNumbers { fraction, percent, meta }
}

/// 结束后右下角那个小按钮该是什么（`None` = 没有动作可用）。
///
/// 只有**收下来的**文件才谈得上「打开」：发送方本机本来就有这个文件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// 打开接收目录。
    OpenFolder,
}

pub fn action(row: &TransferRow) -> Option<Action> {
    match (row.state, row.direction) {
        (TransferState::Done, TransferDirection::Receive) => Some(Action::OpenFolder),
        _ => None,
    }
}
