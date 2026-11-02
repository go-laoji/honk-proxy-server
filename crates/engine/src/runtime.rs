use crate::{
    BoxIo,
    dial::Dialer,
    traffic::{ActiveIo, Activity, Budget, Pacer},
};
use hps_config::{InboundConfig, ProxyConfig, ProxyProtocol};
use hps_domain::{CertificateStatus, InboundStatus, ProxyStatus, ResourceLimits};
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
use tokio::{
    net::TcpListener,
    sync::{OwnedSemaphorePermit, Semaphore},
};
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
    dialer: Arc<Dialer>,
    limits: Arc<ResourceLimits>,
    capacity: Arc<Semaphore>,
    bandwidth: Arc<Pacer>,
    cancel: CancellationToken,
}
struct Entry {
    config: InboundConfig,
    tls: Option<TlsAcceptor>,
    certificate: Option<CertificateStatus>,
    control: tokio::sync::Mutex<Option<CancellationToken>>,
    tracker: TaskTracker,
    running: AtomicBool,
    address: Mutex<Option<SocketAddr>>,
    last_error: Mutex<Option<String>>,
    capacity: Arc<Semaphore>,
    bandwidth: Arc<Pacer>,
    accepted: AtomicU64,
    global_rejected: AtomicU64,
    inbound_rejected: AtomicU64,
    failed: AtomicU64,
    idle_timeouts: AtomicU64,
}
struct Admission {
    _global: OwnedSemaphorePermit,
    _inbound: OwnedSemaphorePermit,
}
#[derive(Clone)]
pub(crate) struct Session {
    pub dialer: Arc<Dialer>,
    pub limits: Arc<ResourceLimits>,
    pub budget: Budget,
    pub activity: Activity,
    pub cancel: CancellationToken,
    pub tracker: TaskTracker,
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
        config.validate().map_err(|_| EngineError::Configuration)?;
        let dialer = Arc::new(Dialer::new(&config).map_err(|_| EngineError::Tls)?);
        let mut entries = BTreeMap::new();
        for config in config.inbounds {
            let (tls, certificate) = match &config.tls {
                Some(tls) => {
                    let (tls, info) = crate::tls::server(tls).map_err(|_| EngineError::Tls)?;
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
                capacity: Arc::new(Semaphore::new(config.max_connections)),
                bandwidth: Pacer::new(config.bytes_per_second),
                config,
                tls,
                certificate,
                control: tokio::sync::Mutex::new(None),
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
            entries,
            dialer,
            capacity: Arc::new(Semaphore::new(config.limits.max_connections)),
            bandwidth: Pacer::new(config.limits.bytes_per_second),
            limits: Arc::new(config.limits),
            cancel: CancellationToken::new(),
        }))
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
            previous.cancel();
            entry.tracker.close();
            entry.tracker.wait().await;
        }
        let listener = match TcpListener::bind(entry.config.listen).await {
            Ok(listener) => listener,
            Err(_) => {
                *entry.last_error.lock().unwrap() = Some("listen_failed".into());
                return Err(EngineError::Bind);
            }
        };
        let address = listener.local_addr().map_err(|_| EngineError::Bind)?;
        let cancel = self.cancel.child_token();
        *control = Some(cancel.clone());
        *entry.address.lock().unwrap() = Some(address);
        *entry.last_error.lock().unwrap() = None;
        entry.running.store(true, Ordering::Release);
        entry.tracker.reopen();
        let engine = self.clone();
        let run = entry.clone();
        entry.tracker.spawn(async move {
            loop {
                let accepted=tokio::select! { biased; _=cancel.cancelled()=>break, accepted=listener.accept()=>accepted };
                let (tcp,_)=match accepted {
                    Ok(accepted)=>accepted,
                    Err(_)=>{*run.last_error.lock().unwrap()=Some("accept_failed".into());cancel.cancel();break;}
                };
                let global=match engine.capacity.clone().try_acquire_owned() { Ok(permit)=>permit,Err(_)=>{run.global_rejected.fetch_add(1,Ordering::Relaxed);continue;} };
                let inbound=match run.capacity.clone().try_acquire_owned() { Ok(permit)=>permit,Err(_)=>{run.inbound_rejected.fetch_add(1,Ordering::Relaxed);continue;} };
                run.accepted.fetch_add(1,Ordering::Relaxed);
                let activity=Activity::new();
                let session=Session { dialer:engine.dialer.clone(),limits:engine.limits.clone(),budget:Budget::new(engine.bandwidth.clone(),run.bandwidth.clone(),activity.clone()),activity,cancel:cancel.child_token(),tracker:run.tracker.clone(),entry:run.clone(),_admission:Arc::new(Admission { _global:global,_inbound:inbound }) };
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
        });
        Ok(entry.status())
    }
    pub async fn stop(&self, id: &str) -> Result<InboundStatus, EngineError> {
        let entry = self.entries.get(id).ok_or(EngineError::NotFound)?;
        let mut control = entry.control.lock().await;
        if let Some(cancel) = control.take() {
            cancel.cancel();
        }
        entry.tracker.close();
        entry.tracker.wait().await;
        Ok(entry.status())
    }
    pub async fn shutdown(&self) {
        self.cancel.cancel();
        for id in self.entries.keys() {
            let _ = self.stop(id).await;
        }
    }
    pub fn status(&self) -> ProxyStatus {
        ProxyStatus {
            inbounds: self.entries.values().map(|entry| entry.status()).collect(),
            active_connections: self.limits.max_connections - self.capacity.available_permits(),
            eligible_upstreams: self.dialer.eligible_count(),
            limits: (*self.limits).clone(),
            shutting_down: self.cancel.is_cancelled(),
        }
    }
    pub fn ready(&self) -> bool {
        !self.cancel.is_cancelled()
            && self.dialer.eligible_count() > 0
            && self.entries.values().any(|entry| {
                entry.running.load(Ordering::Acquire)
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
            active_connections: self.config.max_connections - self.capacity.available_permits(),
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
