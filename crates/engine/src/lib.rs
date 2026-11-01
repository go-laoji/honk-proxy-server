//! Shared user-space transport for explicit and future transparent proxy ingress.
mod capacity;
mod demand;
pub mod dial;
pub mod health;
mod http_proxy;
pub mod nodes;
pub mod pool;
pub mod routing;
pub mod runtime;
mod selection;
mod socks;
pub mod target;
pub mod tls;
mod traffic;

pub use hps_net::{BoxIo, ProxyIo};

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("routing policy rejected this connection")]
    Rejected,
    #[error("routing policy or references are invalid")]
    RuleConfiguration,
    #[error("routing needs a data-plane capability not available on this target")]
    UnsupportedRule,
    #[error("no eligible upstream; direct fallback is disabled")]
    NoUpstream,
    #[error("invalid destination")]
    InvalidTarget,
    #[error("upstream connection or handshake timed out")]
    Timeout,
    #[error("local socket or memory resource exhausted")]
    LocalResource,
    #[error("upstream endpoint connection failed")]
    Endpoint,
    #[error("upstream endpoint DNS resolution failed")]
    Dns,
    #[error("upstream TLS handshake failed")]
    UpstreamTls,
    #[error("upstream authentication failed")]
    Authentication,
    #[error("upstream CONNECT rejected with status {0}")]
    Policy(u16),
    #[error("upstream rejected target connection")]
    TargetConnect,
    #[error("upstream connection or handshake failed")]
    Upstream,
    #[error("invalid TLS certificate, key, trust roots or server name")]
    TlsConfig,
    #[error("invalid or failed proxy protocol")]
    Protocol,
    #[error("I/O failed")]
    Io(#[from] std::io::Error),
}
