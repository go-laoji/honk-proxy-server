use crate::{
    BoxIo,
    capacity::{Capacity, Permit},
    dial::Dialer,
    traffic::{ActiveIo, Activity, Budget, Pacer},
};
use hps_config::{InboundConfig, ProxyConfig, ProxyProtocol};
use hps_domain::{CertificateStatus, InboundStatus, ProxyStatus, ResourceLimits};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    future::Future,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("invalid proxy configuration")]
    Configuration,
    #[error("invalid TLS configuration")]
    Tls,
    #[error("inbound not found")]
    NotFound,
    #[error("daemon is shutting down")]
    ShuttingDown,
    #[error("inbound listen address is unavailable")]
    Bind,
}

pub struct Engine {
    entries: BTreeMap<String, Arc<Entry>>,
    preparation_fingerprint: String,
    dialer: Arc<Dialer>,
    limits: Arc<ResourceLimits>,
    capacity: Arc<Capacity>,
    bandwidth: Arc<Pacer>,
    cancel: CancellationToken,
    generation: std::sync::atomic::AtomicI64,
}
struct Entry {
    config: InboundConfig,
    tls: Option<TlsAcceptor>,
    certificate: Option<CertificateStatus>,
    control: tokio::sync::Mutex<Option<Run>>,
    listener: Mutex<Option<Arc<TcpListener>>>,
    tracker: TaskTracker,
    running: AtomicBool,
    address: Mutex<Option<SocketAddr>>,
    last_error: Mutex<Option<String>>,
    capacity: Arc<Capacity>,
    bandwidth: Arc<Pacer>,
    accepted: AtomicU64,
    global_rejected: AtomicU64,
    inbound_rejected: AtomicU64,
    failed: AtomicU64,
    idle_timeouts: AtomicU64,
}
struct Admission {
    _global: Permit,
    _inbound: Permit,
}
struct Run {
    accept: CancellationToken,
    sessions: CancellationToken,
    accept_stopped: CancellationToken,
}
#[derive(Clone)]
pub(crate) struct Session {
    pub dialer: Arc<Dialer>,
    pub limits: Arc<ResourceLimits>,
    pub budget: Budget,
    pub activity: Activity,
    pub cancel: CancellationToken,
    pub tracker: TaskTracker,
    pub selection_context: hps_domain::SelectionQuery,
    entry: Arc<Entry>,
    _admission: Arc<Admission>,
}
impl Session {
    pub fn handshake_timeout(&self) -> Duration {
        Duration::from_millis(self.limits.handshake_timeout_ms)
    }
    pub fn failed(&self) {
        self.entry.failed.fetch_add(1, Ordering::Relaxed);
    }
    pub fn spawn_io(&self, future: impl Future<Output = ()> + Send + 'static) {
        let session = self.clone();
        self.tracker.spawn(async move {
            session.run(future).await;
        });
    }
    async fn run(&self, future: impl Future<Output = ()>) {
        tokio::select! {
            biased;
            _=self.cancel.cancelled()=>{},
            _=self.activity.idle(Duration::from_millis(self.limits.idle_timeout_ms))=>{self.entry.idle_timeouts.fetch_add(1,Ordering::Relaxed);},
            _=future=>{},
        }
    }
}

impl Engine {
    /// File reads and TLS parsing happen before serving. Call on a blocking task
    /// when constructing a runtime from an async management request.
    pub fn new(config: ProxyConfig) -> Result<Arc<Self>, EngineError> {
        Self::with_previous(config, None)
    }
    pub fn with_previous(
        config: ProxyConfig,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, EngineError> {
        config.validate().map_err(|_| EngineError::Configuration)?;
        let mut dialer = Dialer::new(&config).map_err(|_| EngineError::Tls)?;
        if let Some(old) = previous {
            dialer.preserve_static_deadlines(&old.dialer);
            dialer.pool = old.dialer.pool.clone();
            dialer.demand = old.dialer.demand.clone();
            dialer.health = old.dialer.health.clone();
        }
        let mut preparation = Sha256::new();
        preparation.update(serde_json::to_vec(&config).map_err(|_| EngineError::Configuration)?);
        preparation.update(dialer.prepared_static_fingerprint().as_bytes());
        let dialer = Arc::new(dialer);
        let mut entries = BTreeMap::new();
        for config in config.inbounds {
            let old = previous.and_then(|engine| engine.entries.get(&config.id));
            let (tls, certificate) = match &config.tls {
                Some(tls) => {
                    let (tls, info) = crate::tls::server(tls).map_err(|_| EngineError::Tls)?;
                    preparation.update(info.fingerprint.as_bytes());
                    (
                        Some(TlsAcceptor::from(tls)),
                        Some(CertificateStatus {
                            not_before_unix: info.not_before_unix,
                            not_after_unix: info.not_after_unix,
                            valid_now: info.valid_now,
                        }),
                    )
                }
                None => (None, None),
            };
            let entry = Entry {
                capacity: old
                    .map_or_else(|| Arc::new(Capacity::default()), |old| old.capacity.clone()),
                bandwidth: old
                    .filter(|old| old.config.bytes_per_second == config.bytes_per_second)
                    .map_or_else(
                        || Pacer::new(config.bytes_per_second),
                        |old| old.bandwidth.clone(),
                    ),
                config,
                tls,
                certificate,
                control: tokio::sync::Mutex::new(None),
                listener: Mutex::new(None),
                tracker: TaskTracker::new(),
                running: AtomicBool::new(false),
                address: Mutex::new(None),
                last_error: Mutex::new(None),
                accepted: AtomicU64::new(0),
                global_rejected: AtomicU64::new(0),
                inbound_rejected: AtomicU64::new(0),
                failed: AtomicU64::new(0),
                idle_timeouts: AtomicU64::new(0),
            };
            entries.insert(entry.config.id.clone(), Arc::new(entry));
        }
        Ok(Arc::new(Self {
            preparation_fingerprint: format!("{:x}", preparation.finalize()),
            entries,
            dialer,
            capacity: previous
                .map_or_else(|| Arc::new(Capacity::default()), |old| old.capacity.clone()),
            bandwidth: previous
                .filter(|old| old.limits.bytes_per_second == config.limits.bytes_per_second)
                .map_or_else(
                    || Pacer::new(config.limits.bytes_per_second),
                    |old| old.bandwidth.clone(),
                ),
            limits: Arc::new(config.limits),
            cancel: CancellationToken::new(),
            generation: std::sync::atomic::AtomicI64::new(0),
        }))
    }
    pub fn preparation_fingerprint(&self) -> &str {
        &self.preparation_fingerprint
    }
    pub fn health_registry(&self) -> Arc<crate::health::HealthRegistry> {
        self.dialer.health.clone()
    }
    pub fn configure_health(&self, policy: crate::health::HealthPolicy) {
        self.dialer.configure_health(policy);
    }
    pub fn configure_resolver(&self, resolver: Option<Arc<dyn hps_net::NameResolver>>) {
        self.dialer.configure_resolver(resolver);
    }
    pub fn health_usable(&self, identity: &str) -> bool {
        self.dialer.health_usable(identity, None)
    }
    pub fn probe_plans(&self) -> Vec<crate::dial::ProbePlan> {
        self.dialer.probe_plans()
    }
    pub fn selection(&self, query: &hps_domain::SelectionQuery) -> hps_domain::SelectionDecision {
        self.dialer.selection(query)
    }
    pub fn group_last_choices(&self) -> std::collections::HashMap<String, String> {
        self.dialer.group_last_choices()
    }
    pub fn has_active_demand(&self) -> bool {
        self.dialer.demand.active()
    }
    pub fn demand_age(&self) -> Option<Duration> {
        self.dialer.demand.age()
    }
    pub fn dynamic_pool(&self) -> Arc<crate::pool::DynamicPool> {
        self.dialer.pool.clone()
    }
    pub fn set_provider_sources(&self, sources: std::collections::HashMap<String, String>) {
        self.dialer.set_sources(sources);
    }
    pub async fn start_enabled(self: &Arc<Self>) -> Result<(), EngineError> {
        for entry in self.entries.values().filter(|entry| entry.config.enabled) {
            if let Err(error) = self.start(&entry.config.id).await {
                self.shutdown().await;
                return Err(error);
            }
        }
        Ok(())
    }
    /// Bind all new sockets without accepting traffic. Existing sockets can be
    /// shared during a generation handoff, including an unchanged ephemeral ID.
    pub async fn prepare_enabled(&self, previous: Option<&Arc<Self>>) -> Result<(), EngineError> {
        for entry in self.entries.values().filter(|entry| entry.config.enabled) {
            let existing = previous.and_then(|previous| {
                previous.entries.values().find_map(|old| {
                    let same = old.config.listen == entry.config.listen
                        && (entry.config.listen.port() != 0 || old.config.id == entry.config.id);
                    if same {
                        old.listener.lock().unwrap().clone()
                    } else {
                        None
                    }
                })
            });
            let listener = match existing {
                Some(listener) => listener,
                None => Arc::new(
                    TcpListener::bind(entry.config.listen)
                        .await
                        .map_err(|_| EngineError::Bind)?,
                ),
            };
            listener.local_addr().map_err(|_| EngineError::Bind)?;
            *entry.listener.lock().unwrap() = Some(listener);
        }
        Ok(())
    }
    /// Stop accepting, retaining existing sessions and their generation budget.
    pub async fn drain_acceptors(&self) {
        self.dialer.retire_selection();
        for entry in self.entries.values() {
            let control = entry.control.lock().await;
            if let Some(run) = control.as_ref() {
                run.accept.cancel();
                run.accept_stopped.cancelled().await;
            }
        }
    }
    pub fn is_idle(&self) -> bool {
        self.entries.values().all(|entry| entry.tracker.is_empty())
    }
    pub fn set_generation(&self, id: i64) {
        let _ = self
            .generation
            .compare_exchange(0, id, Ordering::AcqRel, Ordering::Acquire);
    }
    pub fn generation(&self) -> i64 {
        self.generation.load(Ordering::Acquire)
    }
    pub async fn start(self: &Arc<Self>, id: &str) -> Result<InboundStatus, EngineError> {
        let entry = self.entries.get(id).ok_or(EngineError::NotFound)?.clone();
        let mut control = entry.control.lock().await;
        if self.cancel.is_cancelled() {
            return Err(EngineError::ShuttingDown);
        }
        if entry.running.load(Ordering::Acquire) {
            return Ok(entry.status());
        }
        // Await cleanup of any previous run before reusing its tracker/address.
        if let Some(previous) = control.take() {
            previous.accept.cancel();
            previous.sessions.cancel();
            entry.tracker.close();
            entry.tracker.wait().await;
        }
        let prepared = entry.listener.lock().unwrap().take();
        let listener = match prepared {
            Some(listener) => listener,
            None => match TcpListener::bind(entry.config.listen).await {
                Ok(listener) => Arc::new(listener),
                Err(_) => {
                    *entry.last_error.lock().unwrap() = Some("listen_failed".into());
                    return Err(EngineError::Bind);
                }
            },
        };
        let address = listener.local_addr().map_err(|_| EngineError::Bind)?;
        let cancel = self.cancel.child_token();
        let sessions = self.cancel.child_token();
        let stopped = CancellationToken::new();
        *control = Some(Run {
            accept: cancel.clone(),
            sessions: sessions.clone(),
            accept_stopped: stopped.clone(),
        });
        *entry.listener.lock().unwrap() = Some(listener.clone());
        *entry.address.lock().unwrap() = Some(address);
        *entry.last_error.lock().unwrap() = None;
        entry.running.store(true, Ordering::Release);
        entry.tracker.reopen();
        let engine = self.clone();
        let run = entry.clone();
        entry.tracker.spawn(async move {
            loop {
                let accepted=tokio::select! { biased; _=cancel.cancelled()=>break, accepted=listener.accept()=>accepted };
                let (tcp,peer)=match accepted {
                    Ok(accepted)=>accepted,
                    Err(_)=>{*run.last_error.lock().unwrap()=Some("accept_failed".into());cancel.cancel();break;}
                };
                let global=match engine.capacity.acquire(engine.limits.max_connections) { Some(permit)=>permit,None=>{run.global_rejected.fetch_add(1,Ordering::Relaxed);continue;} };
                let inbound=match run.capacity.acquire(run.config.max_connections) { Some(permit)=>permit,None=>{run.inbound_rejected.fetch_add(1,Ordering::Relaxed);continue;} };
                run.accepted.fetch_add(1,Ordering::Relaxed);
                let activity=Activity::new();
                let session=Session { dialer:engine.dialer.clone(),limits:engine.limits.clone(),budget:Budget::new(engine.bandwidth.clone(),run.bandwidth.clone(),activity.clone()),activity,cancel:sessions.child_token(),tracker:run.tracker.clone(),entry:run.clone(),selection_context:hps_domain::SelectionQuery {inbound:Some(run.config.id.clone()),source_ip:Some(peer.ip()),..Default::default()},_admission:Arc::new(Admission { _global:global,_inbound:inbound }) };
                let acceptor=run.tls.clone(); let protocol=run.config.protocol; let child=session.clone();
                session.spawn_io(async move {
                    let _=tcp.set_nodelay(true);
                    let tcp=ActiveIo::new(tcp,child.activity.clone());
                    let io:BoxIo=if let Some(acceptor)=acceptor {
                        match tokio::time::timeout(child.handshake_timeout(),acceptor.accept(tcp)).await {
                            Ok(Ok(tls))=>Box::new(tls),_=>{child.failed();return;}
                        }
                    } else { Box::new(tcp) };
                    let io:BoxIo=Box::new(ActiveIo::new(io,child.activity.clone()));
                    let result=match protocol {
                        ProxyProtocol::Http|ProxyProtocol::Https=>crate::http_proxy::serve(io,child.clone()).await,
                        ProxyProtocol::Socks5=>crate::socks::serve(io,child.clone()).await,
                    };
                    if result.is_err() { child.failed(); }
                });
            }
            run.running.store(false,Ordering::Release);
            *run.address.lock().unwrap()=None;
            run.listener.lock().unwrap().take();
            stopped.cancel();
        });
        Ok(entry.status())
    }
    pub async fn stop(&self, id: &str) -> Result<InboundStatus, EngineError> {
        let entry = self.entries.get(id).ok_or(EngineError::NotFound)?;
        let mut control = entry.control.lock().await;
        if let Some(cancel) = control.take() {
            cancel.accept.cancel();
            cancel.sessions.cancel();
        }
        entry.listener.lock().unwrap().take();
        entry.tracker.close();
        entry.tracker.wait().await;
        Ok(entry.status())
    }
    pub async fn shutdown(&self) {
        self.dialer.retire_selection();
        self.cancel.cancel();
        for id in self.entries.keys() {
            let _ = self.stop(id).await;
        }
    }
    pub fn status(&self) -> ProxyStatus {
        ProxyStatus {
            inbounds: self.entries.values().map(|entry| entry.status()).collect(),
            active_connections: self.capacity.active(),
            eligible_upstreams: self.dialer.eligible_count(),
            limits: (*self.limits).clone(),
            shutting_down: self.cancel.is_cancelled(),
        }
    }
    pub fn ready(&self) -> bool {
        !self.cancel.is_cancelled()
            && self.entries.values().any(|entry| {
                entry.running.load(Ordering::Acquire)
                    && self
                        .dialer
                        .selection(&hps_domain::SelectionQuery {
                            inbound: Some(entry.config.id.clone()),
                            ..Default::default()
                        })
                        .eligible
                        > 0
                    && entry
                        .status()
                        .certificate
                        .is_none_or(|certificate| certificate.valid_now)
            })
    }
}
impl Entry {
    fn status(&self) -> InboundStatus {
        let mut certificate = self.certificate.clone();
        if let Some(info) = &mut certificate {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            info.valid_now = now >= info.not_before_unix && now <= info.not_after_unix;
        }
        InboundStatus {
            id: self.config.id.clone(),
            protocol: format!("{:?}", self.config.protocol).to_lowercase(),
            configured_listen: self.config.listen.to_string(),
            configured_enabled: self.config.enabled,
            listen: self
                .address
                .lock()
                .unwrap()
                .map(|address| address.to_string()),
            running: self.running.load(Ordering::Acquire),
            active_connections: self.capacity.active(),
            max_connections: self.config.max_connections,
            bytes_per_second: self.config.bytes_per_second,
            accepted_connections: self.accepted.load(Ordering::Relaxed),
            rejected_global_limit: self.global_rejected.load(Ordering::Relaxed),
            rejected_inbound_limit: self.inbound_rejected.load(Ordering::Relaxed),
            failed_connections: self.failed.load(Ordering::Relaxed),
            idle_timeouts: self.idle_timeouts.load(Ordering::Relaxed),
            certificate,
            last_error: self.last_error.lock().unwrap().clone(),
        }
    }
}
