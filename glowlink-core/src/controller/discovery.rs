//! 设备发现控制器：只依赖 [`DiscoveryTransport`] 接口，具体实现由工厂注入。
//!
//! 发现模式为「查询 - 单播应答」，不使用多播、不依赖 `SO_REUSEPORT`：
//! 1. 每个节点周期性地向广播地址发送 `QUERY`（携带自身身份、TCP 端口与 UDP 接收端口）；
//! 2. 收到 `QUERY` 的节点向其**报文里声明的来源 UDP 端口单播**回 `HELLO`，不做二次广播；
//! 3. 因此查询方与应答方在一次交互中互相认识，无中心节点，也无广播风暴。
//!
//! 注意应答目标用的是报文里声明的端口而非报文的源端口：
//! 节点向外广播时会临时创建一个 socket，若直接回复该 socket 的源端口，
//! 对方早已关闭它，应答就会丢掉。

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crate::model::device::{Device, DeviceBook};
use crate::model::error::Result;
use crate::model::packet::{Hello, Probe};
use crate::patterns::adapter::reply_target;
use crate::patterns::singleton::DeviceIdentity;

/// 一次发现交互的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// 收到他人查询：需要向 `from` 单播应答。
    Query { hello: Hello, from: SocketAddr },
    /// 收到他人对我们查询的应答。
    Reply { hello: Hello, from: SocketAddr },
    /// 收到端口自检探测：需要回 PONG，声明“这个端口是我们自己人”。
    Probe { from: SocketAddr },
}

/// 发现通道（接口）。UDP 广播实现与测试替身都实现它。
pub trait DiscoveryTransport: Send {
    /// 本机接收发现报文的端口；没有可用接收端口时为 [`None`]（降级为只发查询）。
    fn local_port(&self) -> Option<u16>;

    /// 广播查询包。
    fn query(&self, hello: &Hello) -> Result<()>;

    /// 向对方声明的 UDP 接收端口单播应答。
    fn reply(&self, hello: &Hello, target: SocketAddr) -> Result<()>;

    /// 回应端口自检的 `PROBE`。
    fn reply_probe(&self, probe: &Probe, target: SocketAddr) -> Result<()>;

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

    pub fn local_port(&self) -> Option<u16> {
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
                        // 回包目标用报文声明的接收端口，而不是广播时的临时源端口
                        self.transport.reply(&self.hello(), reply_target(&hello, from))?;
                    }
                    (hello, from)
                }
                DiscoveryEvent::Reply { hello, from } => (hello, from),
                // 端口自检探测：回一个 PONG 告诉对方“这个端口是我们自己人”。
                DiscoveryEvent::Probe { from } => {
                    let pong = Probe {
                        id: self.identity.id.clone(),
                        version: crate::model::packet::VERSION_STR.to_string(),
                        pid: std::process::id(),
                        reply_port: self.transport.local_port().unwrap_or(0),
                    };
                    self.transport.reply_probe(&pong, from)?;
                    continue;
                }
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
        // 降级模式下没有接收端口，字段置 0；对方会回落到报文源地址。
        Hello {
            id: self.identity.id.clone(),
            name: self.identity.name.clone(),
            port: self.port,
            reply_port: self.transport.local_port().unwrap_or(0),
        }
    }
}
