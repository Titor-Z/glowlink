//! 端口分配策略：先试首选端口，被占用则向外顺延，全失败才算错。
//!
//! 不使用 `lsof` / `netstat` 判断占用：它们会漏掉沙盒进程与内核层占用
//! （实测本机某端口被一个 `lsof` 完全看不见的系统进程锁死），
//! 唯一可靠的判据是能否真正 `bind`。

use std::net::UdpSocket;

/// 发现端口的跨度：**既是**「被占用时最多顺延多少个端口」，**也是**「查询时向多少个端口各发一份」。
///
/// 这两个数字必须由同一个常量驱动：若节点能顺延到第 30 个端口，而别人只探测前 21 个，
/// 它就会「明明开着却谁也发现不了」——比直接报错难查得多。
pub const DISCOVERY_PORT_SPAN: u16 = 21;

/// 一次端口尝试的结果，供调用方区分“被占”与“真出错”。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindOutcome {
    /// 绑定成功，返回实际可用端口。
    Bound(u16),
    /// 端口被占用（或权限不允许），可以换一个再试。
    Taken,
}

/// 试探某个 UDP 端口是否可用（绑定后立即释放）。
///
/// 这是判断端口占用与否的唯一可靠方式：系统工具会漏掉不可见的占用者，
/// 而 `bind` 是内核给出的最终裁决。
pub fn probe_udp(port: u16) -> BindOutcome {
    match UdpSocket::bind(("0.0.0.0", port)) {
        Ok(_) => BindOutcome::Bound(port),
        Err(e) if is_port_conflict(&e) => BindOutcome::Taken,
        // 其它错误（例如端口 0 或权限问题）同样认为该端口不可用
        Err(_) => BindOutcome::Taken,
    }
}

/// 从 `preferred` 起算的第 `offset` 个候选端口；越界返回 `None`。
///
/// 越界时**不回绕**：宁可少试几个，也不要跳回低端口区去（那里大概率也被占着，
/// 而且会让「探查端口」与「实际落点」错位）。
pub fn candidate_at(preferred: u16, offset: u16) -> Option<u16> {
    let candidate = preferred.max(1).checked_add(offset)?;
    (candidate > 0).then_some(candidate)
}

/// 查询时要探测的候选端口列表：约定端口 + 顺延位，共 `span` 个。
///
/// 与节点实际可能落入的端口序列完全一致（都用 [`candidate_at`]），
/// 所以「对方顺延到哪」与「我们去哪找」不会脱节。
pub fn candidate_ports(preferred: u16, span: u16) -> Vec<u16> {
    (0..span.max(1)).filter_map(|offset| candidate_at(preferred, offset)).collect()
}

/// 端口冲突类错误：换一个端口就能解决。
pub fn is_port_conflict(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
    )
}
