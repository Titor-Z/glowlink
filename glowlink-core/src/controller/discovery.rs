//! 设备发现控制器：只依赖 [`DiscoveryTransport`] 接口，具体实现由工厂注入。
//!
//! 发现模式为「查询 - 单播应答」，不使用多播、不依赖 `SO_REUSEPORT`：
//! 1. 每个节点周期性地向广播地址发送 `QUERY`（携带自身身份与传输端口）；
//! 2. 收到 `QUERY` 的节点向其**来源地址单播**回 `HELLO`，不做二次广播；
//! 3. 因此查询方与应答方在一次交互中互相认识，无中心节点，也无广播风暴。

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crate::model::device::{Device, DeviceBook};
use crate::model::error::Result;
use crate::model::packet::Hello;
use crate::patterns::singleton::DeviceIdentity;

/// 一次发现交互的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// 收到他人查询：需要向 `from` 单播应答。
    Query { hello: Hello, from: SocketAddr },
    /// 收到他人对我们查询的应答。
    Reply { hello: Hello, from: SocketAddr },
}

/// 发现通道（接口）。UDP 广播实现与测试替身都实现它。
pub trait DiscoveryTransport: Send {
    /// 本机实际用于收发的端口。
    fn local_port(&self) -> Result<u16>;

    /// 广播查询包。
    fn query(&self, hello: &Hello) -> Result<()>;

    /// 向查询来源单播应答。
    fn reply(&self, hello: &Hello, target: SocketAddr) -> Result<()>;

    /// 收取一段时间内到达的发现报文。
    fn poll(&self, wait: Duration) -> Result<Vec<DiscoveryEvent>>;

    /// 是否因端口被占用而处于降级（仅主动查询）模式。
    fn degraded(&self) -> bool {
        false
    }
}

pub struct DiscoveryController {
    transport: Box<dyn DiscoveryTransport>,
    identity: DeviceIdentity,
    port: u16,
    book: DeviceBook,
    query_every: Duration,
    last_query: Option<Instant>,
}

impl DiscoveryController {
    /// `port` 为本机 TCP 传输端口；`timeout` 为设备离线判定时长；
    /// `query_every` 为主动查询周期。
    pub fn new(
        transport: Box<dyn DiscoveryTransport>,
        identity: DeviceIdentity,
        port: u16,
        timeout: Duration,
        query_every: Duration,
    ) -> Self {
        DiscoveryController {
            transport,
            identity,
            port,
            book: DeviceBook::new(timeout),
            query_every,
            last_query: None,
        }
    }

    pub fn local_port(&self) -> Result<u16> {
        self.transport.local_port()
    }

    pub fn degraded(&self) -> bool {
        self.transport.degraded()
    }

    /// 推进一次：按周期查询，应答他人查询，合并设备，裁剪超时设备。
    /// 返回在线设备数。
    pub fn tick(&mut self) -> Result<usize> {
        let now = Instant::now();
        let due = self
            .last_query
            .map_or(true, |last| now.duration_since(last) >= self.query_every);
        if due {
            self.transport.query(&self.hello())?;
            self.last_query = Some(now);
        }

        // 查询周期的一半作为单次收包等待，保证查询节奏不被长阻塞打乱。
        for event in self.transport.poll(self.query_every / 2)? {
            let (hello, from) = match event {
                DiscoveryEvent::Query { hello, from } => {
                    if hello.id != self.identity.id {
                        self.transport.reply(&self.hello(), from)?;
                    }
                    (hello, from)
                }
                DiscoveryEvent::Reply { hello, from } => (hello, from),
            };
            if hello.id == self.identity.id {
                continue;
            }
            self.book.observe(&hello, from.ip());
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

    fn hello(&self) -> Hello {
        Hello {
            id: self.identity.id.clone(),
            name: self.identity.name.clone(),
            port: self.port,
        }
    }
}
