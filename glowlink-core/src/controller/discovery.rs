//! 设备发现控制器：只依赖 [`DiscoveryTransport`] 接口，具体实现由工厂注入。

use std::net::IpAddr;
use std::time::{Duration, Instant};

use crate::model::device::{Device, DeviceBook};
use crate::model::error::Result;
use crate::model::packet::Hello;
use crate::patterns::singleton::DeviceIdentity;

/// 发现通道（接口）。UDP 广播与测试替身都实现它。
pub trait DiscoveryTransport: Send {
    /// 广播本机存在。
    fn announce(&self, hello: &Hello) -> Result<()>;
    /// 收取一段时间内收到的其他节点广播。
    fn poll(&self, wait: Duration) -> Result<Vec<(Hello, IpAddr)>>;
}

pub struct DiscoveryController {
    transport: Box<dyn DiscoveryTransport>,
    identity: DeviceIdentity,
    port: u16,
    book: DeviceBook,
    announce_every: Duration,
    last_announce: Option<Instant>,
}

impl DiscoveryController {
    /// `port` 为本机 TCP 传输端口；`timeout` 为设备离线判定时长。
    pub fn new(
        transport: Box<dyn DiscoveryTransport>,
        identity: DeviceIdentity,
        port: u16,
        timeout: Duration,
        announce: Duration,
    ) -> Self {
        DiscoveryController {
            transport,
            identity,
            port,
            book: DeviceBook::new(timeout),
            announce_every: announce,
            last_announce: None,
        }
    }

    /// 推进一次：按周期广播，收包合并，裁剪超时设备。返回在线设备数。
    pub fn tick(&mut self) -> Result<usize> {
        let now = Instant::now();
        let due = self
            .last_announce
            .map_or(true, |last| now.duration_since(last) >= self.announce_every);
        if due {
            let hello = Hello {
                id: self.identity.id.clone(),
                name: self.identity.name.clone(),
                port: self.port,
            };
            self.transport.announce(&hello)?;
            self.last_announce = Some(now);
        }

        // 广播周期的一半作为单次收包等待，保证广播节奏不被阻塞。
        let wait = self.announce_every / 2;
        for (hello, ip) in self.transport.poll(wait)? {
            if hello.id == self.identity.id {
                continue;
            }
            self.book.observe(&hello, ip);
        }
        self.book.prune();
        Ok(self.book.len())
    }

    pub fn devices(&self) -> Vec<Device> {
        self.book.devices()
    }

    pub fn find(&self, key: &str) -> Option<Device> {
        self.book.find(key)
    }
}
