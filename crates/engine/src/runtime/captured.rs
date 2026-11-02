//! Transparent TCP uses the same router, lease/health selection, node admission,
//! connection observations, global capacity and bandwidth budget as explicit
//! ingress. Only the capture owner may provide independently verified metadata.
use super::*;
use crate::{ProxyError, target::Target, traffic::relay};
use hps_domain::{FlowOrigin, RoutingContext, SelectionTransport};
use tokio::net::TcpStream;

impl Engine {
    /// Metadata must come from the actual captured tuple, never an RPC body.
    /// Refuse stale revisions or a mismatch with either real socket endpoint.
    /// Registration and shutdown share a lock so shutdown awaits every task.
    pub fn spawn_captured_tcp(
        self: &Arc<Self>,
        stream: TcpStream,
        mut context: RoutingContext,
        revision: i64,
    ) -> Result<(), ProxyError> {
        let _registration = self.capture_registration.lock().unwrap();
        if self.cancel.is_cancelled() {
            return Err(ProxyError::Disconnected);
        }
        if !self.capture_enabled
            || revision <= 0
            || revision != self.generation()
            || context.transport != SelectionTransport::Tcp
        {
            return Err(ProxyError::UnsupportedRule);
        }
        let source = stream.peer_addr()?;
        let destination = stream.local_addr()?;
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
        let target = Target::new(&context.destination_host, context.destination_port)?;
        let permit = self
            .capacity
            .acquire(self.limits.max_connections)
            .ok_or(ProxyError::LocalResource)?;
        let engine = self.clone();
        self.capture_tracker.spawn(async move {
            engine
                .run_captured_tcp(stream, context, target, permit)
                .await;
        });
        Ok(())
    }
    async fn run_captured_tcp(
        self: Arc<Self>,
        stream: TcpStream,
        context: RoutingContext,
        target: Target,
        _permit: Permit,
    ) {
        let cancel = self.cancel.child_token();
        let connected = tokio::select! {biased;
            _=cancel.cancelled()=>return,
            result=self.dialer.connect_routed(&target,false,&context)=>result,
        };
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
            _=cancel.cancelled()=>{},
            _=activity.idle(Duration::from_millis(self.limits.idle_timeout_ms))=>{},
            _=relay(left,right,self.limits.buffer_bytes,budget)=>{},
        }
    }
}
