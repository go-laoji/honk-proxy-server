//! Policy-directed DNS with explicit bootstrap and independently bounded transports.
pub mod server;
mod transport;
pub mod wire;
use hickory_proto::op::Message;
use hps_config::{DnsConfig, DnsProtocol, ProxyConfig};
use hps_net::{DnsConnector, NameResolver, NetworkError, NetworkFuture, ResolvePurpose};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, watch};
use transport::PreparedUpstream;

pub type Result<T> = std::result::Result<T, DnsError>;
#[derive(Debug, Clone, Copy, thiserror::Error, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DnsError {
    #[error("DNS configuration, trust roots or static egress is invalid")]
    Configuration,
    #[error("DNS service is disabled")]
    Disabled,
    #[error("DNS accepts one standard IN query with a supported record type")]
    InvalidQuery,
    #[error("upstream DNS response does not match the request or exceeds limits")]
    InvalidResponse,
    #[error("DNS upstream transport failed")]
    Transport,
    #[error("DNS static proxy egress failed")]
    Egress,
    #[error("DNS TLS verification or handshake failed")]
    Tls,
    #[error("DNS HTTPS exchange failed")]
    Http,
    #[error("system DNS resolution failed; no wire response or TTL is available")]
    System,
    #[error("system DNS only supports A and AAAA")]
    SystemQueryType,
    #[error("DNS query timed out")]
    Timeout,
    #[error("DNS concurrent query limit reached")]
    Capacity,
    #[error("DNS worker stopped")]
    Worker,
    #[error("DNS returned no address for this family")]
    NoAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookupInput {
    pub name: String,
    pub query_type: String,
    pub purpose: ResolvePurpose,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordView {
    pub name: String,
    pub query_type: String,
    pub ttl: u32,
    pub value: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookupResult {
    pub name: String,
    pub query_type: String,
    pub purpose: ResolvePurpose,
    pub upstream: String,
    pub route: Option<String>,
    pub protocol: DnsProtocol,
    pub egress: Option<String>,
    pub source: String,
    pub response_code: String,
    pub elapsed_ms: u64,
    pub remaining_ttl: Option<u32>,
    pub answers: Vec<RecordView>,
    pub authorities: Vec<RecordView>,
    pub dnssec_validated_locally: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub id: String,
    pub name: String,
    pub query_type: String,
    pub purpose: ResolvePurpose,
    pub upstream: String,
    pub egress: Option<String>,
    pub remaining_ttl: u32,
    pub negative: bool,
    pub bytes: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachePage {
    pub entries: Vec<CacheEntry>,
    pub next_after: Option<String>,
    pub total: usize,
    pub bytes: usize,
    pub epoch: u64,
    pub in_flight: usize,
    pub hits: u64,
    pub misses: u64,
    pub merged: u64,
    pub failures: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Overview<C> {
    pub running_revision: i64,
    pub configuration: C,
    pub listener: Option<std::net::SocketAddr>,
    pub cache: CachePage,
    pub linux_capture_active: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainContext {
    pub ip: IpAddr,
    pub domains: Vec<String>,
    pub source: String,
    pub ambiguous: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearInput {
    pub domain: Option<String>,
}

#[derive(Clone)]
pub struct Resolver(Arc<Inner>);
struct Inner {
    config: DnsConfig,
    upstreams: HashMap<String, PreparedUpstream>,
    fingerprint: String,
    slots: Arc<Semaphore>,
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    cache: HashMap<String, Cached>,
    flights: HashMap<(String, u64), FlightSender>,
    bytes: usize,
    epoch: u64,
    sequence: u64,
    hits: u64,
    misses: u64,
    merged: u64,
    failures: u64,
}
type FlightSender = watch::Sender<Option<Result<Arc<Resolved>>>>;
struct Cached {
    answer: Arc<Resolved>,
    until: Instant,
    touched: u64,
    bytes: usize,
}
struct Resolved {
    message: Message,
    started: Instant,
    upstream: String,
    route: Option<String>,
    name: String,
    query_type: String,
    purpose: ResolvePurpose,
    ttl: Option<u32>,
}
pub struct WireAnswer {
    pub message: Message,
    pub diagnostic: LookupResult,
}

impl Resolver {
    /// Performs only bounded local preparation. No network request occurs before activation.
    pub fn prepare(
        config: DnsConfig,
        proxy: &ProxyConfig,
        connector: Option<Arc<dyn DnsConnector>>,
    ) -> Result<Self> {
        config
            .validate(proxy)
            .map_err(|_| DnsError::Configuration)?;
        let mut hash = Sha256::new();
        hash.update(serde_json::to_vec(&config).map_err(|_| DnsError::Configuration)?);
        let mut upstreams = HashMap::new();
        for config in config.upstreams.iter().cloned() {
            let up = PreparedUpstream::prepare(config, connector.clone())?;
            hash.update(up.fingerprint.as_bytes());
            upstreams.insert(up.config.id.clone(), up);
        }
        Ok(Self(Arc::new(Inner {
            slots: Arc::new(Semaphore::new(config.max_parallel)),
            config,
            upstreams,
            fingerprint: format!("{:x}", hash.finalize()),
            state: Mutex::new(State::default()),
        })))
    }
    pub fn fingerprint(&self) -> &str {
        &self.0.fingerprint
    }
    pub fn configuration(&self) -> &DnsConfig {
        &self.0.config
    }
    pub async fn diagnose(&self, input: LookupInput) -> Result<LookupResult> {
        Ok(self
            .exchange(wire::query(&input.name, &input.query_type)?, input.purpose)
            .await?
            .diagnostic)
    }
    pub async fn exchange(
        &self,
        mut request: Message,
        purpose: ResolvePurpose,
    ) -> Result<WireAnswer> {
        if !self.0.config.enabled {
            return Err(DnsError::Disabled);
        }
        wire::validate_query(&request)?;
        let client_id = request.metadata.id;
        request.metadata.id = 0;
        let canonical_name = request.queries[0].name().to_lowercase();
        request.queries[0].set_name(canonical_name);
        let name = request.queries[0]
            .name()
            .to_ascii()
            .trim_end_matches('.')
            .to_owned();
        let query_type = request.queries[0].query_type().to_string();
        let route = self.0.config.routes.iter().find(|r| {
            r.domain.as_ref().is_none_or(|domain| {
                let domain = domain.trim_end_matches('.').to_ascii_lowercase();
                name == domain
                    || (r.suffix && name.strip_suffix(&domain).is_some_and(|s| s.ends_with('.')))
            }) && (r.query_types.is_empty() || r.query_types.contains(&query_type))
                && (r.purposes.is_empty() || r.purposes.contains(&purpose))
        });
        let upstream = route
            .map_or(self.0.config.default_upstream.as_str(), |r| {
                r.upstream.as_str()
            })
            .to_owned();
        let route = route.map(|r| r.id.clone());
        let up = self
            .0
            .upstreams
            .get(&upstream)
            .ok_or(DnsError::Configuration)?;
        let mut hash = Sha256::new();
        hash.update(self.fingerprint());
        hash.update(serde_json::to_vec(&purpose).map_err(|_| DnsError::InvalidQuery)?);
        hash.update(request.to_vec().map_err(|_| DnsError::InvalidQuery)?);
        let key = format!("{:x}", hash.finalize());
        let began = Instant::now();
        let (mut receiver, source) = {
            let mut state = self.0.state.lock().unwrap();
            state.prune();
            state.sequence += 1;
            let sequence = state.sequence;
            if let Some(cached) = state.cache.get_mut(&key) {
                cached.touched = sequence;
                let answer = cached.answer.clone();
                state.hits += 1;
                return Ok(self.render(&answer, client_id, "cache", began));
            }
            let epoch = state.epoch;
            if let Some(flight) = state.flights.get(&(key.clone(), epoch)) {
                let receiver = flight.subscribe();
                state.merged += 1;
                (receiver, "merged")
            } else {
                let permit = self
                    .0
                    .slots
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| DnsError::Capacity)?;
                let (sender, receiver) = watch::channel(None);
                state.flights.insert((key.clone(), epoch), sender.clone());
                state.misses += 1;
                let resolver = self.clone();
                // The owned job finishes even if the requesting browser/connection disappears.
                tokio::spawn(async move {
                    let _permit = permit;
                    let started = Instant::now();
                    let deadline = tokio::time::Instant::now()
                        + Duration::from_millis(resolver.0.config.timeout_ms);
                    let up = &resolver.0.upstreams[&upstream];
                    request.metadata.id = if up.config.protocol == DnsProtocol::Https {
                        0
                    } else {
                        Message::query().metadata.id
                    };
                    let result =
                        match tokio::time::timeout_at(deadline, up.exchange(&request, deadline))
                            .await
                        {
                            Ok(Ok(message)) => {
                                let ttl = if up.config.protocol == DnsProtocol::System {
                                    None
                                } else {
                                    wire::cache_ttl(&message)
                                };
                                Ok(Arc::new(Resolved {
                                    message,
                                    started,
                                    upstream,
                                    route,
                                    name,
                                    query_type,
                                    purpose,
                                    ttl,
                                }))
                            }
                            Ok(Err(e)) => Err(e),
                            Err(_) => Err(DnsError::Timeout),
                        };
                    {
                        let mut state = resolver.0.state.lock().unwrap();
                        state.flights.remove(&(key.clone(), epoch));
                        if let Ok(answer) = &result {
                            if state.epoch == epoch {
                                state.insert(key, answer.clone(), &resolver.0.config);
                            }
                        } else {
                            state.failures += 1;
                        }
                    }
                    sender.send_replace(Some(result));
                });
                (receiver, "upstream")
            }
        };
        let _ = up; // Selection was validated before claiming a bounded worker slot.
        loop {
            let result = receiver.borrow_and_update().clone();
            if let Some(result) = result {
                return result.map(|answer| self.render(&answer, client_id, source, began));
            }
            receiver.changed().await.map_err(|_| DnsError::Worker)?;
        }
    }
    fn render(&self, answer: &Resolved, id: u16, source: &str, began: Instant) -> WireAnswer {
        // Round elapsed time up: a downstream cache must not extend our monotonic expiry.
        let age = ceil_seconds(answer.started.elapsed());
        let message = wire::age(&answer.message, age, id);
        let records = |records: &[hickory_proto::rr::Record]| {
            records
                .iter()
                .map(|r| RecordView {
                    name: r.name.to_ascii(),
                    query_type: r.data.record_type().to_string(),
                    ttl: r.ttl,
                    value: r.data.to_string(),
                })
                .collect()
        };
        let up = &self.0.upstreams[&answer.upstream].config;
        let diagnostic = LookupResult {
            name: answer.name.clone(),
            query_type: answer.query_type.clone(),
            purpose: answer.purpose,
            upstream: answer.upstream.clone(),
            route: answer.route.clone(),
            protocol: up.protocol,
            egress: up.proxy_id.clone(),
            source: source.into(),
            response_code: format!("{:?}", message.metadata.response_code),
            elapsed_ms: began.elapsed().as_millis().min(u64::MAX as u128) as u64,
            remaining_ttl: answer.ttl.map(|ttl| ttl.saturating_sub(age)),
            answers: records(&message.answers),
            authorities: records(&message.authorities),
            dnssec_validated_locally: false,
        };
        WireAnswer {
            message,
            diagnostic,
        }
    }
    pub fn cache(&self, after: Option<&str>, limit: usize) -> Result<CachePage> {
        if !(1..=200).contains(&limit)
            || after.is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(DnsError::InvalidQuery);
        }
        let mut state = self.0.state.lock().unwrap();
        state.prune();
        let mut entries = state
            .cache
            .iter()
            .filter(|(key, _)| after.is_none_or(|after| key.as_str() > after))
            .map(|(id, c)| {
                let answer = &c.answer;
                CacheEntry {
                    id: id.clone(),
                    name: answer.name.clone(),
                    query_type: answer.query_type.clone(),
                    purpose: answer.purpose,
                    upstream: answer.upstream.clone(),
                    egress: self.0.upstreams[&answer.upstream].config.proxy_id.clone(),
                    remaining_ttl: answer
                        .ttl
                        .unwrap_or(0)
                        .saturating_sub(ceil_seconds(answer.started.elapsed())),
                    negative: answer.message.answers.is_empty()
                        || answer.message.metadata.response_code
                            == hickory_proto::op::ResponseCode::NXDomain,
                    bytes: c.bytes,
                }
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        let more = entries.len() > limit;
        entries.truncate(limit);
        let next_after = more.then(|| entries.last().unwrap().id.clone());
        Ok(CachePage {
            entries,
            next_after,
            total: state.cache.len(),
            bytes: state.bytes,
            epoch: state.epoch,
            in_flight: state.flights.len(),
            hits: state.hits,
            misses: state.misses,
            merged: state.merged,
            failures: state.failures,
        })
    }
    pub fn clear(&self, domain: Option<&str>) -> Result<usize> {
        let domain = domain.map(|s| s.trim_end_matches('.').to_ascii_lowercase());
        if domain.as_ref().is_some_and(|s| {
            s.is_empty() || s.len() > 253 || hickory_proto::rr::Name::from_ascii(s).is_err()
        }) {
            return Err(DnsError::InvalidQuery);
        }
        let mut state = self.0.state.lock().unwrap();
        state.epoch = state.epoch.wrapping_add(1);
        let before = state.cache.len();
        state.cache.retain(|_, c| {
            domain.as_ref().is_some_and(|d| {
                c.answer.name != *d
                    && !c
                        .answer
                        .name
                        .strip_suffix(d)
                        .is_some_and(|s| s.ends_with('.'))
            })
        });
        state.bytes = state.cache.values().map(|c| c.bytes).sum();
        Ok(before - state.cache.len())
    }
    /// Only unexpired answers to this exact queried name provide context. Additional
    /// records, external encrypted DNS and unobserved traffic never invent a domain.
    pub fn domain_context(&self, ip: IpAddr) -> DomainContext {
        let mut state = self.0.state.lock().unwrap();
        state.prune();
        let mut domains = BTreeSet::new();
        for cached in state.cache.values() {
            let answer = &cached.answer;
            if wire::addresses(&answer.message).contains(&ip) {
                domains.insert(answer.name.clone());
            }
        }
        let ambiguous = domains.len() > 1;
        DomainContext {
            ip,
            domains: domains.into_iter().take(64).collect(),
            source: "unexpired_local_dns_answers".into(),
            ambiguous,
        }
    }
}
impl NameResolver for Resolver {
    fn lookup(&self, host: String, purpose: ResolvePurpose) -> NetworkFuture<'_, Vec<IpAddr>> {
        Box::pin(async move {
            if let Ok(ip) = host.parse() {
                return Ok(vec![ip]);
            }
            let v4 = self
                .exchange(
                    wire::query(&host, "A").map_err(|_| NetworkError("dns_invalid_name"))?,
                    purpose,
                )
                .await;
            let v6 = self
                .exchange(
                    wire::query(&host, "AAAA").map_err(|_| NetworkError("dns_invalid_name"))?,
                    purpose,
                )
                .await;
            let ips = v4
                .into_iter()
                .chain(v6)
                .flat_map(|r| wire::addresses(&r.message))
                .collect::<Vec<_>>();
            if ips.is_empty() {
                Err(NetworkError("dns_no_address"))
            } else {
                Ok(ips)
            }
        })
    }
    fn lookup_family(
        &self,
        host: String,
        purpose: ResolvePurpose,
        family: hps_net::AddressFamily,
    ) -> NetworkFuture<'_, Vec<IpAddr>> {
        Box::pin(async move {
            if let Ok(ip) = host.parse::<IpAddr>() {
                return if family.matches(ip) {
                    Ok(vec![ip])
                } else {
                    Err(NetworkError("dns_family_unavailable"))
                };
            }
            let kind = match family {
                hps_net::AddressFamily::Ipv4 => "A",
                hps_net::AddressFamily::Ipv6 => "AAAA",
            };
            let query = wire::query(&host, kind).map_err(|_| NetworkError("dns_invalid_name"))?;
            let answer = self
                .exchange(query, purpose)
                .await
                .map_err(|_| NetworkError("dns_resolution_failed"))?;
            let ips = wire::addresses(&answer.message)
                .into_iter()
                .filter(|ip| family.matches(*ip))
                .collect::<Vec<_>>();
            if ips.is_empty() {
                Err(NetworkError("dns_family_unavailable"))
            } else {
                Ok(ips)
            }
        })
    }
}
fn ceil_seconds(duration: Duration) -> u32 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() > 0))
        .min(u32::MAX as u64) as u32
}
impl State {
    fn prune(&mut self) {
        self.cache.retain(|_, entry| entry.until > Instant::now());
        self.bytes = self.cache.values().map(|c| c.bytes).sum();
    }
    fn insert(&mut self, key: String, answer: Arc<Resolved>, config: &DnsConfig) {
        let Some(ttl) = answer.ttl else { return };
        let until = answer.started + Duration::from_secs(u64::from(ttl));
        if until <= Instant::now() || config.cache_entries == 0 {
            return;
        }
        // Decoded messages can be larger than their compressed wire representation.
        let records = answer
            .message
            .answers
            .iter()
            .chain(&answer.message.authorities)
            .chain(&answer.message.additionals);
        let bytes = 1024
            + answer.message.to_vec().map_or(65535, |m| m.len())
            + records
                .map(|r| r.to_string().len() + std::mem::size_of::<hickory_proto::rr::Record>())
                .sum::<usize>();
        if bytes > config.cache_bytes {
            return;
        }
        self.prune();
        while self.cache.len() >= config.cache_entries || self.bytes + bytes > config.cache_bytes {
            let Some(key) = self
                .cache
                .iter()
                .min_by_key(|(_, c)| c.touched)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.bytes -= self.cache.remove(&key).unwrap().bytes;
        }
        self.sequence += 1;
        if let Some(old) = self.cache.insert(
            key,
            Cached {
                answer,
                until,
                touched: self.sequence,
                bytes,
            },
        ) {
            self.bytes -= old.bytes;
        }
        self.bytes += bytes;
    }
}
