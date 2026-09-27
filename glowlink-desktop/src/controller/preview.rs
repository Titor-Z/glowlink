//! Controller：**预览实现**——没有守护进程时界面照样画得出来。
//!
//! 存在的理由不是「测试方便」，而是产品的：UI 阶段要在没有第二台设备、
//! 甚至没有守护进程的情况下反复比对设计稿（导出的 PNG 就是这么来的）。
//! 有了它，界面可以在**离线**状态下进入任意状态并被截图，
//! 而不必为了看一眼传输卡去真的传一个文件。

use glowlink_ipc::row::PairingState;
use glowlink_core::DeviceKind;
use glowlink_core::model::error::{GlowError, Result};

use crate::controller::source::{
    DeviceSource, DeviceView, PairingSource, PairingView, TransferDirection, TransferRow,
    TransferSource, TransferState,
};

/// 固定的一组「附近设备」：与设计稿上的三台一一对应。
pub fn preview_devices() -> Vec<DeviceView> {
    vec![
        DeviceView {
            id: "preview-phone".into(),
            name: "Pixel 9".into(),
            kind: DeviceKind::Phone,
            trusted: false,
        },
        DeviceView {
            id: "preview-tablet".into(),
            name: "iPad mini".into(),
            kind: DeviceKind::Tablet,
            trusted: false,
        },
        DeviceView {
            id: "preview-desktop".into(),
            name: "桌面主机".into(),
            kind: DeviceKind::Desktop,
            trusted: false,
        },
    ]
}

/// 预览设备表。
#[derive(Clone, Debug, Default)]
pub struct PreviewSource {
    devices: Vec<DeviceView>,
}

impl PreviewSource {
    /// 空设备表：用来比对「空态」。
    pub fn empty() -> Self {
        PreviewSource { devices: Vec::new() }
    }

    /// 三台设备：用来比对「发现态」。
    pub fn discovered() -> Self {
        PreviewSource { devices: preview_devices() }
    }

    /// 任意台数（含第 4 台以上的重叠槽位行为）。
    pub fn with_devices(devices: Vec<DeviceView>) -> Self {
        PreviewSource { devices }
    }
}

impl DeviceSource for PreviewSource {
    fn devices(&self) -> Vec<DeviceView> {
        self.devices.clone()
    }

    fn refresh(&self) -> Result<()> {
        // 预览数据是固定的：刷新不做任何事，但也**不该报错**
        Ok(())
    }
}

/// 预览配对：`start` 返回一个固定的码，`pending` 给一条假请求。
#[derive(Clone, Debug, Default)]
pub struct PreviewPairing {
    pending: Vec<PairingView>,
}

impl PreviewPairing {
    /// 有对端在请求配对。
    pub fn with_request() -> Self {
        PreviewPairing {
            pending: vec![PairingView {
                token: "001".into(),
                peer: "iPad mini".into(),
                code: "518892".into(),
                remaining: 3,
                state: PairingState::WaitingUser,
            }],
        }
    }
}

impl PairingSource for PreviewPairing {
    fn start(&self, _device_id: &str) -> Result<()> {
        // 预览里配对是固定的：立即成功，码从 `outgoing()` 拿（与 IPC 实现同一口径）
        Ok(())
    }

    fn outgoing(&self) -> Vec<PairingView> {
        vec![PairingView {
            token: "001".into(),
            peer: "Pixel 9".into(),
            code: "518892".into(),
            remaining: 3,
                state: PairingState::WaitingUser,
        }]
    }

    fn pending(&self) -> Vec<PairingView> {
        self.pending.clone()
    }

    fn approve(&self, token: &str, code: &str) -> Result<()> {
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
            return Err(GlowError::Usage("校验码是 6 位数字".into()));
        }
        let _ = token;
        Ok(())
    }

    fn reject(&self, _token: &str) -> Result<()> {
        Ok(())
    }
}

/// 预览传输：给一张固定的进度卡。
#[derive(Clone, Debug, Default)]
pub struct PreviewTransfer {
    transfers: Vec<TransferRow>,
}

impl PreviewTransfer {
    pub fn idle() -> Self {
        PreviewTransfer { transfers: Vec::new() }
    }

    /// 传到大半的那条（设计稿那张卡片的形态：842MB 的录屏，已传 480MB）。
    pub fn halfway() -> Self {
        PreviewTransfer {
            transfers: vec![TransferRow {
                token: "001".into(),
                direction: TransferDirection::Send,
                name: "屏幕录制 2026-09-15 040220.mp4".into(),
                peer: "iPad mini".into(),
                done: 480_000_000,
                total: 842_400_000,
                state: TransferState::Running,
                note: String::new(),
            }],
        }
    }

    /// 刚收完的那条（完成态：卡片留着，等用户自己关）。
    pub fn finished_receive() -> Self {
        PreviewTransfer {
            transfers: vec![TransferRow {
                token: "002".into(),
                direction: TransferDirection::Receive,
                name: "设计交付包.zip".into(),
                peer: "桌面主机".into(),
                done: 12_500_000,
                total: 12_500_000,
                state: TransferState::Done,
                note: String::new(),
            }],
        }
    }
}

impl TransferSource for PreviewTransfer {
    fn transfers(&self) -> Vec<TransferRow> {
        self.transfers.clone()
    }

    fn decide(&self, _token: &str, _accept: bool) -> Result<()> {
        Ok(())
    }

    fn pending(&self) -> Vec<crate::controller::source::PendingView> {
        Vec::new()
    }
}
