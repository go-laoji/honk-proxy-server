//! Bounded, end-to-end health probes. Scheduling and persistence live in application/storage.
use bytes::Bytes;
use hps_config::{HealthConfig, ProbeProfile, ProxyProtocol, UpstreamConfig};
use hps_domain::{HealthTransport, HealthVerdict, IpFamily, ProbePhase, ProbeResult};
use hps_engine::{
    BoxIo, ProxyError,
    dial::{DialFailure, ProbePlan},
    target::Target,
};
use http::{Request, header};
use http_body_util::Empty;
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};
use std::{
    net::IpAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, pki_types::ServerName},
};

#[derive(Clone)]
pub struct PreparedProfile {
    pub profile: ProbeProfile,
    pub fingerprint: String,
    url: url::Url,
    host: String,
    tls: Option<Arc<ClientConfig>>,
    tls_name: String,
    resolver: Option<Arc<dyn hps_net::NameResolver>>,
}
impl PreparedProfile {
    pub fn prepare(profile: ProbeProfile) -> Result<Self, ProxyError> {
        let settings = HealthConfig {
            enabled: false,
            profiles: vec![profile.clone()],
            ..Default::default()
        };
        settings.validate().map_err(|_| ProxyError::InvalidTarget)?;
        let url = url::Url::parse(&profile.url).map_err(|_| ProxyError::InvalidTarget)?;
        let host = match url.host().ok_or(ProxyError::InvalidTarget)? {
            url::Host::Domain(host) => host.to_owned(),
            url::Host::Ipv4(ip) => ip.to_string(),
            url::Host::Ipv6(ip) => ip.to_string(),
        };
        let tls_name = profile
            .tls_server_name
            .clone()
            .unwrap_or_else(|| host.clone());
        let (tls, ca_fp) = if url.scheme() == "https" {
            let config = UpstreamConfig {
                id: "probe-target".into(),
                protocol: ProxyProtocol::Https,
                host: host.clone(),
                port: url
                    .port_or_known_default()
                    .ok_or(ProxyError::InvalidTarget)?,
                enabled: true,
                credentials: None,
                expires_at_unix: None,
                ca_certificate: profile.ca_certificate.clone(),
                tls_server_name: Some(tls_name.clone()),
            };
            let (tls, fp) = hps_engine::tls::client_with_fingerprint(&config)?;
            (Some(tls), fp)
        } else {
            (None, None)
        };
        let mut canonical = profile.clone();
        canonical.enabled = true;
        let mut hash = Sha256::new();
        hash.update(serde_json::to_vec(&canonical).map_err(|_| ProxyError::InvalidTarget)?);
        if let Some(fp) = ca_fp {
            hash.update(fp.as_bytes());
        }
        Ok(Self {
            profile,
            fingerprint: format!("{:x}", hash.finalize()),
            url,
            host,
            tls,
            tls_name,
            resolver: None,
        })
    }
    pub async fn bootstrap_ready(&self) -> bool {
        self.resolve().await.is_ok()
    }
    pub fn with_resolver(
        mut self,
        resolver: Arc<dyn hps_net::NameResolver>,
        fingerprint: &str,
    ) -> Self {
        let mut hash = Sha256::new();
        hash.update(self.fingerprint.as_bytes());
        hash.update(fingerprint.as_bytes());
        self.fingerprint = format!("{:x}", hash.finalize());
        self.resolver = Some(resolver);
        self
    }
    async fn resolve(&self) -> Result<Target, &'static str> {
        let port = self
            .url
            .port_or_known_default()
            .ok_or("invalid_target_port")?;
        let addresses: Result<Vec<IpAddr>, &'static str> =
            if let Ok(ip) = self.host.parse::<IpAddr>() {
                Ok(vec![ip])
            } else if let Some(resolver) = &self.resolver {
                resolver
                    .lookup_family(
                        self.host.clone(),
                        hps_net::ResolvePurpose::Health,
                        match self.profile.family {
                            IpFamily::Ipv4 => hps_net::AddressFamily::Ipv4,
                            IpFamily::Ipv6 => hps_net::AddressFamily::Ipv6,
                        },
                    )
                    .await
                    .map_err(|_| "probe_target_dns_failed")
            } else {
                tokio::net::lookup_host((self.host.as_str(), port))
                    .await
                    .map(|values| values.map(|a| a.ip()).collect())
                    .map_err(|_| "probe_target_dns_failed")
            };
        addresses
            .and_then(|values| {
                values
                    .into_iter()
                    .find(|a| {
                        matches!(
                            (a, self.profile.family),
                            (IpAddr::V4(_), IpFamily::Ipv4) | (IpAddr::V6(_), IpFamily::Ipv6)
                        )
                    })
                    .ok_or("probe_target_family_unavailable")
            })
            .and_then(|address| {
                Target::new(&address.to_string(), port).map_err(|_| "invalid_probe_target")
            })
    }
    pub async fn probe(&self, plan: &ProbePlan, settings: &HealthConfig) -> ProbeResult {
        let started = std::time::Instant::now();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(settings.timeout_ms);
        let mut result = ProbeResult {
            node_id: plan.config().id.clone(),
            identity: plan.identity().into(),
            profile_fingerprint: self.fingerprint.clone(),
            transport: self.profile.transport,
            family: self.profile.family,
            verdict: HealthVerdict::Unknown,
            phase: ProbePhase::Admission,
            error_code: None,
            endpoint_reachable: false,
            authenticated: None,
            tunnel_established: false,
            http_status: None,
            target_ok: None,
            latency_ms: 0,
            checked_at: unix_now(),
        };
        if self.profile.transport == HealthTransport::Udp {
            return finish(
                result,
                HealthVerdict::Unsupported,
                ProbePhase::Unsupported,
                "udp_not_compiled",
                started,
            );
        }
        if !plan.config().enabled {
            return finish(
                result,
                HealthVerdict::Skipped,
                ProbePhase::Admission,
                "node_disabled",
                started,
            );
        }
        let margin = Duration::from_secs(
            plan.minimum_lifetime()
                .saturating_add(settings.min_probe_margin_secs),
        )
        .saturating_add(Duration::from_millis(settings.timeout_ms));
        if let Some(remaining) = plan.remaining()
            && remaining <= margin
        {
            return finish(
                result,
                if remaining.is_zero() {
                    HealthVerdict::Expired
                } else {
                    HealthVerdict::Skipped
                },
                ProbePhase::Admission,
                "lease_margin_insufficient",
                started,
            );
        }
        let target = match tokio::time::timeout_at(deadline, self.resolve()).await {
            Ok(Ok(target)) => target,
            Ok(Err(code)) => {
                return finish(
                    result,
                    HealthVerdict::Unknown,
                    ProbePhase::TargetResolve,
                    code,
                    started,
                );
            }
            Err(_) => {
                return finish(
                    result,
                    HealthVerdict::Unknown,
                    ProbePhase::TargetResolve,
                    "probe_target_dns_timeout",
                    started,
                );
            }
        };
        if deadline.saturating_duration_since(tokio::time::Instant::now())
            < Duration::from_millis(10)
        {
            return finish(
                result,
                HealthVerdict::Skipped,
                ProbePhase::Local,
                "probe_budget_exhausted",
                started,
            );
        }
        let forward = self.url.scheme() == "http";
        let connected = match plan.connect_until(&target, forward, deadline).await {
            Ok(connected) => connected,
            Err(error) => return dial_failure(result, error, started),
        };
        result.endpoint_reachable = true;
        result.tunnel_established = !connected.absolute_form;
        result.authenticated = if connected.absolute_form {
            None
        } else {
            Some(true)
        };
        let absolute = connected.absolute_form;
        let auth = connected.proxy_authorization;
        let mut io: BoxIo = connected.io;
        if let Some(tls) = &self.tls {
            let name = match ServerName::try_from(self.tls_name.clone()) {
                Ok(name) => name,
                Err(_) => {
                    return finish(
                        result,
                        HealthVerdict::Unknown,
                        ProbePhase::Local,
                        "invalid_target_tls_name",
                        started,
                    );
                }
            };
            io = match tokio::time::timeout_at(
                deadline,
                TlsConnector::from(tls.clone()).connect(name, io),
            )
            .await
            {
                Ok(Ok(stream)) => Box::new(stream),
                Ok(Err(_)) => {
                    return finish(
                        result,
                        HealthVerdict::Degraded,
                        ProbePhase::TargetTls,
                        "target_tls_failed",
                        started,
                    );
                }
                Err(_) => {
                    return finish(
                        result,
                        HealthVerdict::Degraded,
                        ProbePhase::TargetTls,
                        "target_tls_timeout",
                        started,
                    );
                }
            };
        }
        let handshake = hyper::client::conn::http1::Builder::new()
            .max_buf_size(settings.max_header_bytes)
            .handshake(TokioIo::new(io));
        let (mut sender, connection) = match tokio::time::timeout_at(deadline, handshake).await {
            Ok(Ok(connection)) => connection,
            _ => {
                return finish(
                    result,
                    HealthVerdict::Degraded,
                    ProbePhase::TargetHttp,
                    "probe_http_handshake_failed",
                    started,
                );
            }
        };
        let _connection = ConnectionGuard(tokio::spawn(async move {
            let _ = connection.await;
        }));
        let host = match Target::new(&self.host, self.url.port_or_known_default().unwrap_or(443)) {
            Ok(host) => host.authority(),
            Err(_) => {
                return finish(
                    result,
                    HealthVerdict::Unknown,
                    ProbePhase::Local,
                    "invalid_probe_host",
                    started,
                );
            }
        };
        let path = if self.url.path().is_empty() {
            "/"
        } else {
            self.url.path()
        };
        let uri = if absolute {
            format!("http://{}{path}", target.authority())
        } else {
            path.to_owned()
        };
        let mut request = Request::builder()
            .method("GET")
            .uri(uri)
            .header(header::HOST, host)
            .header(header::CONNECTION, "close")
            .header(header::USER_AGENT, "honk-proxy-server-health/0.1");
        if let Some(auth) = auth {
            request = request.header(header::PROXY_AUTHORIZATION, auth);
        }
        let request = match request.body(Empty::<Bytes>::new()) {
            Ok(request) => request,
            Err(_) => {
                return finish(
                    result,
                    HealthVerdict::Unknown,
                    ProbePhase::Local,
                    "invalid_probe_request",
                    started,
                );
            }
        };
        let response = match tokio::time::timeout_at(deadline, sender.send_request(request)).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => {
                return finish(
                    result,
                    HealthVerdict::Degraded,
                    ProbePhase::TargetHttp,
                    "probe_http_response_failed",
                    started,
                );
            }
            Err(_) => {
                return finish(
                    result,
                    HealthVerdict::Degraded,
                    ProbePhase::TargetHttp,
                    "probe_http_response_timeout",
                    started,
                );
            }
        };
        let status = response.status().as_u16();
        result.http_status = Some(status);
        if absolute && status == 407 {
            result.authenticated = Some(false);
            return finish(
                result,
                HealthVerdict::Failed,
                ProbePhase::Authentication,
                "upstream_authentication_failed",
                started,
            );
        }
        result.authenticated = Some(true);
        result.target_ok = Some(
            self.profile
                .expected_status
                .map_or((200..400).contains(&status), |expected| expected == status),
        );
        if result.target_ok == Some(false) {
            result.error_code = Some(
                if matches!(status, 403 | 429) {
                    "target_rejected"
                } else {
                    "target_unexpected_status"
                }
                .into(),
            );
        }
        result.verdict = HealthVerdict::Healthy;
        result.phase = ProbePhase::Complete;
        result.latency_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        result
    }
}
struct ConnectionGuard(tokio::task::JoinHandle<()>);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}
fn finish(
    mut result: ProbeResult,
    verdict: HealthVerdict,
    phase: ProbePhase,
    code: &str,
    started: std::time::Instant,
) -> ProbeResult {
    result.verdict = verdict;
    result.phase = phase;
    result.error_code = Some(code.into());
    result.latency_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    result
}
fn dial_failure(
    mut result: ProbeResult,
    error: DialFailure,
    started: std::time::Instant,
) -> ProbeResult {
    result.endpoint_reachable = error.endpoint_reachable;
    let (verdict, code) = match error.error {
        ProxyError::Dns => (HealthVerdict::Unknown, "endpoint_dns_failed"),
        ProxyError::Timeout if error.phase == ProbePhase::Local => {
            (HealthVerdict::Unknown, "endpoint_dns_timeout")
        }
        ProxyError::LocalResource => {
            return finish(
                result,
                HealthVerdict::Unknown,
                ProbePhase::Local,
                "local_resource_exhausted",
                started,
            );
        }
        ProxyError::Endpoint => (HealthVerdict::Failed, "endpoint_connect_failed"),
        ProxyError::UpstreamTls => (HealthVerdict::Failed, "upstream_tls_failed"),
        ProxyError::Authentication => {
            result.authenticated = Some(false);
            (HealthVerdict::Failed, "upstream_authentication_failed")
        }
        ProxyError::Policy(_) => (HealthVerdict::Degraded, "proxy_target_policy"),
        ProxyError::TargetConnect => (HealthVerdict::Degraded, "proxy_target_connect_failed"),
        ProxyError::Timeout => (HealthVerdict::Failed, "proxy_handshake_timeout"),
        ProxyError::TlsConfig | ProxyError::InvalidTarget => {
            (HealthVerdict::Unknown, "local_probe_configuration")
        }
        _ => (HealthVerdict::Failed, "proxy_protocol_failed"),
    };
    finish(result, verdict, error.phase, code, started)
}
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
