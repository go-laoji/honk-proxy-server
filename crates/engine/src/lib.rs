//! Shared user-space transport for explicit and future transparent proxy ingress.
pub mod dial;
pub mod target;
pub mod tls;

use tokio::io::{AsyncRead, AsyncWrite};
pub trait ProxyIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ProxyIo for T {}
pub type BoxIo = Box<dyn ProxyIo>;

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("no eligible upstream; direct fallback is disabled")]
    NoUpstream,
    #[error("invalid destination")]
    InvalidTarget,
    #[error("upstream connection or handshake timed out")]
    Timeout,
    #[error("upstream connection or handshake failed")]
    Upstream,
    #[error("invalid TLS certificate, key, trust roots or server name")]
    TlsConfig,
    #[error("I/O failed")]
    Io(#[from] std::io::Error),
}
