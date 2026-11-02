use super::*;
use crate::{connections::ConnectionGuard, demand::DemandGuard, udp::SocksUdpAssociation};
use hps_domain::{RouteAction, RoutingContext, SelectionTransport};
use std::net::{IpAddr, SocketAddr};
use tokio::net::UdpSocket;
#[cfg(test)]
mod tests;

enum DatagramIo {
    Direct(UdpSocket),
    Socks(SocksUdpAssociation),
}
pub struct DatagramConnection {
    io: DatagramIo,
    target: Target,
    observation: ConnectionGuard,
    _demand: Option<DemandGuard>,
    max_payload: usize,
}
impl DatagramConnection {
    pub(crate) async fn transport_ended(&self) -> ProxyError {
        let error = match &self.io {
            DatagramIo::Direct(_) => std::future::pending::<ProxyError>().await,
            DatagramIo::Socks(socket) => socket.control_ended().await,
        };
        if !self.observation.cancellation().is_cancelled() {
            self.observation
                .fail("transferring", "udp_association_closed", false);
        }
        error
    }
    pub fn observation(&self) -> &ConnectionGuard {
        &self.observation
    }
    /// One datagram, one send. Counters include only accepted business bytes;
    /// SOCKS headers, association handshakes and malformed replies are excluded.
    pub async fn send(&self, payload: &[u8]) -> Result<usize, ProxyError> {
        if payload.len() > self.max_payload {
            return Err(ProxyError::Protocol);
        }
        let cancel = self.observation.cancellation();
        let outcome = tokio::select! {biased;
            _=cancel.cancelled()=>return Err(ProxyError::Disconnected),
            result=async {match &self.io {
                DatagramIo::Direct(socket)=>socket.send(payload).await.map_err(ProxyError::Io),
                DatagramIo::Socks(socket)=>socket.send(&self.target,payload).await,
            }}=>result,
        };
        match outcome {
            Ok(length) if length == payload.len() => {
                self.observation.datagram_bytes(true, length);
                Ok(length)
            }
            Ok(_) => Err(ProxyError::Protocol),
            Err(error) => {
                if matches!(error, ProxyError::Disconnected) && !cancel.is_cancelled() {
                    self.observation
                        .fail("transferring", "udp_association_closed", false);
                } else if !matches!(error, ProxyError::Protocol | ProxyError::Disconnected) {
                    self.observation
                        .fail("transferring", "udp_send_failed", false);
                }
                Err(error)
            }
        }
    }
    pub async fn recv(&self, buffer: &mut [u8]) -> Result<(usize, Target), ProxyError> {
        let cancel = self.observation.cancellation();
        let outcome = tokio::select! {biased;
            _=cancel.cancelled()=>return Err(ProxyError::Disconnected),
            result=async {match &self.io {
                DatagramIo::Direct(socket)=>{
                    let mut packet=vec![0;65536];let length=socket.recv(&mut packet).await?;
                    if length>buffer.len() || length>self.max_payload {return Err(ProxyError::Protocol);}
                    let peer=socket.peer_addr()?;let target=Target::new(&peer.ip().to_string(),peer.port())?;
                    buffer[..length].copy_from_slice(&packet[..length]);Ok((length,target))
                },
                DatagramIo::Socks(socket)=>socket.recv(buffer).await,
            }}=>result,
        };
        match outcome {
            Ok((length, source)) => {
                if source.port != self.target.port
                    || self
                        .target
                        .host
                        .parse::<IpAddr>()
                        .is_ok_and(|ip| source.host.parse::<IpAddr>().ok() != Some(ip))
                {
                    return Err(ProxyError::Protocol);
                }
                self.observation.datagram_bytes(false, length);
                Ok((length, source))
            }
            Err(error) => {
                if matches!(error, ProxyError::Disconnected) && !cancel.is_cancelled() {
                    self.observation
                        .fail("transferring", "udp_association_closed", false);
                } else if !matches!(error, ProxyError::Protocol | ProxyError::Disconnected) {
                    self.observation
                        .fail("transferring", "udp_recv_failed", false);
                }
                Err(error)
            }
        }
    }
}
impl Dialer {
    pub async fn open_udp_routed(
        &self,
        target: &Target,
        context: &RoutingContext,
        max_payload: usize,
    ) -> Result<DatagramConnection, ProxyError> {
        if !self.capabilities.udp || context.transport != SelectionTransport::Udp {
            return Err(ProxyError::UnsupportedRule);
        }
        if context.destination_host != target.host
            || context.destination_port != target.port
            || max_payload == 0
            || max_payload > 65507
        {
            return Err(ProxyError::InvalidTarget);
        }
        if Target::new(&target.host, target.port)? != *target {
            return Err(ProxyError::InvalidTarget);
        }
        let decision = self
            .router
            .evaluate(context, self.revision.load(Ordering::Acquire), false);
        let observation = self
            .nodes
            .observe(context.clone(), decision.clone())
            .ok_or(ProxyError::LocalResource)?;
        let cancel = observation.cancellation();
        let outcome = tokio::select! {biased;
            _=cancel.cancelled()=>Err(ProxyError::Disconnected),
            result=self.open_udp_decided(target,context,&decision,&observation,max_payload)=>result,
        };
        match outcome {
            Ok((io, demand)) => {
                observation.open();
                Ok(DatagramConnection {
                    io,
                    target: target.clone(),
                    observation,
                    _demand: demand,
                    max_payload,
                })
            }
            Err(error) => {
                self.record_connection_error(&observation, &error);
                Err(error)
            }
        }
    }
    async fn open_udp_decided(
        &self,
        target: &Target,
        context: &RoutingContext,
        decision: &hps_domain::RouteDecision,
        observation: &ConnectionGuard,
        max_payload: usize,
    ) -> Result<(DatagramIo, Option<DemandGuard>), ProxyError> {
        if decision.action == RouteAction::Reject {
            return Err(ProxyError::Rejected);
        }
        if decision.action == RouteAction::Direct {
            let socket = tokio::time::timeout(self.timeout, self.direct_datagram(target))
                .await
                .map_err(|_| ProxyError::Timeout)??;
            return Ok((DatagramIo::Direct(socket), None));
        }
        let (upstream, demand) = self.admit_route(target, context, decision, observation)?;
        if upstream.config.protocol != ProxyProtocol::Socks5 {
            return Err(ProxyError::NoUpstream);
        }
        let progress = Progress::new();
        let deadline = tokio::time::Instant::now() + self.timeout;
        let outcome = tokio::time::timeout_at(deadline, async {
            let control = self.connect_endpoint(&upstream, &progress).await?;
            progress.stage(ProbePhase::Authentication);
            let resolver = self.resolver.read().unwrap().clone();
            SocksUdpAssociation::associate_with_progress(
                control,
                upstream.config.credentials.as_ref(),
                resolver,
                max_payload,
                deadline,
                || progress.stage(ProbePhase::ProxyTunnel),
            )
            .await
        })
        .await
        .unwrap_or(Err(ProxyError::Timeout));
        match outcome {
            Ok(socket) => {
                self.traffic_feedback(
                    &upstream.config.id,
                    &upstream.identity,
                    ProbePhase::ProxyTunnel,
                    "udp_associated",
                    false,
                );
                Ok((DatagramIo::Socks(socket), Some(demand)))
            }
            Err(error) => {
                self.record_attempt_failure(&upstream, &progress, &error);
                Err(error)
            }
        }
    }
    async fn direct_datagram(&self, target: &Target) -> Result<UdpSocket, ProxyError> {
        let resolver = self.resolver.read().unwrap().clone();
        let ip = if let Ok(ip) = target.host.parse::<IpAddr>() {
            ip
        } else if let Some(resolver) = resolver {
            resolver
                .lookup(target.host.clone(), hps_net::ResolvePurpose::Business)
                .await
                .map_err(|_| ProxyError::Dns)?
                .into_iter()
                .next()
                .ok_or(ProxyError::Dns)?
        } else {
            tokio::net::lookup_host((target.host.as_str(), target.port))
                .await
                .map_err(|_| ProxyError::Dns)?
                .next()
                .ok_or(ProxyError::Dns)?
                .ip()
        };
        let bind = if ip.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
        let socket = UdpSocket::bind(bind).await.map_err(socket_error)?;
        socket
            .connect(SocketAddr::new(ip, target.port))
            .await
            .map_err(|_| ProxyError::TargetConnect)?;
        Ok(socket)
    }
}
