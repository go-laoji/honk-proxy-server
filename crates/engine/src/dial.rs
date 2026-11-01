use crate::{BoxIo, ProxyError, target::Target, tls};
use base64::Engine;
use hps_config::{ProxyConfig, ProxyProtocol, UpstreamConfig};
use hps_domain::ProbePhase;
use http::HeaderValue;
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, SystemTime},
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
    pub(crate) identity: String,
    tls: Option<Arc<ClientConfig>>,
    pub(crate) source: Option<(String, String)>,
    pub(crate) deadline: Option<std::time::Instant>,
    pub(crate) minimum_lifetime: u64,
}
impl Upstream {
    pub(crate) fn prepare(config: UpstreamConfig) -> Result<Self, ProxyError> {
        let (tls, ca_fingerprint) = if config.protocol == ProxyProtocol::Https {
            let (tls, fp) = tls::client_with_fingerprint(&config)?;
            (Some(tls), fp)
        } else {
            (None, None)
        };
        let mut canonical = config.clone();
        canonical.id.clear();
        canonical.enabled = true;
        canonical.expires_at_unix = None;
        let mut hash = Sha256::new();
        hash.update(serde_json::to_vec(&canonical).map_err(|_| ProxyError::TlsConfig)?);
        if let Some(fp) = ca_fingerprint {
            hash.update(fp.as_bytes());
        }
        let identity = format!("{:x}", hash.finalize());
        Ok(Self {
            config,
            identity,
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
    pub(crate) health: Arc<crate::health::HealthRegistry>,
    policy: std::sync::RwLock<crate::health::HealthPolicy>,
    sources: std::sync::RwLock<std::collections::HashMap<String, String>>,
    selector: crate::selection::Selector,
    timeout: Duration,
    min_lifetime: u64,
    max_header: usize,
    resolver: std::sync::RwLock<Option<Arc<dyn hps_net::NameResolver>>>,
}
#[derive(Debug)]
pub struct DialFailure {
    pub error: ProxyError,
    pub phase: ProbePhase,
    pub endpoint_reachable: bool,
}
struct Progress {
    phase: std::sync::Mutex<ProbePhase>,
    endpoint: std::sync::atomic::AtomicBool,
}
impl Progress {
    fn new() -> Self {
        Self {
            phase: std::sync::Mutex::new(ProbePhase::EndpointConnect),
            endpoint: std::sync::atomic::AtomicBool::new(false),
        }
    }
    fn stage(&self, phase: ProbePhase) {
        *self.phase.lock().unwrap() = phase;
    }
    fn failure(&self, error: ProxyError) -> DialFailure {
        DialFailure {
            error,
            phase: *self.phase.lock().unwrap(),
            endpoint_reachable: self.endpoint.load(Ordering::Relaxed),
        }
    }
}
#[derive(Clone)]
pub struct ProbePlan {
    dialer: Arc<Dialer>,
    upstream: Arc<Upstream>,
}
impl ProbePlan {
    pub fn config(&self) -> &UpstreamConfig {
        &self.upstream.config
    }
    pub fn identity(&self) -> &str {
        &self.upstream.identity
    }
    pub fn provider_id(&self) -> Option<&str> {
        self.upstream.source.as_ref().map(|(id, _)| id.as_str())
    }
    pub fn minimum_lifetime(&self) -> u64 {
        self.dialer.min_lifetime.max(self.upstream.minimum_lifetime)
    }
    pub fn remaining(&self) -> Option<Duration> {
        remaining(&self.upstream)
    }
    /// Bypasses selection/health and production demand, preserving prepared trust roots.
    pub async fn connect(
        &self,
        target: &Target,
        http_forward: bool,
    ) -> Result<Connected, DialFailure> {
        self.connect_until(
            target,
            http_forward,
            tokio::time::Instant::now() + self.dialer.timeout,
        )
        .await
    }
    pub async fn connect_until(
        &self,
        target: &Target,
        http_forward: bool,
        deadline: tokio::time::Instant,
    ) -> Result<Connected, DialFailure> {
        let progress = Progress::new();
        let deadline = deadline.min(tokio::time::Instant::now() + self.dialer.timeout);
        tokio::time::timeout_at(
            deadline,
            self.dialer
                .connect_selected(&self.upstream, target, http_forward, &progress),
        )
        .await
        .map_err(|_| progress.failure(ProxyError::Timeout))?
        .map_err(|e| progress.failure(e))
    }
}
fn remaining(upstream: &Upstream) -> Option<Duration> {
    let wall = upstream.config.expires_at_unix.map(|expiry| {
        std::time::UNIX_EPOCH
            .checked_add(Duration::from_secs(expiry))
            .and_then(|date| date.duration_since(SystemTime::now()).ok())
            .unwrap_or(Duration::ZERO)
    });
    match (wall, upstream.deadline) {
        (Some(wall), Some(deadline)) => {
            Some(wall.min(deadline.saturating_duration_since(std::time::Instant::now())))
        }
        (wall, None) => wall,
        (None, Some(deadline)) => {
            Some(deadline.saturating_duration_since(std::time::Instant::now()))
        }
    }
}
pub struct Connected {
    pub io: BoxIo,
    pub upstream_id: String,
    pub identity: String,
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
            health: Arc::new(crate::health::HealthRegistry::default()),
            policy: Default::default(),
            sources: std::sync::RwLock::new(std::collections::HashMap::new()),
            selector: crate::selection::Selector::new(config.selection.clone()),
            timeout: Duration::from_millis(config.limits.connect_timeout_ms),
            min_lifetime: config.limits.min_remaining_lifetime_secs,
            max_header: config.limits.max_header_bytes,
            resolver: Default::default(),
        })
    }
    pub fn probe_plans(self: &Arc<Self>) -> Vec<ProbePlan> {
        let dynamic = self.pool.snapshot();
        self.upstreams
            .iter()
            .chain(dynamic.iter())
            .filter(|upstream| {
                upstream
                    .source
                    .as_ref()
                    .is_none_or(|(id, fp)| self.sources.read().unwrap().get(id) == Some(fp))
            })
            .map(|upstream| ProbePlan {
                dialer: self.clone(),
                upstream: upstream.clone(),
            })
            .collect()
    }
    pub fn configure_health(&self, policy: crate::health::HealthPolicy) {
        *self.policy.write().unwrap() = policy;
    }
    pub fn configure_resolver(&self, resolver: Option<Arc<dyn hps_net::NameResolver>>) {
        *self.resolver.write().unwrap() = resolver;
    }
    pub fn health_usable(&self, identity: &str, family: Option<hps_domain::IpFamily>) -> bool {
        self.health
            .usable(identity, &self.policy.read().unwrap(), family)
    }
    pub fn traffic_feedback(
        &self,
        node_id: &str,
        identity: &str,
        phase: ProbePhase,
        code: &str,
        global: bool,
    ) {
        self.health.traffic(
            node_id,
            identity,
            phase,
            code,
            global,
            self.policy.read().unwrap().traffic_failure_threshold,
        );
    }
    pub(crate) fn prepared_static_fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        for upstream in &self.upstreams {
            hash.update(upstream.identity.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
    pub fn set_sources(&self, sources: std::collections::HashMap<String, String>) {
        *self.sources.write().unwrap() = sources;
    }
    pub fn eligible_count(&self) -> usize {
        self.selection(&hps_domain::SelectionQuery::default())
            .eligible
    }
    pub fn group_last_choices(&self) -> std::collections::HashMap<String, String> {
        self.selector.last()
    }
    pub fn selection(&self, query: &hps_domain::SelectionQuery) -> hps_domain::SelectionDecision {
        self.select(query, false).0
    }
    fn select(
        &self,
        query: &hps_domain::SelectionQuery,
        commit: bool,
    ) -> (hps_domain::SelectionDecision, Option<Arc<Upstream>>) {
        let dynamic = self.pool.snapshot();
        let family = query
            .destination_host
            .as_ref()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .map(|ip| {
                if ip.is_ipv4() {
                    hps_domain::IpFamily::Ipv4
                } else {
                    hps_domain::IpFamily::Ipv6
                }
            });
        let policy = self.policy.read().unwrap().clone();
        let choices = self
            .upstreams
            .iter()
            .chain(dynamic.iter())
            .map(|upstream| {
                let sampled = std::time::Instant::now();
                let remaining = remaining(upstream);
                let mut reasons = vec![];
                if !upstream.config.enabled {
                    reasons.push("disabled".into());
                }
                if upstream
                    .source
                    .as_ref()
                    .is_some_and(|(id, fp)| self.sources.read().unwrap().get(id) != Some(fp))
                {
                    reasons.push("source_reconfigured".into());
                }
                let minimum = self.min_lifetime.max(upstream.minimum_lifetime);
                if let Some(time) = remaining
                    && time <= Duration::from_secs(minimum)
                {
                    reasons.push(
                        if time.is_zero() {
                            "expired"
                        } else {
                            "insufficient_lifetime"
                        }
                        .into(),
                    );
                }
                if !self.health.usable(&upstream.identity, &policy, family) {
                    reasons.push("health_unavailable".into());
                }
                crate::selection::Choice {
                    upstream: upstream.clone(),
                    reasons,
                    remaining,
                    minimum_lifetime: minimum,
                    sampled,
                    latency: self.health.latency(&upstream.identity, &policy, family),
                }
            })
            .collect();
        self.selector.decide(choices, query, commit)
    }
    /// Select exactly once. Neither failed handshakes nor business data are retried.
    pub async fn connect(
        &self,
        target: &Target,
        http_forward: bool,
    ) -> Result<Connected, ProxyError> {
        self.connect_context(target, http_forward, &hps_domain::SelectionQuery::default())
            .await
    }
    pub async fn connect_context(
        &self,
        target: &Target,
        http_forward: bool,
        context: &hps_domain::SelectionQuery,
    ) -> Result<Connected, ProxyError> {
        let demand = self.demand.begin();
        let mut query = context.clone();
        query.destination_host = Some(target.host.clone());
        query.destination_port = Some(target.port);
        let (_, upstream) = self.select(&query, true);
        let upstream = upstream.ok_or(ProxyError::NoUpstream)?;
        let progress = Progress::new();
        let outcome = tokio::time::timeout(
            self.timeout,
            self.connect_selected(&upstream, target, http_forward, &progress),
        )
        .await
        .unwrap_or(Err(ProxyError::Timeout));
        if let Err(error) = &outcome {
            let phase = *progress.phase.lock().unwrap();
            let (code, global) = match error {
                ProxyError::LocalResource => ("local_resource_exhausted", false),
                ProxyError::Endpoint => ("endpoint_connect_failed", true),
                ProxyError::Dns => ("endpoint_dns_failed", false),
                ProxyError::UpstreamTls => ("upstream_tls_failed", true),
                ProxyError::Authentication => ("upstream_authentication_failed", true),
                ProxyError::Timeout => (
                    "proxy_handshake_timeout",
                    matches!(
                        phase,
                        ProbePhase::EndpointConnect
                            | ProbePhase::UpstreamTls
                            | ProbePhase::Authentication
                    ),
                ),
                ProxyError::Policy(_) => ("proxy_target_policy", false),
                ProxyError::TargetConnect => ("proxy_target_connect_failed", false),
                _ => ("proxy_protocol_failed", false),
            };
            self.traffic_feedback(&upstream.config.id, &upstream.identity, phase, code, global);
        }
        if let Ok(connected) = &outcome {
            self.traffic_feedback(
                &connected.upstream_id,
                &connected.identity,
                if connected.absolute_form {
                    ProbePhase::EndpointConnect
                } else {
                    ProbePhase::ProxyTunnel
                },
                if connected.absolute_form {
                    "endpoint_connected"
                } else {
                    "tunnel_established"
                },
                false,
            );
        }
        outcome.map(|mut connected| {
            connected.io = demand.wrap(connected.io);
            connected
        })
    }

    async fn connect_selected(
        &self,
        upstream: &Upstream,
        target: &Target,
        http_forward: bool,
        progress: &Progress,
    ) -> Result<Connected, ProxyError> {
        let config = &upstream.config;
        let resolver = self.resolver.read().unwrap().clone();
        let tcp = if config.host.parse::<std::net::IpAddr>().is_err()
            && let Some(resolver) = resolver
        {
            progress.stage(ProbePhase::Local);
            let addresses = resolver
                .lookup(config.host.clone(), hps_net::ResolvePurpose::Bootstrap)
                .await
                .map_err(|_| ProxyError::Dns)?;
            let addresses = addresses
                .into_iter()
                .map(|ip| std::net::SocketAddr::new(ip, config.port))
                .collect::<Vec<_>>();
            progress.stage(ProbePhase::EndpointConnect);
            TcpStream::connect(addresses.as_slice())
                .await
                .map_err(socket_error)?
        } else {
            TcpStream::connect((config.host.as_str(), config.port))
                .await
                .map_err(socket_error)?
        };
        progress.endpoint.store(true, Ordering::Relaxed);
        tcp.set_nodelay(true)?;
        let mut io: BoxIo = if let Some(tls) = &upstream.tls {
            progress.stage(ProbePhase::UpstreamTls);
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
                    .map_err(|_| ProxyError::UpstreamTls)?,
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
                    progress.stage(ProbePhase::ProxyTunnel);
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
                    if !response.parse(&head).is_ok_and(|s| s.is_complete()) {
                        return Err(ProxyError::Upstream);
                    }
                    match response.code {
                        Some(200..=299) => {}
                        Some(407) => {
                            progress.stage(ProbePhase::Authentication);
                            return Err(ProxyError::Authentication);
                        }
                        Some(code) => return Err(ProxyError::Policy(code)),
                        None => return Err(ProxyError::Upstream),
                    }
                    io = stream;
                    auth = None;
                }
            }
            ProxyProtocol::Socks5 => {
                progress.stage(ProbePhase::Authentication);
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
                    let c = config
                        .credentials
                        .as_ref()
                        .ok_or(ProxyError::Authentication)?;
                    let mut auth = vec![1, c.username.len() as u8];
                    auth.extend(c.username.as_bytes());
                    auth.push(c.password.len() as u8);
                    auth.extend(c.password.as_bytes());
                    io.write_all(&auth).await?;
                    io.flush().await?;
                    io.read_exact(&mut choice).await?;
                    if choice != [1, 0] {
                        return Err(ProxyError::Authentication);
                    }
                } else if choice != [5, 0] {
                    return Err(ProxyError::Authentication);
                }
                progress.stage(ProbePhase::ProxyTunnel);
                let mut request = vec![5, 1, 0];
                request.extend(target.socks_bytes());
                io.write_all(&request).await?;
                io.flush().await?;
                let mut reply = [0; 4];
                io.read_exact(&mut reply).await?;
                if reply[0..3] != [5, 0, 0] {
                    return Err(ProxyError::TargetConnect);
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
            identity: upstream.identity.clone(),
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

fn socket_error(error: std::io::Error) -> ProxyError {
    #[cfg(unix)]
    if error.raw_os_error().is_some_and(|code| {
        [libc::EMFILE, libc::ENFILE, libc::ENOMEM, libc::ENOBUFS].contains(&code)
    }) {
        return ProxyError::LocalResource;
    }
    if error.kind() == std::io::ErrorKind::OutOfMemory {
        ProxyError::LocalResource
    } else {
        ProxyError::Endpoint
    }
}
#[cfg(all(test, unix))]
mod resource_tests {
    use super::*;
    #[test]
    fn local_exhaustion_is_separate_from_remote_refusal() {
        for code in [libc::EMFILE, libc::ENFILE, libc::ENOMEM, libc::ENOBUFS] {
            assert!(matches!(
                socket_error(std::io::Error::from_raw_os_error(code)),
                ProxyError::LocalResource
            ));
        }
        assert!(matches!(
            socket_error(std::io::Error::from_raw_os_error(libc::ECONNREFUSED)),
            ProxyError::Endpoint
        ));
    }
}
