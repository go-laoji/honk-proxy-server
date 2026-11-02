use super::*;
use crate::{
    ProxyError,
    capture::{CapturedUdpSession, QueuedDatagram, ReplyGate, UdpAdmission},
    dial::DatagramConnection,
    target::Target,
};
use hps_domain::{RoutingContext, SelectionTransport};
use tokio::{net::UdpSocket, sync::mpsc};

struct Closed(Arc<crate::capture::CaptureBudget>);
struct UdpRun {
    reply: UdpSocket,
    context: RoutingContext,
    target: Target,
    admission: UdpAdmission,
    _permit: Permit,
    gate: Arc<ReplyGate>,
    receiver: mpsc::Receiver<QueuedDatagram>,
}
impl Drop for Closed {
    fn drop(&mut self) {
        self.0.closed_sessions.fetch_add(1, Ordering::Relaxed);
    }
}
impl Engine {
    /// The reply descriptor must be bound to the actual original destination,
    /// connected to the actual client, and explicitly bypass marked by its owner.
    pub fn spawn_captured_udp(
        self: &Arc<Self>,
        reply: UdpSocket,
        mut context: RoutingContext,
        revision: i64,
        admission: UdpAdmission,
    ) -> Result<CapturedUdpSession, ProxyError> {
        let _registration = self.capture_registration.lock().unwrap();
        if self.cancel.is_cancelled()
            || self.capture_tracker.is_closed()
            || admission.admission.cancel.is_cancelled()
        {
            return Err(ProxyError::Disconnected);
        }
        if !self.capture_udp_enabled {
            return Err(ProxyError::UnsupportedRule);
        }
        let target = self.capture_target(
            reply.peer_addr()?,
            reply.local_addr()?,
            &mut context,
            revision,
            SelectionTransport::Udp,
        )?;
        let permit = self
            .capacity
            .acquire(self.limits.max_connections)
            .ok_or(ProxyError::LocalResource)?;
        let (session, receiver) = CapturedUdpSession::channel(&admission);
        let gate = session.gate.clone();
        let engine = self.clone();
        let complete = session.completion_guard();
        self.capture_tracker.spawn(async move {
            let _complete = complete;
            engine
                .run_captured_udp(UdpRun {
                    reply,
                    context,
                    target,
                    admission,
                    _permit: permit,
                    gate,
                    receiver,
                })
                .await;
        });
        Ok(session)
    }
    async fn run_captured_udp(self: Arc<Self>, run: UdpRun) {
        let UdpRun {
            reply,
            context,
            target,
            mut admission,
            _permit,
            gate,
            mut receiver,
        } = run;
        let _closed = Closed(admission.budget.clone());
        let cancel = admission.admission.cancel.clone();
        let connection = tokio::select! {biased;
            _=self.cancel.cancelled()=>{cancel.cancel();return;},
            _=cancel.cancelled()=>return,
            result=self.dialer.open_udp_routed(&target,&context,admission.budget.config.max_datagram_bytes as usize)=>result,
        };
        admission.admission.pending.take();
        let Ok(connection) = connection else {
            return;
        };
        connection.observation().bind_inbound_cancel(cancel.clone());
        let activity = Activity::new();
        let budget = Budget::new(self.bandwidth.clone(), Pacer::new(0), activity.clone());
        // Both directions remain polled while the other is paced. In particular,
        // a closed SOCKS control TCP aborts queued sends even under a slow budget.
        let sending = send_datagrams(&connection, &mut receiver, budget.clone(), activity.clone());
        let replying = reply_datagrams(
            &connection,
            &reply,
            &gate,
            &admission.budget,
            budget,
            activity.clone(),
        );
        tokio::select! {biased;
            _=self.cancel.cancelled()=>{cancel.cancel();},
            _=cancel.cancelled()=>{},
            _=connection.transport_ended()=>{},
            _=activity.idle(Duration::from_secs(u64::from(admission.budget.config.udp_idle_secs)))=>{cancel.cancel();},
            _=sending=>{},
            _=replying=>{},
        }
    }
}
async fn send_datagrams(
    connection: &DatagramConnection,
    receiver: &mut mpsc::Receiver<QueuedDatagram>,
    budget: Budget,
    activity: Activity,
) {
    while let Some(packet) = receiver.recv().await {
        activity.touch();
        budget.wait(packet.payload.len()).await;
        if connection.send(&packet.payload).await.is_err() {
            return;
        }
    }
}
async fn reply_datagrams(
    connection: &DatagramConnection,
    reply: &UdpSocket,
    gate: &ReplyGate,
    limits: &crate::capture::CaptureBudget,
    budget: Budget,
    activity: Activity,
) {
    let mut buffer = vec![0; limits.config.max_datagram_bytes as usize];
    let mut invalid = 0usize;
    loop {
        let length = match connection.recv(&mut buffer).await {
            Ok((length, _)) => length,
            Err(ProxyError::Protocol) => {
                limits.invalid_replies.fetch_add(1, Ordering::Relaxed);
                invalid += 1;
                if invalid >= 32 {
                    tokio::task::yield_now().await;
                    invalid = 0;
                }
                continue;
            }
            Err(_) => return,
        };
        activity.touch();
        budget.wait(length).await;
        loop {
            let result = {
                let _guard = gate.lock.lock().unwrap();
                if gate.cancel.is_cancelled() {
                    return;
                }
                reply.try_send(&buffer[..length])
            };
            match result {
                Ok(written) if written == length => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if reply.writable().await.is_err() {
                        return;
                    }
                }
                _ => {
                    connection.observation().fail(
                        "transferring",
                        "transparent_udp_reply_failed",
                        false,
                    );
                    return;
                }
            }
        }
    }
}
