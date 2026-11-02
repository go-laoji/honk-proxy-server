use hps_config::LinuxCaptureConfig;
use hps_ebpf_abi::{CapturedFlow, FlowKey};
use hps_engine::{
    capture::{CaptureBudget, CapturedTcpSession, CapturedUdpSession},
    runtime::Engine,
};
use hps_platform::transparent::{Capture, CaptureSockets, PreparedCapture};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    os::fd::AsFd,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub(super) struct Controller {
    running: Mutex<Option<Arc<Running>>>,
    operation: tokio::sync::Mutex<()>,
    diagnostics: Arc<tokio::sync::Semaphore>,
    prepared: PreparedCapture,
    engine: Mutex<Option<Arc<Engine>>>,
    requested: AtomicBool,
    attempts: AtomicU32,
    stop: CancellationToken,
    tasks: TaskTracker,
}
impl Controller {
    pub async fn prepare(prepared: PreparedCapture) -> Result<Arc<Self>, String> {
        let running = Running::prepare(prepared.clone()).await?;
        let controller = Arc::new(Self {
            running: Mutex::new(Some(running)),
            operation: tokio::sync::Mutex::new(()),
            diagnostics: Arc::new(tokio::sync::Semaphore::new(4)),
            prepared,
            engine: Mutex::new(None),
            requested: AtomicBool::new(true),
            attempts: AtomicU32::new(0),
            stop: CancellationToken::new(),
            tasks: TaskTracker::new(),
        });
        let weak = Arc::downgrade(&controller);
        let stop = controller.stop.clone();
        controller.tasks.spawn(async move {
            let mut next = tokio::time::Instant::now();
            loop {
                tokio::select! {biased; _=stop.cancelled()=>break, _=tokio::time::sleep(Duration::from_millis(100))=>{}}
                let Some(controller) = weak.upgrade() else { break };
                if !controller.requested.load(Ordering::Acquire)
                    || controller.attempts.load(Ordering::Acquire) >= 3
                { continue; }
                let failed = controller.running.lock().unwrap().as_ref()
                    .is_some_and(|r| r.stop.is_cancelled() && r.error.lock().unwrap().is_some());
                if !failed { next = tokio::time::Instant::now() + Duration::from_secs(2); continue; }
                if tokio::time::Instant::now() < next { continue; }
                let Ok(_operation) = controller.operation.try_lock() else { continue };
                if !controller.requested.load(Ordering::Acquire) { continue; }
                let attempt = controller.attempts.fetch_add(1, Ordering::AcqRel) + 1;
                let _ = controller.recover_locked().await;
                next = tokio::time::Instant::now() + Duration::from_secs(2u64.pow(attempt));
            }
        });
        Ok(controller)
    }
    pub async fn publish(&self, engine: Arc<Engine>) -> Result<(), String> {
        let _operation = self.operation.lock().await;
        if self.stop.is_cancelled() {
            return Err("capture controller is shut down".into());
        }
        if self.requested.load(Ordering::Acquire) {
            let running = self
                .running
                .lock()
                .unwrap()
                .clone()
                .ok_or("capture is stopped")?;
            running.collect().await?;
            running.publish(engine.clone()).await?;
        }
        *self.engine.lock().unwrap() = Some(engine);
        Ok(())
    }
    async fn recover_locked(&self) -> Result<(), String> {
        let engine = self
            .engine
            .lock()
            .unwrap()
            .clone()
            .ok_or("capture engine is unavailable")?;
        let previous = self.running.lock().unwrap().clone();
        let next = if let Some(previous) = &previous {
            match previous.recover().await {
                Ok(next) => next,
                Err(error) => {
                    *previous.error.lock().unwrap() = Some(error.clone());
                    return Err(error);
                }
            }
        } else {
            Running::prepare(self.prepared.clone()).await?
        };
        if let Err(error) = next.publish(engine).await {
            next.fail(error.clone()).await;
            *self.running.lock().unwrap() = Some(next);
            return Err(error);
        }
        *self.running.lock().unwrap() = Some(next);
        Ok(())
    }
    pub async fn control(&self, action: hps_domain::LinuxCaptureAction) -> Result<(), String> {
        let _operation = self.operation.lock().await;
        if self.stop.is_cancelled() {
            return Err("capture controller is shut down".into());
        }
        if action == hps_domain::LinuxCaptureAction::Stop {
            self.requested.store(false, Ordering::Release);
            let running = self.running.lock().unwrap().take();
            if let Some(running) = running {
                running.shutdown().await?;
            }
            return Ok(());
        }
        self.requested.store(true, Ordering::Release);
        self.attempts.store(0, Ordering::Release);
        let running = self.running.lock().unwrap().clone();
        if let Some(running) = &running
            && !running.stop.is_cancelled()
        {
            match running.collect().await {
                Ok(()) => return Ok(()),
                Err(error) => running.fail(error).await,
            }
        }
        self.recover_locked().await
    }
    pub async fn shutdown(&self) -> Result<(), String> {
        self.stop.cancel();
        self.tasks.close();
        self.tasks.wait().await;
        let _operation = self.operation.lock().await;
        self.requested.store(false, Ordering::Release);
        let running = self.running.lock().unwrap().take();
        if let Some(running) = running {
            running.shutdown().await?;
        }
        self.engine.lock().unwrap().take();
        Ok(())
    }
    pub async fn status(
        &self,
        mut view: hps_domain::LinuxCaptureStatus,
    ) -> hps_domain::LinuxCaptureStatus {
        view.retry_attempts = self.attempts.load(Ordering::Acquire);
        view.max_retry_attempts = 3;
        let permit = match self.diagnostics.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                view.state = "busy".into();
                view.reason = Some("capture diagnostics capacity reached".into());
                return view;
            }
        };
        let Ok(_operation) = self.operation.try_lock() else {
            view.state = "busy".into();
            view.reason = Some("capture operation in progress".into());
            return view;
        };
        let running = self.running.lock().unwrap().clone();
        let Some(running) = running else {
            view.state = "stopped".into();
            view.reason = Some("capture is stopped".into());
            return view;
        };
        let _publication = running.publication.lock().await;
        let capture = running.capture.clone();
        let facts = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            Ok::<_, String>((
                capture.published_policy()?,
                capture.links_present()?,
                capture.interfaces_ready()?,
                capture.routes_present()?,
                capture.program_ids()?,
                capture.map_ids()?,
                capture.stats()?,
                capture.flows()?.len(),
            ))
        })
        .await
        .map_err(|_| "capture inspection worker stopped".to_owned())
        .and_then(|r| r);
        view.generations = running
            .engines
            .lock()
            .unwrap()
            .entries
            .keys()
            .copied()
            .collect();
        view.resources = Some(running.budget.usage());
        view.active_owners = running
            .owners
            .lock()
            .unwrap()
            .values()
            .filter(|owner| !owner.session.finished())
            .count();
        view.rejected_ingress = running.dropped.load(Ordering::Relaxed);
        for listener in [&running.listeners.tcp4, &running.listeners.tcp6]
            .into_iter()
            .flatten()
        {
            match listener.local_addr() {
                Ok(address) => view.listeners.push(hps_domain::CaptureListenerStatus {
                    transport: "tcp".into(),
                    address: address.to_string(),
                }),
                Err(error) => {
                    view.state = "degraded".into();
                    view.reason = Some(format!("capture listener: {error}"));
                    return view;
                }
            }
        }
        for listener in [&running.listeners.udp4, &running.listeners.udp6]
            .into_iter()
            .flatten()
        {
            match listener.local_addr() {
                Ok(address) => view.listeners.push(hps_domain::CaptureListenerStatus {
                    transport: "udp".into(),
                    address: address.to_string(),
                }),
                Err(error) => {
                    view.state = "degraded".into();
                    view.reason = Some(format!("capture listener: {error}"));
                    return view;
                }
            }
        }
        match facts {
            Err(error) => {
                view.state = "degraded".into();
                view.reason = Some(error);
            }
            Ok((policy, links, interfaces, routes, programs, maps, stats, flows)) => {
                view.kernel_revision = policy.map(|(revision, _)| revision);
                view.suspended = policy.is_some_and(|(_, suspended)| suspended);
                view.links_present = Some(links);
                view.interfaces_ready = Some(interfaces);
                view.routes_present = Some(routes);
                view.program_ids = programs;
                view.map_ids = maps;
                view.flow_entries = flows;
                view.hooks = stats
                    .into_iter()
                    .zip(["cgroup", "egress", "ingress"])
                    .map(|(s, hook)| hps_domain::CaptureHookStatus {
                        hook: hook.into(),
                        observed_sockets: s.observed_sockets,
                        marked_sockets: s.marked_sockets,
                        captured_packets: s.captured_packets,
                        bypassed_packets: s.bypassed_packets,
                        rejected_packets: s.rejected_packets,
                        unsupported_packets: s.unsupported_packets,
                        missing_process: s.missing_process,
                        errors: s.errors,
                        last_error: s.last_error,
                        error_phase: s.error_phase,
                    })
                    .collect();
                view.ready = links
                    && interfaces
                    && routes
                    && !view.suspended
                    && policy.is_some_and(|(revision, _)| revision == view.userspace_revision)
                    && running.engines.lock().unwrap().active == view.userspace_revision
                    && running.started.load(Ordering::Acquire)
                    && !running.stop.is_cancelled()
                    && running.error.lock().unwrap().is_none();
                view.state = if view.ready { "running" } else { "degraded" }.into();
                view.reason = if view.ready {
                    None
                } else {
                    Some(running.error.lock().unwrap().clone().unwrap_or_else(|| {
                        "capture listeners, scope, routes or generations are not ready".into()
                    }))
                };
            }
        }
        view
    }
    pub fn pinned(&self) -> Vec<i64> {
        self.running
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| r.engines.lock().unwrap().entries.keys().copied().collect())
            .unwrap_or_default()
    }
}
#[derive(Clone, Default)]
struct Listeners {
    tcp4: Option<Arc<TcpListener>>,
    tcp6: Option<Arc<TcpListener>>,
    udp4: Option<Arc<UdpSocket>>,
    udp6: Option<Arc<UdpSocket>>,
}
impl Listeners {
    fn flush(&self) -> Result<(), String> {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        for listener in [&self.tcp4, &self.tcp6].into_iter().flatten() {
            let mut empty = false;
            for _ in 0..65536 {
                let fd = unsafe {
                    libc::accept4(
                        listener.as_raw_fd(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                    )
                };
                if fd >= 0 {
                    drop(unsafe { OwnedFd::from_raw_fd(fd) });
                    continue;
                }
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    empty = true;
                    break;
                }
                if matches!(error.raw_os_error(), Some(libc::EINTR | libc::ECONNABORTED)) {
                    continue;
                }
                return Err(format!("capture backlog drain: {error}"));
            }
            if !empty {
                return Err("capture TCP backlog drain bound reached".into());
            }
        }
        let mut bytes = [0u8; 65536];
        for socket in [&self.udp4, &self.udp6].into_iter().flatten() {
            let mut empty = false;
            for _ in 0..65536 {
                // Read the OS queue directly; Tokio readiness can be stale once
                // the previous receiver task has been drained.
                let n = unsafe {
                    libc::recv(
                        socket.as_raw_fd(),
                        bytes.as_mut_ptr().cast(),
                        bytes.len(),
                        libc::MSG_DONTWAIT,
                    )
                };
                if n >= 0 {
                    continue;
                }
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    empty = true;
                    break;
                }
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(format!("capture datagram drain: {error}"));
            }
            if !empty {
                return Err("capture UDP backlog drain bound reached".into());
            }
        }
        Ok(())
    }
    fn bind(config: &LinuxCaptureConfig) -> Result<Self, String> {
        let mut result = Self::default();
        if config.tcp {
            if config.ipv4 {
                result.tcp4 = Some(Arc::new(
                    hps_net::transparent::bind_tcp(
                        ([0, 0, 0, 0], config.tcp_port).into(),
                        config.bypass_mark,
                        config.max_pending.min(4096),
                    )
                    .map_err(|e| format!("capture TCP4 bind: {e}"))?,
                ));
            }
            if config.ipv6 {
                result.tcp6 = Some(Arc::new(
                    hps_net::transparent::bind_tcp(
                        std::net::SocketAddr::new(
                            std::net::Ipv6Addr::UNSPECIFIED.into(),
                            config.tcp_port,
                        ),
                        config.bypass_mark,
                        config.max_pending.min(4096),
                    )
                    .map_err(|e| format!("capture TCP6 bind: {e}"))?,
                ));
            }
        }
        if config.udp {
            if config.ipv4 {
                result.udp4 = Some(Arc::new(
                    hps_net::transparent::bind_udp(
                        ([0, 0, 0, 0], config.udp_port).into(),
                        config.bypass_mark,
                    )
                    .map_err(|e| format!("capture UDP4 bind: {e}"))?,
                ));
            }
            if config.ipv6 {
                result.udp6 = Some(Arc::new(
                    hps_net::transparent::bind_udp(
                        std::net::SocketAddr::new(
                            std::net::Ipv6Addr::UNSPECIFIED.into(),
                            config.udp_port,
                        ),
                        config.bypass_mark,
                    )
                    .map_err(|e| format!("capture UDP6 bind: {e}"))?,
                ));
            }
        }
        Ok(result)
    }
}
#[derive(Default)]
struct Generations {
    active: i64,
    entries: BTreeMap<i64, Arc<Engine>>,
}
enum Session {
    Tcp(CapturedTcpSession),
    Udp(CapturedUdpSession),
}
impl Session {
    fn cancel(&self) {
        match self {
            Self::Tcp(s) => s.cancel(),
            Self::Udp(s) => s.cancel(),
        }
    }
    fn finished(&self) -> bool {
        match self {
            Self::Tcp(s) => s.is_finished(),
            Self::Udp(s) => s.is_finished(),
        }
    }
    async fn wait(&self) {
        match self {
            Self::Tcp(s) => s.finished().await,
            Self::Udp(s) => s.finished().await,
        }
    }
}
struct Owner {
    flow: CapturedFlow,
    session: Session,
    completed: Option<Instant>,
}
struct Running {
    capture: Arc<Capture>,
    listeners: Listeners,
    budget: Arc<CaptureBudget>,
    engines: Mutex<Generations>,
    owners: Mutex<HashMap<FlowKey, Owner>>,
    publication: tokio::sync::Mutex<()>,
    stop: CancellationToken,
    tasks: TaskTracker,
    started: AtomicBool,
    error: Mutex<Option<String>>,
    dropped: AtomicU64,
}
fn monotonic_ns() -> Result<u64, String> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return Err("monotonic clock unavailable".into());
    }
    Ok((time.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(time.tv_nsec as u64))
}
impl Running {
    async fn prepare(prepared: PreparedCapture) -> Result<Arc<Self>, String> {
        let listeners = Listeners::bind(&prepared.specification)?;
        let budget = CaptureBudget::new(&prepared.specification).map_err(|e| e.to_string())?;
        let descriptors = listeners.clone();
        let capture = tokio::task::spawn_blocking(move || {
            let capture = prepared.load(CaptureSockets {
                tcp4: descriptors.tcp4.as_ref().map(|s| s.as_fd()),
                tcp6: descriptors.tcp6.as_ref().map(|s| s.as_fd()),
                udp4: descriptors.udp4.as_ref().map(|s| s.as_fd()),
                udp6: descriptors.udp6.as_ref().map(|s| s.as_fd()),
            })?;
            capture.prepare_routes()?;
            capture.settled_revision()?;
            Ok::<_, String>(capture)
        })
        .await
        .map_err(|_| "capture preparation worker stopped")??;
        Ok(Arc::new(Self {
            capture: Arc::new(capture),
            listeners,
            budget,
            engines: Mutex::new(Generations::default()),
            owners: Mutex::new(HashMap::new()),
            publication: tokio::sync::Mutex::new(()),
            stop: CancellationToken::new(),
            tasks: TaskTracker::new(),
            started: AtomicBool::new(false),
            error: Mutex::new(None),
            dropped: AtomicU64::new(0),
        }))
    }
    async fn publish(self: &Arc<Self>, engine: Arc<Engine>) -> Result<(), String> {
        let _publication = self.publication.lock().await;
        if self.stop.is_cancelled() {
            return Err("capture is stopped".into());
        }
        if let Some(error) = self.error.lock().unwrap().clone() {
            return Err(error);
        }
        let revision = engine.generation();
        if revision <= 0 {
            return Err("capture engine generation must be positive".into());
        }
        let inserted = {
            let mut engines = self.engines.lock().unwrap();
            if let Some(old) = engines.entries.get(&revision) {
                if old.preparation_fingerprint() != engine.preparation_fingerprint() {
                    return Err("capture revision already has a different prepared snapshot; restart is required".into());
                }
                false
            } else {
                if engines.entries.len() >= 32 {
                    return Err(
                        "capture generation budget reached; wait for flows to finish".into(),
                    );
                }
                engines.entries.insert(revision, engine);
                true
            }
        };
        self.start_loops();
        let capture = self.capture.clone();
        let result = tokio::task::spawn_blocking(move || capture.activate(revision, true))
            .await
            .map_err(|_| "capture publication worker stopped".to_owned())
            .and_then(|r| r);
        match result {
            Ok(()) => {
                self.engines.lock().unwrap().active = revision;
                Ok(())
            }
            Err(error) => {
                if inserted {
                    self.engines.lock().unwrap().entries.remove(&revision);
                }
                Err(error)
            }
        }
    }
    fn start_loops(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        for listener in [&self.listeners.tcp4, &self.listeners.tcp6]
            .into_iter()
            .flatten()
        {
            let listener = listener.clone();
            let weak = Arc::downgrade(self);
            let stop = self.stop.clone();
            self.tasks.spawn(async move{loop{
                let incoming=tokio::select!{biased;_=stop.cancelled()=>break,result=listener.accept()=>result};
                let Some(running)=weak.upgrade()else{break};
                match incoming {Ok((stream,_))=>{if running.tcp(stream).is_err(){running.dropped.fetch_add(1,Ordering::Relaxed);}},Err(e)=>{running.fail(format!("capture TCP accept: {e}")).await;break;}}
            }});
        }
        for socket in [&self.listeners.udp4, &self.listeners.udp6]
            .into_iter()
            .flatten()
        {
            let socket = socket.clone();
            let weak = Arc::downgrade(self);
            let stop = self.stop.clone();
            self.tasks.spawn(async move{let mut bytes=vec![0u8;65536];loop{
                let incoming=tokio::select!{biased;_=stop.cancelled()=>break,result=hps_net::transparent::recv_original(&socket,&mut bytes)=>result};
                let Some(running)=weak.upgrade()else{break};
                match incoming {Ok(packet)=>{if running.udp(packet,&bytes[..packet.bytes]).is_err(){running.dropped.fetch_add(1,Ordering::Relaxed);}},Err(e)if e.kind()==std::io::ErrorKind::InvalidData=>{running.dropped.fetch_add(1,Ordering::Relaxed);},Err(e)=>{running.fail(format!("capture UDP receive: {e}")).await;break;}}
            }});
        }
        let weak = Arc::downgrade(self);
        let stop = self.stop.clone();
        self.tasks.spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_millis(250));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {biased;_=stop.cancelled()=>break,_=timer.tick()=>{}}
                let Some(running) = weak.upgrade() else { break };
                if let Err(error) = running.collect().await {
                    running.fail(error).await;
                    break;
                }
            }
        });
    }
    fn engine(&self, revision: u64) -> Result<Arc<Engine>, String> {
        self.engines
            .lock()
            .unwrap()
            .entries
            .get(&(revision as i64))
            .cloned()
            .ok_or_else(|| {
                "captured generation is unavailable; refusing current-generation fallback".into()
            })
    }
    fn tcp(&self, stream: TcpStream) -> Result<(), String> {
        if self.stop.is_cancelled() {
            return Err("capture stopped".into());
        }
        let metadata = self.capture.routing_metadata(
            stream.peer_addr().map_err(|e| e.to_string())?,
            stream.local_addr().map_err(|e| e.to_string())?,
            6,
        )?;
        let engine = self.engine(metadata.flow.revision)?;
        let mut owners = self.owners.lock().unwrap();
        if owners.contains_key(&metadata.key)
            || owners.len() >= self.capture.specification.max_flows as usize
        {
            return Err("capture tuple owner capacity or identity conflict".into());
        }
        let admission = self
            .budget
            .tcp_admission(&self.stop)
            .map_err(|e| e.to_string())?;
        let session = engine
            .spawn_captured_tcp_scoped(
                stream,
                metadata.context,
                metadata.flow.revision as i64,
                admission,
            )
            .map_err(|e| e.to_string())?;
        owners.insert(
            metadata.key,
            Owner {
                flow: metadata.flow,
                session: Session::Tcp(session),
                completed: None,
            },
        );
        Ok(())
    }
    fn udp(
        &self,
        packet: hps_net::transparent::OriginalDatagram,
        bytes: &[u8],
    ) -> Result<(), String> {
        if self.stop.is_cancelled() {
            return Err("capture stopped".into());
        }
        let metadata = self
            .capture
            .routing_metadata(packet.source, packet.destination, 17)?;
        let engine = self.engine(metadata.flow.revision)?;
        let mut owners = self.owners.lock().unwrap();
        if let Some(owner) = owners.get(&metadata.key) {
            if !owner.flow.same_identity(&metadata.flow) {
                owner.session.cancel();
                return Err("capture UDP identity changed before owner retirement".into());
            }
            return match &owner.session {
                Session::Udp(session) => session.try_send(bytes).map_err(|e| e.to_string()),
                _ => Err("capture transport owner mismatch".into()),
            };
        }
        if owners.len() >= self.capture.specification.max_flows as usize {
            return Err("capture tuple owner capacity reached".into());
        }
        let reply = hps_net::transparent::bind_udp_reply(
            packet.destination,
            packet.source,
            self.capture.specification.bypass_mark,
        )
        .map_err(|e| e.to_string())?;
        let admission = self
            .budget
            .udp_admission(&self.stop)
            .map_err(|e| e.to_string())?;
        let session = engine
            .spawn_captured_udp(
                reply,
                metadata.context,
                metadata.flow.revision as i64,
                admission,
            )
            .map_err(|e| e.to_string())?;
        let result = session.try_send(bytes).map_err(|e| e.to_string());
        owners.insert(
            metadata.key,
            Owner {
                flow: metadata.flow,
                session: Session::Udp(session),
                completed: None,
            },
        );
        result
    }
    async fn collect(&self) -> Result<(), String> {
        let _publication = self.publication.lock().await;
        let active = self.engines.lock().unwrap().active;
        if active == 0 || self.stop.is_cancelled() {
            return Ok(());
        }
        let capture = self.capture.clone();
        let (settled, flows) = tokio::task::spawn_blocking(move || {
            if !capture.links_present()? {
                return Err("capture scope or an attached link was lost".into());
            }
            if !capture.interfaces_ready()? {
                return Err("capture interface is down or lost operational carrier".into());
            }
            if !capture.routes_present()? {
                return Err("capture owned routing objects were lost".into());
            }
            if capture.published_policy()? != Some((active, false)) {
                return Err("capture kernel policy is suspended or changed".into());
            }
            Ok::<_, String>((capture.settled_revision()?, capture.flows()?))
        })
        .await
        .map_err(|_| "capture GC worker stopped")??;
        if settled != active {
            return Err("capture kernel and user-space generations diverged".into());
        }
        let now = monotonic_ns()?;
        let mut owners = self.owners.lock().unwrap();
        for (key, flow) in flows {
            let retire = if let Some(owner) = owners.get_mut(&key) {
                if !owner.flow.same_identity(&flow) {
                    owner.session.cancel();
                    false
                } else if owner.session.finished() {
                    let completed = owner.completed.get_or_insert_with(Instant::now);
                    key.protocol == 17 || completed.elapsed() >= Duration::from_secs(2)
                } else {
                    false
                }
            } else {
                now.saturating_sub(flow.first_seen_ns) >= 30_000_000_000
            };
            if retire && self.capture.retire_flow(key, flow)? {
                owners.remove(&key);
            }
        }
        // Only this GC removes flow keys. Stable old keys cannot be skipped by
        // a concurrent user-space removal during this bounded post-grace scan.
        let retained = self
            .capture
            .flows()?
            .into_iter()
            .map(|(_, f)| f.revision as i64)
            .collect::<HashSet<_>>();
        self.engines
            .lock()
            .unwrap()
            .entries
            .retain(|revision, engine| {
                *revision == active || retained.contains(revision) || !engine.is_idle()
            });
        Ok(())
    }
    async fn fail(&self, mut error: String) {
        let _publication = self.publication.lock().await;
        if self.stop.is_cancelled() {
            return;
        }
        let capture = self.capture.clone();
        let suspended = tokio::task::spawn_blocking(move || capture.suspend())
            .await
            .map_err(|_| "capture fault worker stopped".to_owned())
            .and_then(|r| r);
        if let Err(cause) = suspended {
            error.push_str("; ");
            error.push_str(&cause);
        }
        *self.error.lock().unwrap() = Some(error);
        self.stop.cancel();
    }
    async fn drain(&self) {
        self.stop.cancel();
        self.tasks.close();
        self.tasks.wait().await;
        let owners = std::mem::take(&mut *self.owners.lock().unwrap());
        for owner in owners.values() {
            owner.session.cancel();
        }
        for owner in owners.values() {
            owner.session.wait().await;
        }
    }
    async fn recover(&self) -> Result<Arc<Self>, String> {
        self.drain().await;
        let _publication = self.publication.lock().await;
        let capture = self.capture.clone();
        tokio::task::spawn_blocking(move || {
            capture.reconcile()?;
            capture.settled_revision()?;
            Ok::<_, String>(())
        })
        .await
        .map_err(|_| "capture repair worker stopped")??;
        // Drain all old listener backlog before retiring identities. New selected
        // traffic remains refused by the suspended policy throughout this step.
        self.listeners.flush()?;
        for (key, flow) in self.capture.flows()? {
            self.capture.retire_flow(key, flow)?;
        }
        Ok(Arc::new(Self {
            capture: self.capture.clone(),
            listeners: self.listeners.clone(),
            budget: self.budget.clone(),
            engines: Mutex::new(Generations::default()),
            owners: Mutex::new(HashMap::new()),
            publication: tokio::sync::Mutex::new(()),
            stop: CancellationToken::new(),
            tasks: TaskTracker::new(),
            started: AtomicBool::new(false),
            error: Mutex::new(None),
            dropped: AtomicU64::new(self.dropped.load(Ordering::Relaxed)),
        }))
    }
    async fn shutdown(&self) -> Result<(), String> {
        // Stop new capture before draining, retaining fail-closed scope until
        // the user's explicit stop finally detaches it.
        let capture = self.capture.clone();
        let _ = tokio::task::spawn_blocking(move || capture.suspend()).await;
        self.drain().await;
        let _publication = self.publication.lock().await;
        let capture = self.capture.clone();
        tokio::task::spawn_blocking(move || capture.shutdown())
            .await
            .map_err(|_| "capture shutdown worker stopped")?
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.cancel();
        for owner in self.owners.get_mut().unwrap().values() {
            owner.session.cancel();
        }
    }
}
