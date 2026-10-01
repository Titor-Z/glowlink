//! 端口自检握手：判断「发现端口上的监听者是谁」。
//!
//! 场景：本机只能有一个节点绑定发现端口。后来者绑定失败时需要区分几种情况：
//! - **我们自己的节点**：提示版本冲突，必要时可顶替；
//! - **别的程序占用**：友好报错，让用户换端口或关掉它；
//! - **无人应答的残留绑定**：macOS 上进程已退出但端口仍被内核持有，
//!   既不能绑也收不到应答。不能说成"被其它程序占用"，否则纯属误报。
//!
//! 判据不能靠 `lsof`（会漏掉沙盒进程与内核残留，实测有端口被完全不可见的
//! 持有者锁死），也不能只看"端口被占"，而是发一个 [`Frame::Probe`] 到
//! `127.0.0.1`，看对方是否回 [`Frame::Pong`]。

use std::net::UdpSocket;
use std::time::Duration;

use crate::model::error::{GlowError, Result};
use crate::model::packet::{Frame, Probe, VERSION_STR};
use crate::model::ports::{candidate_at, is_port_conflict, DISCOVERY_PORT_SPAN};

/// 单次握手的等待时长。
///
/// 对方发现线程在 `poll` 里阻塞收包（一轮窗口 = `query_interval_secs / 2`，
/// 默认 500ms），窗口缝隙里到达的报文会被内核丢弃 —— 所以靠**多次重发**覆盖窗口，
/// 而不是靠把单次等待拉长。
///
/// 实测口径（本机回环）：探头发出后活着的守护进程是**亚毫秒级**应答，
/// 所以等待时长只决定「**没人应答时**要空等多久」。端口空闲是最常见的情况
/// （服务没在跑时就是这样），原先 4×(600+250) = **3.15 秒**全花在这上面，
/// 用户点开开关后的「怎么还没动静」就是这么来的。现在 3 次 ×350ms + 2×200ms
/// 最坏 **1.45 秒**，而三次重发仍横跨 400ms ≥ 一个收包窗口，覆盖没有变差。
const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(350);
/// 重试次数：UDP 可能丢包，多试几次降低误判。
const ATTEMPTS: usize = 3;
/// 两次尝试之间的最小间隔：错开对方的收包窗口，避免连续扑在同一缝隙里。
const RETRY_GAP: Duration = Duration::from_millis(200);

/// 探测结果。
///
/// 【四态而非三态】macOS 上存在 UDP 内核残留绑定：进程已退出但端口仍被持有，
/// 且不会应答任何报文。若把"无人应答"归为"别的程序占用"会造成误报。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortOwner {
    /// 端口可绑定（`bind` 成功）。
    Free,
    /// 端口上是我们自己的节点，返回它的版本与 PID。
    Ours(Probe),
    /// 端口上有东西，但它回的不是合法 GLWK 帧（真的别的程序）。
    Stranger,
    /// 端口不可绑定，但无人应答：内核残留绑定、防火墙丢弃等。
    Stale,
}

impl PortOwner {
    pub fn is_ours(&self) -> bool {
        matches!(self, PortOwner::Ours(_))
    }

    /// 是否属于「端口被外人占着」——只有真收到过非协议应答才算。
    pub fn is_foreign(&self) -> bool {
        matches!(self, PortOwner::Stranger)
    }

    /// 是否是没有任何应答的残留占用。
    pub fn is_stale(&self) -> bool {
        matches!(self, PortOwner::Stale)
    }
}

/// 向本机发现端口发起 `PROBE`，判断占用者身份。
///
/// 调用前应已确认端口**不可**绑定（否则没有探测的必要）。
pub fn identify(port: u16) -> Result<PortOwner> {
    let socket = UdpSocket::bind(("0.0.0.0", 0))?;
    socket.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;

    let probe = Frame::Probe(Probe {
        id: crate::patterns::singleton::DeviceIdentity::global().id.clone(),
        version: VERSION_STR.to_string(),
        pid: std::process::id(),
        // 探测者自己还没有监听端口
        reply_port: 0,
    })
    .encode()?;

    let target = ("127.0.0.1", port);
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            // 错开对方收包窗口的缝隙，避免几次尝试全部撞在同一段空档
            std::thread::sleep(RETRY_GAP);
        }
        // 对方可能来不及应答就重启，因此每轮都重发
        if socket.send_to(&probe, target).is_err() {
            // 连发都发不出去：同样属于"无人应答"
            return Ok(PortOwner::Stale);
        }
        let mut buf = vec![0u8; 512];
        match socket.recv_from(&mut buf) {
            Ok((len, from)) => {
                // 只接受来自回环的应答，避免被局域网上的噪声误导
                if !from.ip().is_loopback() {
                    continue;
                }
                return Ok(match Frame::decode(&buf[..len]) {
                    Ok(Frame::Pong(p)) => PortOwner::Ours(p),
                    // 端口上确实有人，但答的不是我们的协议 → 别的程序
                    _ => PortOwner::Stranger,
                });
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue
            }
            // ICMP 端口不可达：没有进程在收包，但仍 bind 不了（残留绑定）
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                return Ok(PortOwner::Stale)
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(PortOwner::Stale)
}

/// 应答收到的 `PROBE`：回一个带自身版本与 PID 的 `PONG`。
pub fn answer(socket: &UdpSocket, probe_target: std::net::SocketAddr) -> Result<()> {
    let identity = crate::patterns::singleton::DeviceIdentity::global();
    let pong = Frame::Pong(Probe {
        id: identity.id.clone(),
        version: VERSION_STR.to_string(),
        pid: std::process::id(),
        reply_port: socket.local_addr().ok().map(|a| a.port()).unwrap_or(0),
    })
    .encode()?;
    socket.send_to(&pong, probe_target)?;
    Ok(())
}

/// 版本比较结论，用于决定"重复启动"时该说什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionVerdict {
    /// 运行中的就是同一版本：无需重复启动。
    Same,
    /// 运行中的更旧：可以顶替升级。
    RunningIsOlder,
    /// 运行中的更新：本次启动应自行退出。
    RunningIsNewer,
}

/// 比较 `current`（本次要启动的版本）与 `running`（已在运行的版本）。
///
/// 只解析 `major.minor.patch` 数字段，解析不出的一律视为"相同"，
/// 避免版本号格式变化时误判成需要顶替。
pub fn compare_versions(current: &str, running: &str) -> VersionVerdict {
    let parse = |v: &str| -> Option<(u64, u64, u64)> {
        let mut parts = v.trim().split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().unwrap_or("0").parse().ok()?;
        let patch = parts.next().unwrap_or("0").parse().ok()?;
        Some((major, minor, patch))
    };
    match (parse(current), parse(running)) {
        (Some(a), Some(b)) if a == b => VersionVerdict::Same,
        (Some(a), Some(b)) if a > b => VersionVerdict::RunningIsOlder,
        (Some(_), Some(_)) => VersionVerdict::RunningIsNewer,
        _ => VersionVerdict::Same,
    }
}

/// 尝试绑定端口；被占用时先握手判定占用者身份。
///
/// 返回 `Ok(Some(socket))` 表示绑定成功；`Ok(None)` 表示端口被别人占着
/// （调用方应据此报错或提示版本冲突），`Err` 才是真正出不来。
pub fn bind_or_identify(port: u16) -> Result<(Option<UdpSocket>, PortOwner)> {
    match UdpSocket::bind(("0.0.0.0", port)) {
        Ok(socket) => {
            socket.set_broadcast(true)?;
            Ok((Some(socket), PortOwner::Free))
        }
        Err(e) if is_port_conflict(&e) => Ok((None, identify(port)?)),
        Err(e) => Err(e.into()),
    }
}

/// 服务启动时的端口决策结果。
#[derive(Debug)]
pub struct PortDecision {
    /// 实际持有的套接字。
    pub socket: UdpSocket,
    /// 实际使用的端口。
    pub port: u16,
    /// 首选端口（用于生成提示语）。
    pub preferred: u16,
    /// 首选端口被什么占着；直接绑上首选端口时为 [`PortOwner::Free`]。
    pub owner: PortOwner,
    /// 是否顺延到了其它端口。
    pub shifted: bool,
}

impl PortDecision {
    /// 需要告知用户的一句话；无需提示时返回 [`None`]。
    ///
    /// 四种情况分别给出不同措辞：只有真收到过非协议应答才说"被其它程序占用"，
    /// 残留绑定只说"残留占用"，自家节点则直接报出版本与 PID。
    pub fn notice(&self) -> Option<String> {
        if !self.shifted {
            return None;
        }
        Some(match &self.owner {
            PortOwner::Ours(p) => {
                let verdict = compare_versions(VERSION_STR, &p.version);
                let what = match verdict {
                    VersionVerdict::RunningIsNewer => "已有更新的节点在运行",
                    VersionVerdict::RunningIsOlder => "已有旧版本节点在运行",
                    VersionVerdict::Same => "已有本机节点在运行",
                };
                format!(
                    "[信息] 端口 {} 上{what}（v{}，PID {}），本次使用端口 {}。",
                    self.preferred, p.version, p.pid, self.port
                )
            }
            PortOwner::Stranger => format!(
                "[信息] 端口 {} 被其他程序占用，已自动使用端口 {}。",
                self.preferred, self.port
            ),
            PortOwner::Stale => format!(
                "[信息] 检测到端口 {} 残留占用（需重启系统释放），已自动使用端口 {}。",
                self.preferred, self.port
            ),
            PortOwner::Free => return None,
        })
    }
}

/// 服务启动时的端口决策：首选不可用就顺延，并把"为什么顺延"一并交回。
///
/// 与 [`bind_or_identify`] 的区别：本函数不会因为端口被占而失败，
/// 而是在握手判定后换个端口继续绑，适合 `serve` / 守护进程这类
/// "一定要跑起来"的场景。
pub fn decide_port(preferred: u16) -> Result<PortDecision> {
    decide_port_within(preferred, DISCOVERY_PORT_SPAN)
}

/// 与 [`decide_port`] 相同，但显式给定顺延跨度。
///
/// 跨度**必须与查询时的候选端口数一致**（见 [`crate::model::ports::candidate_ports`]）：
/// 我们最多顺延到哪，别人就最多找到哪。
pub fn decide_port_within(preferred: u16, span: u16) -> Result<PortDecision> {
    let start = preferred.max(1);
    // 只在第一个失败的端口上做一次握手，用来解释"为什么顺延"
    let mut owner = PortOwner::Free;

    for offset in 0..span.max(1) {
        let candidate = candidate_at(start, offset);
        let Some(candidate) = candidate else { continue };
        match UdpSocket::bind(("0.0.0.0", candidate)) {
            Ok(socket) => {
                socket.set_broadcast(true)?;
                return Ok(PortDecision {
                    socket,
                    port: candidate,
                    preferred: start,
                    owner,
                    shifted: offset != 0,
                });
            }
            Err(e) if is_port_conflict(&e) => {
                if offset == 0 {
                    owner = identify(candidate)?;
                }
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(GlowError::Config(format!(
        "端口 {start} 起的 {span} 个端口全部用不了，请用 --discovery-port 指定其它端口"
    )))
}

