//! UDP/TCP DNS ingress. Binding is separate from activation so configuration CAS
//! cannot publish a listener which failed to bind. Each request holds one resolver generation.
use crate::{DnsError, Resolver, wire};
use hickory_proto::op::{Message, OpCode, ResponseCode};
use hps_net::ResolvePurpose;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::Semaphore,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub struct DnsServer {
    configured: SocketAddr,
    local: SocketAddr,
    sockets: Mutex<Option<(TcpListener, UdpSocket)>>,
    resolver: RwLock<Resolver>,
    stop_accepting: CancellationToken,
    stop_connections: CancellationToken,
    tasks: TaskTracker,
}
impl DnsServer {
    pub async fn bind(address: SocketAddr, resolver: Resolver) -> std::io::Result<Arc<Self>> {
        if resolver
            .configuration()
            .upstreams
            .iter()
            .any(|u| u.protocol == hps_config::DnsProtocol::System)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "DNS listeners require explicit wire upstreams",
            ));
        }
        let tcp = TcpListener::bind(address).await?;
        let local = tcp.local_addr()?;
        let udp = UdpSocket::bind(local).await?;
        #[cfg(target_os = "linux")]
        hps_net::packet_info::enable(&udp)?;
        Ok(Arc::new(Self {
            configured: address,
            local,
            sockets: Mutex::new(Some((tcp, udp))),
            resolver: RwLock::new(resolver),
            stop_accepting: CancellationToken::new(),
            stop_connections: CancellationToken::new(),
            tasks: TaskTracker::new(),
        }))
    }
    #[cfg(target_os = "linux")]
    pub async fn bind_capture(
        address: SocketAddr,
        resolver: Resolver,
    ) -> std::io::Result<Arc<Self>> {
        if resolver
            .configuration()
            .upstreams
            .iter()
            .any(|u| u.protocol == hps_config::DnsProtocol::System)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "DNS capture requires explicit wire upstreams",
            ));
        }
        let tcp = hps_net::packet_info::bind_capture_tcp(address)?;
        let local = tcp.local_addr()?;
        let udp = hps_net::packet_info::bind_capture_udp(local)?;
        Ok(Arc::new(Self {
            configured: address,
            local,
            sockets: Mutex::new(Some((tcp, udp))),
            resolver: RwLock::new(resolver),
            stop_accepting: CancellationToken::new(),
            stop_connections: CancellationToken::new(),
            tasks: TaskTracker::new(),
        }))
    }
    pub fn configured_address(&self) -> SocketAddr {
        self.configured
    }
    pub fn local_address(&self) -> SocketAddr {
        self.local
    }
    pub fn replace_resolver(&self, resolver: Resolver) {
        *self.resolver.write().unwrap() = resolver;
    }
    pub fn start(self: &Arc<Self>) {
        let Some((tcp, udp)) = self.sockets.lock().unwrap().take() else {
            return;
        };
        let service = self.clone();
        self.tasks.spawn(async move {
            let slots=Arc::new(Semaphore::new(64));
            loop {
                let accepted=tokio::select! {biased;_=service.stop_accepting.cancelled()=>break,
                    result=tcp.accept()=>result};
                let Ok((stream,_))=accepted else {break};
                let Ok(permit)=slots.clone().try_acquire_owned() else {continue};
                let child=service.clone();
                service.tasks.spawn(async move {
                    let _permit=permit;
                    let _=tokio::select! {_=child.stop_connections.cancelled()=>Ok(()),
                        result=tokio::time::timeout(Duration::from_secs(120),serve_tcp(stream,child.clone()))=>result.unwrap_or(Ok(()))};
                });
            }
        });
        let service = self.clone();
        self.tasks.spawn(async move {
            let socket = Arc::new(udp);
            let slots = Arc::new(Semaphore::new(256));
            let mut buffer = vec![0; 65535];
            loop {
                let received = tokio::select! {biased;_=service.stop_accepting.cancelled()=>break,
                result=recv_packet(&socket,&mut buffer)=>result};
                let (len, peer, metadata) = match received {
                    Ok(packet) => packet,
                    Err(e) if e.kind() == std::io::ErrorKind::InvalidData => continue,
                    Err(_) => break,
                };
                if len > 4096 {
                    continue;
                }
                let Ok(permit) = slots.clone().try_acquire_owned() else {
                    continue;
                };
                let Ok(request) = Message::from_vec(&buffer[..len]) else {
                    continue;
                };
                let socket = socket.clone();
                let child = service.clone();
                let resolver = child.resolver.read().unwrap().clone();
                service.tasks.spawn(async move {
                    let _permit = permit;
                    let limit = request
                        .edns
                        .as_ref()
                        .map_or(512, |e| e.max_payload().clamp(512, 4096))
                        as usize;
                    let response = tokio::select! {_=child.stop_connections.cancelled()=>return,
                    response=reply(&resolver, request)=>response};
                    let mut data = response.to_vec().unwrap_or_default();
                    if data.len() > limit {
                        let mut truncated = response;
                        truncated.metadata.truncation = true;
                        truncated.answers.clear();
                        truncated.authorities.clear();
                        truncated.additionals.clear();
                        // Keep only minimal EDNS metadata. Padding/cookies/options
                        // from an oversized upstream must not exceed the client's
                        // UDP limit after the answer sections have been removed.
                        if let Some(old) = truncated.edns.take() {
                            let mut minimal = hickory_proto::op::Edns::new();
                            minimal
                                .set_max_payload(limit as u16)
                                .set_rcode_high(old.rcode_high())
                                .set_dnssec_ok(old.flags().dnssec_ok);
                            truncated.edns = Some(minimal);
                        }
                        data = truncated.to_vec().unwrap_or_default();
                    }
                    if data.len() > limit {
                        return;
                    }
                    let _ = send_packet(&socket, &data, peer, metadata).await;
                });
            }
        });
    }
    pub async fn shutdown(&self) {
        self.stop_accepting.cancel();
        self.tasks.close();
        if tokio::time::timeout(Duration::from_secs(6), self.tasks.wait())
            .await
            .is_err()
        {
            self.stop_connections.cancel();
            self.tasks.wait().await;
        }
    }
}
#[cfg(target_os = "linux")]
type PacketMetadata = hps_net::packet_info::PacketInfo;
#[cfg(not(target_os = "linux"))]
type PacketMetadata = ();
async fn recv_packet(
    socket: &UdpSocket,
    buffer: &mut [u8],
) -> std::io::Result<(usize, SocketAddr, PacketMetadata)> {
    #[cfg(target_os = "linux")]
    {
        hps_net::packet_info::recv(socket, buffer).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        socket
            .recv_from(buffer)
            .await
            .map(|(len, peer)| (len, peer, ()))
    }
}
async fn send_packet(
    socket: &UdpSocket,
    buffer: &[u8],
    peer: SocketAddr,
    _metadata: PacketMetadata,
) -> std::io::Result<usize> {
    #[cfg(target_os = "linux")]
    {
        hps_net::packet_info::send(socket, buffer, peer, _metadata).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        socket.send_to(buffer, peer).await
    }
}
impl Drop for DnsServer {
    fn drop(&mut self) {
        self.stop_accepting.cancel();
        self.stop_connections.cancel();
    }
}
async fn serve_tcp(mut stream: TcpStream, service: Arc<DnsServer>) -> std::io::Result<()> {
    for _ in 0..64 {
        let len = match tokio::time::timeout(Duration::from_secs(5), stream.read_u16()).await {
            Ok(Ok(len)) if (12..=4096).contains(&len) => len as usize,
            _ => return Ok(()),
        };
        let mut data = vec![0; len];
        if !matches!(
            tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut data)).await,
            Ok(Ok(_))
        ) {
            return Ok(());
        }
        let Ok(request) = Message::from_vec(&data) else {
            return Ok(());
        };
        let resolver = service.resolver.read().unwrap().clone();
        let data = reply(&resolver, request).await.to_vec().unwrap_or_default();
        if !matches!(
            tokio::time::timeout(Duration::from_secs(5), async {
                stream.write_u16(data.len() as u16).await?;
                stream.write_all(&data).await
            })
            .await,
            Ok(Ok(()))
        ) {
            return Ok(());
        }
    }
    Ok(())
}
async fn reply(resolver: &Resolver, request: Message) -> Message {
    let id = request.metadata.id;
    let valid = wire::validate_query(&request).is_ok();
    let question = request.queries.first().cloned();
    let rd = request.metadata.recursion_desired;
    match resolver.exchange(request, ResolvePurpose::Business).await {
        Ok(answer) => answer.message,
        Err(error) => {
            let code = if !valid || error == DnsError::InvalidQuery {
                ResponseCode::FormErr
            } else {
                ResponseCode::ServFail
            };
            let mut response = Message::error_msg(id, OpCode::Query, code);
            response.metadata.recursion_desired = rd;
            response.metadata.recursion_available = true;
            response.queries = question.into_iter().collect();
            response
        }
    }
}
