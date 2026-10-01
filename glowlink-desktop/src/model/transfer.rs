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
        // 排队中：还没有任何字节在动，写 0% 会让人以为「开始就卡住了」
        TransferState::Queued => "等待中".to_string(),
        TransferState::Running => format!("{}%", format::percent(row.done, row.total)),
        TransferState::Done => "已完成".to_string(),
        TransferState::Rejected => "被拒绝".to_string(),
        TransferState::Failed => "失败".to_string(),
        TransferState::Canceled => "已取消".to_string(),
    };
    // 元信息行（用户定的版式）：`2.4 MB / 3.9 MB · 剩余 4 秒`。
    // **不写方向**（卡片本来就只放一条传输，方向由发起方自己心里清楚），
    // 也不写速度（剩余时间已经是速度的结论，两个一起放是重复）。
    let meta = match row.state {
        // 排队中：只说排到了第几个（数字要准——它确实还没开始动）
        TransferState::Queued => "前面的文件传完就开始".to_string(),
        TransferState::Running => {
            let mut text = format!(
                "{} / {}",
                format::human_size(row.done),
                format::human_size(row.total)
            );
            if let Some(eta) = format::eta_seconds(row.total.saturating_sub(row.done), speed) {
                text.push_str(&format!(" · 剩余 {}", format::human_eta(eta)));
            }
            text
        }
        TransferState::Done => format!("{} · {direction}完成", format::human_size(row.total)),
        // 结束态一律**如实转述守护进程给的原因**（「磁盘没空间」与「对方说不要」
        // 对用户是两件事，不能被抹成同一句「未完成」）
        TransferState::Rejected | TransferState::Failed | TransferState::Canceled => {
            if row.note.is_empty() {
                format!("{} · 未完成", format::human_size(row.total))
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

/// 交替高亮的周期（秒）：一次「本机亮 → 对端亮」。
pub const PULSE_PERIOD_SECS: f32 = 1.2;

/// 传输中的交替高亮：返回 `(本机强度, 对端强度)`，都在 0..1。
///
/// 为什么不画连线：环上的气泡本来就贴在本机盘边上，一条线几乎没有信息量；
/// 而「光在这两台之间来回跳」既说清了**哪两台**在传，又说清了**还在传**——
/// 灯一停就是传完了。两个半周期各一条升余弦包络（0→1→0），所以不会有硬闪。
pub fn pulse(phase: f32) -> (f32, f32) {
    let cycle = phase.rem_euclid(1.0);
    let bump = |local: f32| (1.0 - (local * std::f32::consts::TAU).cos()) * 0.5;
    if cycle < 0.5 {
        (bump(cycle * 2.0), 0.0)
    } else {
        (0.0, bump((cycle - 0.5) * 2.0))
    }
}

// ── 批次：一次投放 = 一批（守护进程按顺序一个文件一个文件地发） ──

/// 一次投放进去的那几个文件。
///
/// 卡片只有一张（设计稿就一个槽位），所以「第几个 / 共几个」靠它算：
/// 当前在传的那个 = 这批里**还没结束**的第一个。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    /// 发给谁（对端显示名——同时是「哪几行属于这一批」的判据）。
    pub peer: String,
    /// 这批的文件名（拖进来的顺序）。
    pub names: Vec<String>,
}

impl Batch {
    pub fn total(&self) -> usize {
        self.names.len()
    }

    /// 这一批结束了吗（每个文件都落了终态）。
    pub fn is_finished(&self, rows: &[TransferRow]) -> bool {
        self.names.iter().all(|name| {
            rows.iter().any(|row| {
                &row.name == name && row.direction == TransferDirection::Send && row.state.is_terminal()
            })
        })
    }

    /// 这一批的总字节数（各行报的大小的和；对不上就只算认得出来的）。
    pub fn total_bytes(&self, rows: &[TransferRow]) -> u64 {
        self.names
            .iter()
            .filter_map(|name| {
                rows.iter()
                    .find(|row| &row.name == name && row.direction == TransferDirection::Send)
                    .map(|row| row.total)
            })
            .sum()
    }

    /// `(当前第几个, 总数)`：当前 = 这批里第一个还没结束的（都结束了就报总数）。
    pub fn position(&self, rows: &[TransferRow]) -> (usize, usize) {
        let total = self.total().max(1);
        let finished = self
            .names
            .iter()
            .filter(|name| {
                rows.iter().any(|row| {
                    &row.name == *name && row.direction == TransferDirection::Send && row.state.is_terminal()
                })
            })
            .count();
        (finished.min(total - 1) + 1, total)
    }

    /// 只有多个文件时才需要「2/5」这种位置标签（单个文件写它反而多余）。
    pub fn label(&self, rows: &[TransferRow]) -> Option<String> {
        if self.total() < 2 {
            return None;
        }
        let (index, total) = self.position(rows);
        Some(format!("{index}/{total}"))
    }

    /// 这一批属于哪个对端（取消整批时按它筛行）。
    pub fn cancel_targets<'a>(&self, rows: &'a [TransferRow]) -> Vec<&'a TransferRow> {
        rows.iter()
            .filter(|row| {
                row.direction == TransferDirection::Send
                    && row.peer == self.peer
                    && !row.state.is_terminal()
            })
            .collect()
    }
}

// ── 接收侧待确认（别人要发文件给你） ──

/// 接受是不可轻率的动作：**码就在眼前**才谈得上核对。
///
/// 这句话必须和 6 位码同屏出现——发送方屏幕上也有同一个码
/// （「已信任」不隐藏码，两侧都要无条件显示）。
pub const PENDING_HINT: &str = "请与发送方屏幕上的校验码核对，一致再接受";

/// 还有几个请求在排队等拍板（只有一个时不写——那是废话）。
pub fn pending_queue_note(total: usize) -> Option<String> {
    (total > 1).then(|| format!("另有 {} 个请求在等", total - 1))
}
