//! 适配器模式：把标准库的 UDP / TCP 套接字适配成控制器定义的接口。

use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::time::Duration;

use crate::controller::discovery::DiscoveryTransport;
use crate::controller::transfer::{TransferListener, TransferTransport};
use crate::model::error::{GlowError, Result};
use crate::model::packet::{Frame, Hello, MAX_FRAME};

/// UDP 广播发现适配器。
pub struct UdpDiscoveryAdapter {
    socket: UdpSocket,
    broadcast_port: u16,
}

impl UdpDiscoveryAdapter {
    pub fn bind(broadcast_port: u16) -> Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", broadcast_port))?;
        socket.set_broadcast(true)?;
        Ok(UdpDiscoveryAdapter { socket, broadcast_port })
    }
}

impl DiscoveryTransport for UdpDiscoveryAdapter {
    fn announce(&self, hello: &Hello) -> Result<()> {
        let bytes = Frame::Hello(hello.clone()).encode()?;
        self.socket
            .send_to(&bytes, ("255.255.255.255", self.broadcast_port))?;
        Ok(())
    }

    fn poll(&self, wait: Duration) -> Result<Vec<(Hello, IpAddr)>> {
        self.socket.set_read_timeout(Some(wait))?;
        let mut received = Vec::new();
        let mut buf = vec![0u8; MAX_FRAME];
        loop {
            match self.socket.recv_from(&mut buf) {
                Ok((len, peer)) => match Frame::decode(&buf[..len]) {
                    // 非本协议报文一律忽略，不因外部噪声中断发现流程。
                    Ok(Frame::Hello(hello)) => received.push((hello, peer.ip())),
                    _ => continue,
                },
                Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                    break
                }
                Err(e) => return Err(GlowError::Io(e)),
            }
        }
        Ok(received)
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
