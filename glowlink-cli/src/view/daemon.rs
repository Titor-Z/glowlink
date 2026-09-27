//! 守护进程视图：把「收到文件先问人」搬到没有终端的分离进程里。
//!
//! 守护进程没有 stdin，问不了人，所以：
//! 1. `confirm_receive` 登记待确认请求（**带指纹与 6 位校验码**），
//!    并在 [`PendingQueue`] 上阻塞等待；
//! 2. 用户用 `glowlink list` 看到编号与校验码，用
//!    `glowlink accept <编号>` / `reject <编号>` 经 IPC 落决定；
//! 3. 用户确认后把对端指纹写入信任列表——下次该设备免确认；
//! 4. 超时按拒绝处理——没人管的时候宁可什么都不收。
//!
//! `auto_accept`（`glowlink on --yes`）**只对已信任指纹生效**：
//! 陌生设备即使开了 `--yes` 也必须人工确认，否则「无感」就等于「谁都能塞文件」。

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use glowlink_core::controller::transfer::TransferView;
use glowlink_core::patterns::secure::SessionSlot;
use glowlink_ipc::row::TransferDirection;

use crate::keys::TrustFile;
use crate::pending::{PendingOffer, PendingQueue};
use crate::transfers::{Job, Transfers};
use glowlink_ipc::state::append_log;

/// `glowlink list` 里每条待确认请求的「来源」提示（已信任也要带校验码）。
pub fn pending_hint(fingerprint: &str, code: &str, trusted: bool) -> String {
    if fingerprint.is_empty() {
        return String::new();
    }
    if trusted {
        // 已信任不等于不用核对：发送方那边可能正准备核对，码必须看得见。
        // 同时说清「已信任为什么还问」：免确认由 --yes 控制。
        format!(
            "设备指纹 {fingerprint}（已信任；免确认请用 glowlink on --yes）  校验码 **{code}**"
        )
    } else {
        format!("设备指纹 {fingerprint}  校验码 **{code}**（请与发送方屏幕核对）")
    }
}

pub struct DaemonView {
    queue: Arc<PendingQueue>,
    timeout: Duration,
    /// 本次连接的会话信息（指纹 + 校验码），由 `SecureTransport` 在握手后写入。
    session: SessionSlot,
    trust: Arc<TrustFile>,
    auto_accept: bool,
    /// 传输登记表：接收方向也要能看见进度（不然桌面版只能看“已接收”前的一片空白）。
    transfers: Arc<Transfers>,
    /// 本次接收的登记项（确认接收时才建，之后进度往它上写）。
    job: Mutex<Option<Job>>,
}

impl DaemonView {
    pub fn new(
        queue: Arc<PendingQueue>,
        timeout: Duration,
        session: SessionSlot,
        trust: Arc<TrustFile>,
        auto_accept: bool,
        transfers: Arc<Transfers>,
    ) -> Self {
        DaemonView {
            queue,
            timeout,
            session,
            trust,
            auto_accept,
            transfers,
            job: Mutex::new(None),
        }
    }
}

impl TransferView for DaemonView {
    fn confirm_receive(&self, peer: &str, name: &str, size: u64) -> bool {
        // 没有会话信息 ⇒ 这不是一次加密连接，绝不能按「未知设备」放行
        let Some(info) = self
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            append_log(&format!("拒绝 {peer} 的「{name}」：缺少加密会话信息"));
            return false;
        };
        let (fingerprint, code, trusted) =
            (info.peer_fingerprint.clone(), info.code.clone(), info.trusted);

        // 收到就登记：界面从「待确认」变「正在接收」是同一个事件的两半，
        // 中间不排空档（否则点完确认会有一瞬间什么都没有）。
        let job = self.transfers.start(
            TransferDirection::Receive,
            name.to_string(),
            peer.to_string(),
            size,
        );
        *self.job.lock().unwrap_or_else(|e| e.into_inner()) = Some(job.clone());

        // 已信任 + 开了 --yes ⇒ 无感接收
        if trusted && self.auto_accept {
            append_log(&format!(
                "已信任设备 {fingerprint} 发来「{name}」（{size} 字节），自动接收"
            ));
            return true;
        }

        let offer = PendingOffer {
            peer: peer.to_string(),
            name: name.to_string(),
            size,
            fingerprint: fingerprint.clone(),
            code: code.clone(),
        };
        let hint = if trusted {
            // 日志也要留码：事后排查「这次传输到底跟谁核对过」只有它可查
            format!("已信任设备，校验码 {code}")
        } else {
            format!("未配对设备，校验码 {code}（请与发送方核对）")
        };
        let accepted = self.queue.ask(offer, self.timeout, |token| {
            append_log(&format!(
                "收到 {peer} 的文件「{name}」（{size} 字节），编号 {token}，{hint}；\
                 确认：glowlink accept {token} / reject {token}"
            ));
        });

        if accepted {
            append_log("已确认接收，开始传输");
            // 用户既然确认了，就把这台设备记下来，下次免确认
            if !trusted && !fingerprint.is_empty() {
                if let Err(e) = self.trust.trust(&fingerprint, peer) {
                    append_log(&format!("写入信任列表失败：{e}"));
                } else {
                    append_log(&format!("已记住设备 {fingerprint}（{peer}），下次免确认"));
                }
            }
        } else {
            append_log("未确认（被拒绝或超时），已放弃本次接收");
            job.reject("本机用户未确认（拒绝或超时）");
        }
        accepted
    }

    /// 用户取消了这次接收吗（桌面版的 ✕）。
    fn canceled(&self) -> bool {
        self.job
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|job| job.is_canceled())
            .unwrap_or(false)
    }

    /// 进度写入登记表：守护进程没有终端，日志里刷进度会把有用信息淹掉。
    fn progress(&self, _label: &str, done: u64, total: u64) {
        if let Some(job) = self.job.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            job.update(done, total);
        }
    }
}

impl DaemonView {
    /// 这次接收传完了。
    ///
    /// 接收侧必须自己收尾：只管写 `progress` 而不落终态的话，
    /// 界面上的卡片会永远停在 `running`（实测踩到过）。
    pub fn finish(&self) {
        if let Some(job) = self.job.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            job.finish();
        }
    }

    /// 这次接收失败了（磁盘写满、对端断开……）。
    pub fn fail(&self, note: impl Into<String>) {
        if let Some(job) = self.job.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            job.fail(note);
        }
    }
}
