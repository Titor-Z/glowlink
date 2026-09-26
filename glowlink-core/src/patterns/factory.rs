//! 工厂模式：按平台/场景创建具体通道实现，控制器只见到接口。

use std::net::SocketAddr;
use std::time::Duration;

use crate::controller::discovery::DiscoveryTransport;
use crate::controller::transfer::{TransferListener, TransferTransport};
use crate::model::error::Result;
use crate::patterns::adapter::{TcpListenerAdapter, TcpTransportAdapter, UdpDiscoveryAdapter};

/// 发现通道工厂。
pub trait DiscoveryTransportFactory {
    fn create(&self, broadcast_port: u16) -> Result<Box<dyn DiscoveryTransport>>;
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
    fn create(&self, broadcast_port: u16) -> Result<Box<dyn DiscoveryTransport>> {
        Ok(Box::new(UdpDiscoveryAdapter::bind(broadcast_port)?))
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
