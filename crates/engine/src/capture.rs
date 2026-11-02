//! Shared transparent TCP/UDP admission and bounded queued business datagrams.
use crate::ProxyError;
use hps_config::LinuxCaptureConfig;
use hps_domain::CaptureResourceUsage;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

pub struct CaptureBudget {
    pub(crate) config: LinuxCaptureConfig,
    pending: Arc<Semaphore>,
    sessions: Arc<Semaphore>,
    messages: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
    dropped: AtomicU64,
    pub(crate) invalid_replies: AtomicU64,
    pub(crate) closed_sessions: AtomicU64,
}
impl CaptureBudget {
    pub fn new(config: &LinuxCaptureConfig) -> Result<Arc<Self>, hps_config::ConfigError> {
        config.validate()?;
        Ok(Arc::new(Self {
            config: config.clone(),
            pending: Arc::new(Semaphore::new(config.max_pending as usize)),
            sessions: Arc::new(Semaphore::new(config.max_udp_sessions as usize)),
            messages: Arc::new(Semaphore::new(config.max_queued_datagrams as usize)),
            bytes: Arc::new(Semaphore::new(config.max_queued_bytes as usize)),
            dropped: AtomicU64::new(0),
            invalid_replies: AtomicU64::new(0),
            closed_sessions: AtomicU64::new(0),
        }))
    }
    pub fn tcp_admission(&self, scope: &CancellationToken) -> Result<CaptureAdmission, ProxyError> {
        if scope.is_cancelled() {
            return Err(ProxyError::Disconnected);
        }
        let pending = self
            .pending
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProxyError::LocalResource)?;
        Ok(CaptureAdmission {
            pending: Some(pending),
            cancel: scope.child_token(),
        })
    }
    pub fn udp_admission(
        self: &Arc<Self>,
        scope: &CancellationToken,
    ) -> Result<UdpAdmission, ProxyError> {
        let admission = self.tcp_admission(scope)?;
        let session = self
            .sessions
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProxyError::LocalResource)?;
        Ok(UdpAdmission {
            admission,
            _session: session,
            budget: self.clone(),
        })
    }
    pub fn usage(&self) -> CaptureResourceUsage {
        CaptureResourceUsage {
            pending: self.config.max_pending as usize - self.pending.available_permits(),
            max_pending: self.config.max_pending as usize,
            udp_sessions: self.config.max_udp_sessions as usize - self.sessions.available_permits(),
            max_udp_sessions: self.config.max_udp_sessions as usize,
            queued_datagrams: self.config.max_queued_datagrams as usize
                - self.messages.available_permits(),
            max_queued_datagrams: self.config.max_queued_datagrams as usize,
            queued_bytes: self.config.max_queued_bytes as usize - self.bytes.available_permits(),
            max_queued_bytes: self.config.max_queued_bytes as usize,
            dropped_datagrams: self.dropped.load(Ordering::Relaxed),
            invalid_replies: self.invalid_replies.load(Ordering::Relaxed),
            closed_udp_sessions: self.closed_sessions.load(Ordering::Relaxed),
        }
    }
    fn packet(&self, payload: &[u8]) -> Result<QueuedDatagram, ProxyError> {
        if payload.len() > self.config.max_datagram_bytes as usize {
            return Err(ProxyError::Protocol);
        }
        let messages = self
            .messages
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProxyError::LocalResource)?;
        let bytes = self
            .bytes
            .clone()
            .try_acquire_many_owned(payload.len() as u32)
            .map_err(|_| ProxyError::LocalResource)?;
        Ok(QueuedDatagram {
            payload: payload.to_vec(),
            _messages: messages,
            _bytes: bytes,
        })
    }
}
pub struct CaptureAdmission {
    pub(crate) pending: Option<OwnedSemaphorePermit>,
    pub(crate) cancel: CancellationToken,
}
impl CaptureAdmission {
    pub(crate) fn standalone(cancel: CancellationToken) -> Self {
        Self {
            pending: None,
            cancel,
        }
    }
}
pub struct UdpAdmission {
    pub(crate) admission: CaptureAdmission,
    _session: OwnedSemaphorePermit,
    pub(crate) budget: Arc<CaptureBudget>,
}
pub(crate) struct QueuedDatagram {
    pub(crate) payload: Vec<u8>,
    _messages: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}
pub(crate) struct ReplyGate {
    pub(crate) lock: Mutex<()>,
    pub(crate) cancel: CancellationToken,
}
pub struct CapturedUdpSession {
    sender: mpsc::Sender<QueuedDatagram>,
    budget: Arc<CaptureBudget>,
    pub(crate) gate: Arc<ReplyGate>,
}
impl CapturedUdpSession {
    pub(crate) fn channel(admission: &UdpAdmission) -> (Self, mpsc::Receiver<QueuedDatagram>) {
        let (sender, receiver) =
            mpsc::channel(admission.budget.config.max_queued_datagrams as usize);
        (
            Self {
                sender,
                budget: admission.budget.clone(),
                gate: Arc::new(ReplyGate {
                    lock: Mutex::new(()),
                    cancel: admission.admission.cancel.clone(),
                }),
            },
            receiver,
        )
    }
    pub fn try_send(&self, payload: &[u8]) -> Result<(), ProxyError> {
        let result = (|| {
            if self.is_closed() {
                return Err(ProxyError::Disconnected);
            }
            let packet = self.budget.packet(payload)?;
            let _gate = self.gate.lock.lock().unwrap();
            if self.gate.cancel.is_cancelled() {
                return Err(ProxyError::Disconnected);
            }
            self.sender.try_send(packet).map_err(|error| match error {
                mpsc::error::TrySendError::Closed(_) => ProxyError::Disconnected,
                mpsc::error::TrySendError::Full(_) => ProxyError::LocalResource,
            })
        })();
        if result.is_err() {
            self.budget.dropped.fetch_add(1, Ordering::Relaxed);
        }
        result
    }
    /// Serialize cancellation with the final nonblocking reply syscall. Once
    /// this returns, a replaced owner cannot send to a reused client tuple.
    pub fn cancel(&self) {
        let _gate = self.gate.lock.lock().unwrap();
        self.gate.cancel.cancel();
    }
    pub fn is_closed(&self) -> bool {
        self.gate.cancel.is_cancelled() || self.sender.is_closed()
    }
}
