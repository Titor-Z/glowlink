//! 适配器模式：把标准库的 UDP / TCP 套接字适配成控制器定义的接口。

use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::time::Duration;

use crate::controller::discovery::{DiscoveryEvent, DiscoveryTransport};
use crate::controller::transfer::{TransferListener, TransferTransport};
use crate::model::error::{GlowError, Result};
use crate::model::packet::{Frame, Hello, Probe, MAX_FRAME};
use crate::model::probe::PortDecision;

/// 回环地址：广播包不保证回到本机，额外单发一份让同机多节点也能互相发现。
const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;

/// UDP 查询 / 单播应答发现适配器。
///
/// 端口被占用时降级为「只发查询、只收单播应答」：此时**不绑定固定的发现端口**
/// （否则会把端口占死，让后来者永远无法成为应答方），但会保留一个临时端口用于
/// 接收他人对自己的应答。
pub struct UdpDiscoveryAdapter {
    /// 用于收发的套接字；降级模式下只用于发送。
    socket: UdpSocket,
    degraded: bool,
    /// 对外宣告使用的端口（用于发送到广播地址）。
    announced_port: u16,
}

impl UdpDiscoveryAdapter {
    /// 从已完成端口决策的结果创建适配器（`serve` / 守护进程使用）。
    pub fn from_decision(decision: PortDecision) -> Result<Self> {
        // 顺延只是“没用上约定端口”，并不代表失去监听能力：
        // 这个套接字确实是绑上的，所以 degraded 必须为 false，
        // 否则 local_port() 会返回 None，PONG 里的 reply_port 就变成 0。
        Ok(UdpDiscoveryAdapter {
            socket: decision.socket,
            degraded: false,
            announced_port: decision.preferred,
        })
    }

    /// 绑定发现端口；端口已被占用时自动降级为仅主动查询。
    pub fn bind(port: u16) -> Result<Self> {
        match UdpSocket::bind(("0.0.0.0", port)) {
            Ok(socket) => {
                socket.set_broadcast(true)?;
                Ok(UdpDiscoveryAdapter { socket, degraded: false, announced_port: port })
            }
            Err(e) if e.kind() == ErrorKind::AddrInUse => {
                // 不绑定固定端口，让 UDP 栈在每次发送时分配临时源端口。
                let socket = UdpSocket::bind(("0.0.0.0", 0))?;
                socket.set_broadcast(true)?;
                Ok(UdpDiscoveryAdapter { socket, degraded: true, announced_port: port })
            }
            Err(e) => Err(GlowError::Io(e)),
        }
    }

    /// 实际监听端口；降级模式下没有监听端口。
    pub fn bound_port(&self) -> Option<u16> {
        if self.degraded {
            // send-only 套接字绑在 0，其 local_addr 每次发送后都可能变化，不对外暴露
            None
        } else {
            self.socket.local_addr().ok().map(|addr| addr.port())
        }
    }

    fn send_to<A: std::net::ToSocketAddrs>(&self, bytes: &[u8], target: A) -> Result<()> {
        self.socket.send_to(bytes, target)?;
        Ok(())
    }
}

impl DiscoveryTransport for UdpDiscoveryAdapter {
    fn local_port(&self) -> Option<u16> {
        self.bound_port()
    }

    fn query(&self, hello: &Hello) -> Result<()> {
        let bytes = Frame::Query(hello.clone()).encode()?;
        // 广播到“对外宣告的端口”：局域网内其它节点监听的都是那个端口，
        // 即使我们自己因为冲突换了端口，别人也要能按约定端口收到查询。
        //
        // 广播发不出去**不算协议失败**：网卡关着、没有路由、机器刚唤醒都会
        // 报 ENETUNREACH。这类环境问题不能中断这一轮——否则收包也不做了、
        // 查询时间戳也不更新，主循环就会原地空转（曾一秒写上万行日志把磁盘写满）。
        let broadcast = self.send_to(&bytes, (Ipv4Addr::BROADCAST, self.announced_port));
        // 同机多节点：广播不一定回到本机，补发回环
        let loopback = self.send_to(&bytes, (LOOPBACK, self.announced_port));
        // 只有「回环也发不出去」才说明套接字本身出了问题，那才是真错误
        loopback?;
        let _ = broadcast;
        Ok(())
    }

    fn reply(&self, hello: &Hello, target: SocketAddr) -> Result<()> {
        let bytes = Frame::Hello(hello.clone()).encode()?;
        self.send_to(&bytes, target)
    }

    fn reply_probe(&self, probe: &Probe, target: SocketAddr) -> Result<()> {
        let bytes = Frame::Pong(probe.clone()).encode()?;
        self.send_to(&bytes, target)
    }

    fn poll(&self, wait: Duration) -> Result<Vec<DiscoveryEvent>> {
        // 降级模式收不到广播查询（没有固定的接收端口），但临时端口上仍可能
        // 收到他人对自己查询的单播应答，所以必须照常读包。
        self.socket.set_read_timeout(Some(wait))?;
        let mut events = Vec::new();
        let mut buf = vec![0u8; MAX_FRAME];
        loop {
            match self.socket.recv_from(&mut buf) {
                Ok((len, peer)) => match Frame::decode(&buf[..len]) {
                    Ok(Frame::Query(hello)) => {
                        events.push(DiscoveryEvent::Query { hello, from: peer })
                    }
                    Ok(Frame::Hello(hello)) => {
                        events.push(DiscoveryEvent::Reply { hello, from: peer })
                    }
                    // 端口自检探测：交给控制器回 PONG，证明“这个端口是我们自己人”
                    Ok(Frame::Probe(_)) => {
                        events.push(DiscoveryEvent::Probe { from: peer })
                    }
                    // 非本协议报文一律忽略，不因外部噪声中断发现流程。
                    _ => continue,
                },
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                    break
                }
                // 回环探测可能触发 ICMP 端口不可达，属于预期噪声。
                Err(e) if e.kind() == ErrorKind::ConnectionRefused => continue,
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(GlowError::Io(e)),
            }
        }
        Ok(events)
    }

    fn degraded(&self) -> bool {
        self.degraded
    }
}

/// 根据收到的查询 / 应答报文算出该回包的目标地址。
///
/// 优先用报文里声明的 `reply_port`：广播时系统分配的源端口是临时的，
/// 直接回源端口会打到一个已经关闭的 socket 上。`reply_port` 为 0（对方也是
/// 降级节点）时才回退到报文的真实来源地址。
pub fn reply_target(hello: &Hello, from: SocketAddr) -> SocketAddr {
    if hello.reply_port == 0 {
        from
    } else {
        SocketAddr::new(from.ip(), hello.reply_port)
    }
}

/// TCP 传输适配器（已连接的一端）。
pub struct TcpTransportAdapter {
    stream: TcpStream,
}

impl TcpTransportAdapter {
    pub fn connect(addr: SocketAddr, timeout: Duration) -> Result<Self> {
        Ok(TcpTransportAdapter::from_stream(TcpStream::connect_timeout(&addr, timeout)?)?)
    }

    pub fn from_stream(stream: TcpStream) -> Result<Self> {
        stream.set_nodelay(true)?;
        Ok(TcpTransportAdapter { stream })
    }
}

impl TransferTransport for TcpTransportAdapter {
    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        frame.write_to(&mut self.stream)
    }

    fn recv_frame(&mut self) -> Result<Frame> {
        Frame::read_from(&mut self.stream)
    }
}

/// TCP 监听适配器（监听端）。
pub struct TcpListenerAdapter {
    listener: TcpListener,
}

impl TcpListenerAdapter {
    /// `port` 传 0 表示由系统分配空闲端口。
    pub fn bind(port: u16) -> Result<Self> {
        Ok(TcpListenerAdapter { listener: TcpListener::bind(("0.0.0.0", port))? })
    }
}

impl TransferListener for TcpListenerAdapter {
    fn local_port(&self) -> Result<u16> {
        Ok(self.listener.local_addr()?.port())
    }

    fn accept(&self) -> Result<(Box<dyn TransferTransport>, SocketAddr)> {
        let (stream, peer) = self.listener.accept()?;
        Ok((Box::new(TcpTransportAdapter::from_stream(stream)?), peer))
    }
}
