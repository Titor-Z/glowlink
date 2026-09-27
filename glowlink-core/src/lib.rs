//! 萤光 GlowLink 核心库。
//!
//! 分层（MVC）：
//! - [`model`]：纯数据结构与算法，不做任何 IO。
//! - [`controller`]：业务编排，只依赖 `model` 与接口（trait），不依赖具体实现。
//! - [`patterns`]：工厂、适配器、单例等具体实现的装配点。

pub mod controller;
pub mod model;
pub mod patterns;

pub use controller::discovery::DiscoveryController;
pub use controller::transfer::{
    TransferController, TransferListener, TransferTransport, TransferView,
};
pub use model::config::Config;
pub use model::crypto::{
    fingerprint_of, hkdf_expand, hmac_sha256, sha256, FrameCipher, Identity, Role, SecureHello, Session,
};
pub use model::device::{Device, DeviceBook, DeviceId};
pub use model::error::{GlowError, Result};
pub use model::packet::{Frame, Hello, Probe, CHUNK_SIZE, DISCOVERY_PORT, VERSION_STR};
pub use model::ports::{bind_udp_with_fallback, resolve_udp_port, BindOutcome};
pub use model::probe::{
    bind_or_identify, compare_versions, decide_port, identify, PortDecision, PortOwner,
    VersionVerdict,
};
pub use model::ring::{RingBuffer, SharedRing};
pub use patterns::secure::{
    session_slot, NoTrust, SecureTransport, SessionInfo, SessionSlot, TrustList,
};
pub use patterns::factory::{
    DiscoveryTransportFactory, NativeTransportFactory, TransferTransportFactory,
};
pub use patterns::singleton::DeviceIdentity;
