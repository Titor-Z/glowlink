//! 适配器模式：把标准库的 UDP / TCP 套接字适配成控制器定义的接口。

use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::time::Duration;

use crate::controller::discovery::{DiscoveryEvent, DiscoveryTransport};
use crate::controller::transfer::{TransferListener, TransferTransport};
use crate::model::error::{GlowError, Result};
use crate::model::packet::{Frame, Hello, MAX_FRAME};

/// 本机回环地址：广播包在同机多实例下不一定被回环，额外单发一份保证本机可联通。
const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;

/// UDP 查询 / 单播应答发现适配器。
pub struct UdpDiscoveryAdapter {
    socket: UdpSocket,
    port: u16,
    degraded: bool,
}

impl UdpDiscoveryAdapter {
    /// 绑定发现端口；端口已被占用时自动降级为临时端口（只能主动查询与收单播应答）。
    pub fn bind(port: u16) -> Result<Self> {
        let (socket, degraded) = match UdpSocket::bind(("0.0.0.0", port)) {
            Ok(socket) => (socket, false),
            Err(e) if e.kind() == ErrorKind::AddrInUse => (UdpSocket::bind(("0.0.0.0", 0))?, true),
            Err(e) => return Err(GlowError::Io(e)),
        };
        socket.set_broadcast(true)?;
        Ok(UdpDiscoveryAdapter { socket, port, degraded })
    }

    /// 实际监听端口（降级时为系统分配的临时端口）。
    pub fn bound_port(&self) -> Result<u16> {
        Ok(self.socket.local_addr()?.port())
    }
}

impl DiscoveryTransport for UdpDiscoveryAdapter {
    fn local_port(&self) -> Result<u16> {
        self.bound_port()
    }

    fn query(&self, hello: &Hello) -> Result<()> {
        let bytes = Frame::Query(hello.clone()).encode()?;
        self.socket.send_to(&bytes, ("255.255.255.255", self.port))?;
        // 广播不保证回到本机，补发回环：同机多实例（含降级实例）因此也能互相发现。
        self.socket.send_to(&bytes, (LOOPBACK, self.port))?;
        Ok(())
    }

    fn reply(&self, hello: &Hello, target: SocketAddr) -> Result<()> {
        let bytes = Frame::Hello(hello.clone()).encode()?;
        self.socket.send_to(&bytes, target)?;
        Ok(())
    }

    fn poll(&self, wait: Duration) -> Result<Vec<DiscoveryEvent>> {
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
                    // 非本协议报文一律忽略，不因外部噪声中断发现流程。
                    _ => continue,
                },
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                    break
                }
                // 回环探测可能触发 ICMP 端口不可达，属于预期噪声，继续收包。
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
