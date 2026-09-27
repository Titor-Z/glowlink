//! 工厂模式：按平台/场景创建具体通道实现，控制器只见到接口。

use std::net::SocketAddr;
use std::time::Duration;

use crate::controller::discovery::DiscoveryTransport;
use crate::controller::transfer::{TransferListener, TransferTransport};
use crate::model::error::Result;
use crate::model::probe::PortDecision;
use crate::patterns::adapter::{TcpListenerAdapter, TcpTransportAdapter, UdpDiscoveryAdapter};

/// 发现通道工厂。
pub trait DiscoveryTransportFactory {
    /// `port` 为本机发现端口，`span` 为查询要覆盖的候选端口个数（局域网内应统一）。
    fn create(&self, port: u16, span: u16) -> Result<Box<dyn DiscoveryTransport>>;

    /// 用已经做好端口决策的套接字创建通道，避免重复 bind。
    fn from_bound_socket(
        &self,
        decision: PortDecision,
        span: u16,
    ) -> Result<Box<dyn DiscoveryTransport>>;
}

/// 传输通道工厂。
pub trait TransferTransportFactory {
    fn connect(&self, addr: SocketAddr, timeout: Duration) -> Result<Box<dyn TransferTransport>>;
    fn listen(&self, port: u16) -> Result<Box<dyn TransferListener>>;
}

/// 桌面端原生实现（UDP 广播 + TCP 直连）。
///
/// 未来 Android 的 Wi-Fi Direct 实现只需再提供一个工厂，控制器无需改动。
pub struct NativeTransportFactory;

impl DiscoveryTransportFactory for NativeTransportFactory {
    fn create(&self, port: u16, span: u16) -> Result<Box<dyn DiscoveryTransport>> {
        Ok(Box::new(UdpDiscoveryAdapter::bind(port)?.with_span(span)))
    }

    fn from_bound_socket(
        &self,
        decision: PortDecision,
        span: u16,
    ) -> Result<Box<dyn DiscoveryTransport>> {
        Ok(Box::new(UdpDiscoveryAdapter::from_decision(decision)?.with_span(span)))
    }
}

impl TransferTransportFactory for NativeTransportFactory {
    fn connect(&self, addr: SocketAddr, timeout: Duration) -> Result<Box<dyn TransferTransport>> {
        Ok(Box::new(TcpTransportAdapter::connect(addr, timeout)?))
    }

    fn listen(&self, port: u16) -> Result<Box<dyn TransferListener>> {
        Ok(Box::new(TcpListenerAdapter::bind(port)?))
    }
}
