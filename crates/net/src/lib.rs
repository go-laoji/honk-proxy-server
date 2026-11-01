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
    fn lookup_family(
        &self,
        host: String,
        purpose: ResolvePurpose,
        family: AddressFamily,
    ) -> NetworkFuture<'_, Vec<IpAddr>> {
        let pending = self.lookup(host, purpose);
        Box::pin(async move {
            Ok(pending
                .await?
                .into_iter()
                .filter(|ip| family.matches(*ip))
                .collect())
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
}
impl AddressFamily {
    pub fn matches(self, ip: IpAddr) -> bool {
        matches!(
            (self, ip),
            (Self::Ipv4, IpAddr::V4(_)) | (Self::Ipv6, IpAddr::V6(_))
        )
    }
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
