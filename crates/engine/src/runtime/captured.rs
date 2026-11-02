//! Transparent TCP uses the same router, lease/health selection, node admission,
//! connection observations, global capacity and bandwidth budget as explicit
//! ingress. Only the capture owner may provide independently verified metadata.
use super::*;
use crate::capture::{CaptureAdmission, CapturedTcpSession};
use crate::{ProxyError, target::Target, traffic::relay};
use hps_domain::{FlowOrigin, RoutingContext, SelectionTransport};
use tokio::net::TcpStream;
struct ReopenCapture<'a>(&'a Engine);
impl Drop for ReopenCapture<'_> {
    fn drop(&mut self) {
        let _registration = self.0.capture_registration.lock().unwrap();
        if !self.0.cancel.is_cancelled() {
            self.0.capture_tracker.reopen();
        }
    }
}

impl Engine {
    /// Metadata must come from the actual captured tuple, never an RPC body.
    /// Refuse stale revisions or a mismatch with either real socket endpoint.
    /// Registration and shutdown share a lock so shutdown awaits every task.
    pub fn spawn_captured_tcp(
        self: &Arc<Self>,
        stream: TcpStream,
        context: RoutingContext,
        revision: i64,
    ) -> Result<(), ProxyError> {
        self.spawn_captured_tcp_scoped(
            stream,
            context,
            revision,
            CaptureAdmission::standalone(self.cancel.child_token()),
        )
        .map(|_| ())
    }
    pub fn spawn_captured_tcp_scoped(
        self: &Arc<Self>,
        stream: TcpStream,
        mut context: RoutingContext,
        revision: i64,
        admission: CaptureAdmission,
    ) -> Result<CapturedTcpSession, ProxyError> {
        let _registration = self.capture_registration.lock().unwrap();
        if self.cancel.is_cancelled()
            || self.capture_tracker.is_closed()
            || admission.cancel.is_cancelled()
        {
            return Err(ProxyError::Disconnected);
        }
        let target = self.capture_target(
            stream.peer_addr()?,
            stream.local_addr()?,
            &mut context,
            revision,
            SelectionTransport::Tcp,
        )?;
        let permit = self
            .capacity
            .acquire(self.limits.max_connections)
            .ok_or(ProxyError::LocalResource)?;
        let engine = self.clone();
        let session = CapturedTcpSession::new(admission.cancel.clone());
        let complete = session.completion_guard();
        self.capture_tracker.spawn(async move {
            let _complete = complete;
            engine
                .run_captured_tcp(stream, context, target, permit, admission)
                .await;
        });
        Ok(session)
    }
    pub(super) fn capture_target(
        &self,
        source: std::net::SocketAddr,
        destination: std::net::SocketAddr,
        context: &mut RoutingContext,
        revision: i64,
        transport: SelectionTransport,
    ) -> Result<Target, ProxyError> {
        if !self.capture_enabled
            || revision <= 0
            || revision != self.generation()
            || context.transport != transport
        {
            return Err(ProxyError::UnsupportedRule);
        }
        if context.source_ip != Some(source.ip())
            || context.source_port != Some(source.port())
            || context.destination_host != destination.ip().to_string()
            || context.destination_port != destination.port()
        {
            return Err(ProxyError::InvalidTarget);
        }
        match context.origin {
            FlowOrigin::LinuxLocal
                if context
                    .process
                    .as_ref()
                    .is_some_and(|p| p.pid > 0 && p.cgroup_id > 0) => {}
            FlowOrigin::Gateway => context.process = None,
            _ => return Err(ProxyError::UnsupportedRule),
        }
        context.inbound_protocol = Some("transparent".into());
        Target::new(&context.destination_host, context.destination_port)
    }
    async fn run_captured_tcp(
        self: Arc<Self>,
        stream: TcpStream,
        context: RoutingContext,
        target: Target,
        _permit: Permit,
        mut admission: CaptureAdmission,
    ) {
        let cancel = admission.cancel.clone();
        let connected = tokio::select! {biased;
            _=self.cancel.cancelled()=>{cancel.cancel();return;},
            _=cancel.cancelled()=>return,
            result=self.dialer.connect_routed(&target,false,&context)=>result,
        };
        admission.pending.take();
        let Ok(connected) = connected else {
            return;
        };
        if let Some(observation) = &connected.observation {
            observation.bind_inbound_cancel(cancel.clone());
        }
        let _ = stream.set_nodelay(true);
        let activity = Activity::new();
        let budget = Budget::new(self.bandwidth.clone(), Pacer::new(0), activity.clone());
        let left: BoxIo = Box::new(ActiveIo::new(stream, activity.clone()));
        let right: BoxIo = Box::new(ActiveIo::new(connected.io, activity.clone()));
        tokio::select! {biased;
            _=self.cancel.cancelled()=>{cancel.cancel();},
            _=cancel.cancelled()=>{},
            _=activity.idle(Duration::from_millis(self.limits.idle_timeout_ms))=>{},
            _=relay(left,right,self.limits.buffer_bytes,budget)=>{},
        }
    }
    /// Stop the owner's accept/receive loops before calling. New capture
    /// registrations are rejected during this drain; explicit ingress continues.
    /// Cancelling the wait restores admission unless the engine is shutting down.
    pub async fn wait_captured_idle(&self) {
        let _drain = self.capture_drain.lock().await;
        {
            let _registration = self.capture_registration.lock().unwrap();
            self.capture_tracker.close();
        }
        let _reopen = ReopenCapture(self);
        self.capture_tracker.wait().await;
    }
}
