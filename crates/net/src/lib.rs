//! Transport contracts shared by DNS and the proxy engine, without a dependency cycle.
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    net::{IpAddr, SocketAddr},
    pin::Pin,
};
use tokio::io::{AsyncRead, AsyncWrite};

pub trait ProxyIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ProxyIo for T {}
pub type BoxIo = Box<dyn ProxyIo>;
pub type NetworkFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, NetworkError>> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvePurpose {
    Bootstrap,
    Provider,
    Health,
    Business,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct NetworkError(pub &'static str);

pub trait NameResolver: Send + Sync {
    fn lookup(&self, host: String, purpose: ResolvePurpose) -> NetworkFuture<'_, Vec<IpAddr>>;
}

/// A fixed static egress. Implementations must not select/buy a pool node or resolve
/// the proxy endpoint through this same DNS service while opening this connection.
pub trait DnsConnector: Send + Sync {
    fn connect(
        &self,
        proxy_id: String,
        target: SocketAddr,
        deadline: tokio::time::Instant,
    ) -> NetworkFuture<'_, BoxIo>;
    /// Includes prepared proxy credentials and actual trust roots, never returned in diagnostics.
    fn fingerprint(&self, proxy_id: &str) -> Option<String>;
}
