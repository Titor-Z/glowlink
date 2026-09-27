//! 待确认接收队列：token + 多槽位 + 超时自动拒绝。
//!
//! 为什么不是单槽：单槽下第二个发送方会静默挂在 TCP backlog 里，
//! 既没有提示也没有拒绝，对发送方来说就是「卡死」，体验是灾难。
//! 所以每个到达的 OFFER 各占一个槽位、各拿一个短 token，
//! 由接收线程各自阻塞等待，`glowlink pending` 能看到全部，
//! `glowlink accept <token>` 按号下决定，超时则各自自动拒绝。

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use glowlink_core::model::error::{GlowError, Result};

/// 一个待确认的接收请求。
///
/// 带上**指纹与校验码**：未配对的设备必须让用户先把校验码与发送方屏幕核对，
/// 确认后才接收，并把指纹写进信任列表（下次免确认）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingOffer {
    pub peer: String,
    pub name: String,
    pub size: u64,
    /// 对端设备指纹。
    pub fingerprint: String,
    /// 6 位校验码（两端相同，供人工比对）。
    pub code: String,
}

/// 队列里的一个槽位：`token` 给用户手敲，`decision` 由 IPC 线程写入。
#[derive(Clone, Debug)]
struct Waiting {
    token: String,
    offer: PendingOffer,
    decision: Option<bool>,
}

#[derive(Default)]
struct Inner {
    /// 单调递增的取号器；只用于生成 token，不和槽位序号绑定。
    issued: u64,
    waiting: Vec<Waiting>,
}

pub struct PendingQueue {
    inner: Mutex<Inner>,
    ready: Condvar,
}

impl Default for PendingQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingQueue {
    pub fn new() -> Self {
        PendingQueue {
            inner: Mutex::new(Inner::default()),
            ready: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 登记请求并阻塞等待决定；超时**自动拒绝**（没人管的时候宁可什么都不收）。
    /// 返回值为是否接受，`token` 通过回调交给调用方打日志/提示。
    pub fn ask<F>(&self, offer: PendingOffer, timeout: Duration, on_register: F) -> bool
    where
        F: FnOnce(&str),
    {
        let deadline = Instant::now() + timeout;
        let mut inner = self.lock();
        let token = register(&mut inner, offer);
        on_register(&token);
        loop {
            if let Some(slot) = inner.waiting.iter().find(|w| w.token == token) {
                if let Some(decision) = slot.decision {
                    inner.waiting.retain(|w| w.token != token);
                    return decision;
                }
            } else {
                // 槽位被外部摘掉了（正常流程不会发生），按拒绝处理
                return false;
            }
            let now = Instant::now();
            if now >= deadline {
                inner.waiting.retain(|w| w.token != token);
                return false;
            }
            let (guard, _) = self
                .ready
                .wait_timeout(inner, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            inner = guard;
        }
    }

    /// 当前所有**尚未落决定**的待确认请求（按登记顺序），供 `PENDING` 列表使用。
    ///
    /// 已落决定的条目要等接收线程醒来才摘除，必须在这里过滤掉：
    /// 否则刚 accept 完的几毫秒内，列表与计数仍把它当成待确认。
    pub fn list(&self) -> Vec<(String, PendingOffer)> {
        self.lock()
            .waiting
            .iter()
            .filter(|w| w.decision.is_none())
            .map(|w| (w.token.clone(), w.offer.clone()))
            .collect()
    }

    pub fn count(&self) -> usize {
        self.lock()
            .waiting
            .iter()
            .filter(|w| w.decision.is_none())
            .count()
    }

    /// 落决定。`token` 为空表示「就一个，直接决定」；
    /// 有多个待确认请求时**必须**给 token，否则容易点错对象。
    pub fn decide(&self, token: Option<&str>, accept: bool) -> Result<String> {
        let mut inner = self.lock();
        // 只能对还没落决定的下手：已决定的条目只是等接收线程来取走
        let undecided: Vec<usize> = inner
            .waiting
            .iter()
            .enumerate()
            .filter(|(_, w)| w.decision.is_none())
            .map(|(index, _)| index)
            .collect();
        let target = match token {
            Some(token) => undecided
                .iter()
                .copied()
                .find(|index| inner.waiting[*index].token == token)
                .ok_or_else(|| GlowError::Usage(format!("没有编号为 {token} 的待确认请求")))?,
            None => match undecided.len() {
                0 => return Err(GlowError::Usage("当前没有待确认的接收请求".into())),
                1 => undecided[0],
                n => {
                    return Err(GlowError::Usage(format!(
                        "有 {n} 个待确认请求，请用 glowlink accept <编号> / reject <编号> 指定，\
                         编号见 glowlink pending"
                    )))
                }
            },
        };
        let token = inner.waiting[target].token.clone();
        inner.waiting[target].decision = Some(accept);
        self.ready.notify_all();
        Ok(token)
    }
}

/// 登记一个槽位并分配当前未被占用的短 token。
fn register(inner: &mut Inner, offer: PendingOffer) -> String {
    let token = loop {
        inner.issued += 1;
        // 三位十进制：够短好敲，且同一时刻几乎不可能撞号
        let candidate = format!("{:03}", inner.issued % 1000);
        if !inner.waiting.iter().any(|w| w.token == candidate) {
            break candidate;
        }
    };
    inner.waiting.push(Waiting { token: token.clone(), offer, decision: None });
    token
}
