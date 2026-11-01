use crate::{BoxIo, ProxyError, target::Target, tls};
use base64::Engine;
use hps_config::{ProxyConfig, ProxyProtocol, UpstreamConfig};
use http::HeaderValue;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, pki_types::ServerName},
};

pub(crate) struct Upstream {
    pub(crate) config: UpstreamConfig,
    tls: Option<Arc<ClientConfig>>,
    pub(crate) source: Option<(String, String)>,
    pub(crate) deadline: Option<std::time::Instant>,
    pub(crate) minimum_lifetime: u64,
}
impl Upstream {
    pub(crate) fn prepare(config: UpstreamConfig) -> Result<Self, ProxyError> {
        let tls = if config.protocol == ProxyProtocol::Https {
            Some(tls::client(&config)?)
        } else {
            None
        };
        Ok(Self {
            config,
            tls,
            source: None,
            deadline: None,
            minimum_lifetime: 0,
        })
    }
}
pub struct Dialer {
    upstreams: Vec<Arc<Upstream>>,
    pub(crate) pool: Arc<crate::pool::DynamicPool>,
    pub(crate) demand: Arc<crate::demand::Demand>,
    sources: std::sync::RwLock<std::collections::HashMap<String, String>>,
    cursor: AtomicUsize,
    timeout: Duration,
    min_lifetime: u64,
    max_header: usize,
}
pub struct Connected {
    pub io: BoxIo,
    pub upstream_id: String,
    pub absolute_form: bool,
    pub proxy_authorization: Option<HeaderValue>,
}
impl Dialer {
    pub fn new(config: &ProxyConfig) -> Result<Self, ProxyError> {
        let upstreams = config
            .upstreams
            .iter()
            .map(|config| Upstream::prepare(config.clone()).map(Arc::new))
            .collect::<Result<_, ProxyError>>()?;
        Ok(Self {
            upstreams,
            pool: Arc::new(crate::pool::DynamicPool::default()),
            demand: Arc::new(crate::demand::Demand::default()),
            sources: std::sync::RwLock::new(std::collections::HashMap::new()),
            cursor: AtomicUsize::new(0),
            timeout: Duration::from_millis(config.limits.connect_timeout_ms),
            min_lifetime: config.limits.min_remaining_lifetime_secs,
            max_header: config.limits.max_header_bytes,
        })
    }
    pub fn set_sources(&self, sources: std::collections::HashMap<String, String>) {
        *self.sources.write().unwrap() = sources;
    }
    fn eligible(&self, upstream: &Upstream) -> bool {
        let config = &upstream.config;
        let minimum = self.min_lifetime.max(upstream.minimum_lifetime);
        if upstream
            .source
            .as_ref()
            .is_some_and(|(id, fp)| self.sources.read().unwrap().get(id) != Some(fp))
            || upstream.deadline.is_some_and(|deadline| {
                deadline <= std::time::Instant::now() + Duration::from_secs(minimum)
            })
        {
            return false;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        config.enabled
            && config
                .expires_at_unix
                .is_none_or(|expires| expires > now.saturating_add(minimum))
    }
    pub fn eligible_count(&self) -> usize {
        let dynamic = self.pool.snapshot();
        self.upstreams
            .iter()
            .chain(dynamic.iter())
            .filter(|s| self.eligible(s))
            .count()
    }
    /// Select exactly once. Neither failed handshakes nor business data are retried.
    pub async fn connect(
        &self,
        target: &Target,
        http_forward: bool,
    ) -> Result<Connected, ProxyError> {
        let demand = self.demand.begin();
        let dynamic = self.pool.snapshot();
        let total = self.upstreams.len() + dynamic.len();
        if total == 0 {
            return Err(ProxyError::NoUpstream);
        }
        let start = self.cursor.fetch_add(1, Ordering::Relaxed) % total;
        let upstream = (0..total)
            .map(|n| {
                let index = (start + n) % total;
                if index < self.upstreams.len() {
                    &self.upstreams[index]
                } else {
                    &dynamic[index - self.upstreams.len()]
                }
            })
            .find(|s| self.eligible(s))
            .ok_or(ProxyError::NoUpstream)?;
        tokio::time::timeout(
            self.timeout,
            self.connect_selected(upstream, target, http_forward),
        )
        .await
        .map_err(|_| ProxyError::Timeout)?
        .map(|mut connected| {
            connected.io = demand.wrap(connected.io);
            connected
        })
    }
    async fn connect_selected(
        &self,
        upstream: &Upstream,
        target: &Target,
        http_forward: bool,
    ) -> Result<Connected, ProxyError> {
        let config = &upstream.config;
        let tcp = TcpStream::connect((config.host.as_str(), config.port))
            .await
            .map_err(|_| ProxyError::Upstream)?;
        tcp.set_nodelay(true)?;
        let mut io: BoxIo = if let Some(tls) = &upstream.tls {
            let name = ServerName::try_from(
                config
                    .tls_server_name
                    .as_deref()
                    .unwrap_or(&config.host)
                    .to_owned(),
            )
            .map_err(|_| ProxyError::TlsConfig)?;
            Box::new(
                TlsConnector::from(tls.clone())
                    .connect(name, tcp)
                    .await
                    .map_err(|_| ProxyError::Upstream)?,
            )
        } else {
            Box::new(tcp)
        };
        let mut auth = None;
        let absolute_form = http_forward && config.protocol != ProxyProtocol::Socks5;
        match config.protocol {
            ProxyProtocol::Http | ProxyProtocol::Https => {
                auth = config.credentials.as_ref().map(|c| {
                    let mut value = HeaderValue::from_str(&format!(
                        "Basic {}",
                        base64::engine::general_purpose::STANDARD
                            .encode(format!("{}:{}", c.username, c.password))
                    ))
                    .expect("base64 header");
                    value.set_sensitive(true);
                    value
                });
                if !absolute_form {
                    let mut head = format!(
                        "CONNECT {} HTTP/1.1\r\nHost: {}\r\n",
                        target.authority(),
                        target.authority()
                    );
                    if let Some(auth) = &auth {
                        head.push_str("Proxy-Authorization: ");
                        head.push_str(auth.to_str().expect("base64 ASCII"));
                        head.push_str("\r\n");
                    }
                    head.push_str("\r\n");
                    io.write_all(head.as_bytes()).await?;
                    io.flush().await?;
                    let (head, stream) = read_head(io, self.max_header).await?;
                    let mut headers = [httparse::EMPTY_HEADER; 128];
                    let mut response = httparse::Response::new(&mut headers);
                    if !response.parse(&head).is_ok_and(|s| s.is_complete())
                        || !response.code.is_some_and(|code| (200..300).contains(&code))
                    {
                        return Err(ProxyError::Upstream);
                    }
                    io = stream;
                    auth = None;
                }
            }
            ProxyProtocol::Socks5 => {
                let methods: &[u8] = if config.credentials.is_some() {
                    &[5, 2, 0, 2]
                } else {
                    &[5, 1, 0]
                };
                io.write_all(methods).await?;
                io.flush().await?;
                let mut choice = [0; 2];
                io.read_exact(&mut choice).await?;
                if choice == [5, 2] {
                    let c = config.credentials.as_ref().ok_or(ProxyError::Upstream)?;
                    let mut auth = vec![1, c.username.len() as u8];
                    auth.extend(c.username.as_bytes());
                    auth.push(c.password.len() as u8);
                    auth.extend(c.password.as_bytes());
                    io.write_all(&auth).await?;
                    io.flush().await?;
                    io.read_exact(&mut choice).await?;
                    if choice != [1, 0] {
                        return Err(ProxyError::Upstream);
                    }
                } else if choice != [5, 0] {
                    return Err(ProxyError::Upstream);
                }
                let mut request = vec![5, 1, 0];
                request.extend(target.socks_bytes());
                io.write_all(&request).await?;
                io.flush().await?;
                let mut reply = [0; 4];
                io.read_exact(&mut reply).await?;
                if reply[0..3] != [5, 0, 0] {
                    return Err(ProxyError::Upstream);
                }
                let len = match reply[3] {
                    1 => 4,
                    4 => 16,
                    3 => io.read_u8().await? as usize,
                    _ => return Err(ProxyError::Upstream),
                };
                if len == 0 {
                    return Err(ProxyError::Upstream);
                }
                let mut bound = vec![0; len + 2];
                io.read_exact(&mut bound).await?;
            }
        }
        Ok(Connected {
            io,
            upstream_id: config.id.clone(),
            absolute_form,
            proxy_authorization: auth,
        })
    }
}

pub(crate) async fn read_head(io: BoxIo, max: usize) -> Result<(Vec<u8>, BoxIo), ProxyError> {
    let mut io = BufReader::new(io);
    let mut head = Vec::new();
    while head.len() < max {
        head.push(io.read_u8().await?);
        if head.ends_with(b"\r\n\r\n") {
            return Ok((head, Box::new(io)));
        }
    }
    Err(ProxyError::Upstream)
}
