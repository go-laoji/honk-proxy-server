//! RFC 1928 association owner. The TCP peer owns the UDP socket and all flows.
use crate::{BoxIo, ProxyError, runtime::Session, target::Target, udp};
use hps_domain::SelectionTransport;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UdpSocket,
    sync::mpsc,
    task::JoinSet,
};

const MAX_TARGETS: usize = 32;
const QUEUE: usize = 8;
const MAX_WIRE: usize = 65507;

pub(crate) async fn serve(
    mut control: BoxIo,
    session: Session,
    requested_ip: IpAddr,
    requested_port: u16,
) -> Result<(), ProxyError> {
    let peer_ip = session
        .selection_context
        .source_ip
        .ok_or(ProxyError::InvalidTarget)?;
    if !requested_ip.is_unspecified() && requested_ip.to_canonical() != peer_ip.to_canonical() {
        control.write_all(&[5, 2, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Err(ProxyError::Rejected);
    }
    if !session.dialer.can_forward_transport(
        session.selection_context.inbound.as_deref().unwrap_or(""),
        "socks5",
        SelectionTransport::Udp,
    ) {
        control.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Err(ProxyError::NoUpstream);
    }
    // Bind on the TCP's actual local address, never an advertised 0.0.0.0/::.
    let socket = Arc::new(UdpSocket::bind(SocketAddr::new(session.local_address.ip(), 0)).await?);
    let address = socket.local_addr()?;
    let mut response = vec![5, 0, 0];
    response.extend(Target::new(&address.ip().to_string(), address.port())?.socks_bytes());
    control.write_all(&response).await?;
    control.flush().await?;
    let association = session.cancel.child_token();
    let _cancel_on_exit = association.clone().drop_guard();
    let mut locked_peer = (requested_port != 0).then(|| SocketAddr::new(peer_ip, requested_port));
    let mut targets: HashMap<String, mpsc::Sender<Vec<u8>>> = HashMap::new();
    let mut tasks = JoinSet::new();
    let mut packet = vec![0; 65536];
    loop {
        tokio::select! { biased;
            // Any control data, EOF or error ends this association. It is not a CONNECT stream.
            _ = control.read_u8() => return Ok(()),
            _ = association.cancelled() => return Ok(()),
            Some(_) = tasks.join_next(), if !tasks.is_empty() => { targets.retain(|_, tx| !tx.is_closed()); },
            received = socket.recv_from(&mut packet) => {
                let (length, peer) = received?;
                if peer.ip().to_canonical() != peer_ip.to_canonical() || locked_peer.is_some_and(|expected| expected != peer) { continue; }
                // RSV/FRAG/address/size validation occurs before endpoint pinning.
                let Ok((target, payload)) = udp::decode_datagram(&packet[..length]) else { continue; };
                let payload = payload.to_vec();
                locked_peer = Some(peer);
                session.activity.touch();
                let key = target.authority();
                if targets.get(&key).is_some_and(|tx| tx.is_closed()) { targets.remove(&key); }
                if !targets.contains_key(&key) {
                    if targets.len() >= MAX_TARGETS { continue; }
                    let Some(permits) = session.udp_target_permits() else { continue; };
                    let (tx, rx) = mpsc::channel(QUEUE);
                    targets.insert(key.clone(), tx);
                    let child = session.clone(); let socket = socket.clone(); let cancel = association.child_token();
                    tasks.spawn(async move {
                        let _permits = permits;
                        tokio::select! { biased;
                            _ = cancel.cancelled() => {},
                            _ = flow(socket, peer, target, rx, child) => {},
                        }
                    });
                }
                // Drop excess datagrams; do not allocate an unbounded pending-send list.
                if let Some(tx) = targets.get(&key) { let _ = tx.try_send(payload); }
            }
        }
    }
}
async fn flow(
    socket: Arc<UdpSocket>,
    peer: SocketAddr,
    target: Target,
    mut receiver: mpsc::Receiver<Vec<u8>>,
    session: Session,
) {
    let mut query = session.selection_context.clone();
    query.source_port = Some(peer.port());
    query.transport = SelectionTransport::Udp;
    let context = session.dialer.routing_context(&target, &query);
    let Ok(connection) = session
        .dialer
        .open_udp_routed(&target, &context, MAX_WIRE - 262)
        .await
    else {
        return;
    };
    // Management disconnect ends this flow, without cancelling unrelated targets.
    let cancel = connection.observation().cancellation();
    let idle = crate::traffic::Activity::new();
    let sending = async {
        while let Some(payload) = receiver.recv().await {
            session.budget.wait(payload.len()).await;
            if connection.send(&payload).await.is_err() {
                return;
            }
            idle.touch();
            session.activity.touch();
        }
    };
    let replying = async {
        let mut buffer = vec![0; MAX_WIRE];
        loop {
            let (length, source) = match connection.recv(&mut buffer).await {
                Ok(value) => value,
                Err(ProxyError::Protocol) => {
                    tokio::task::yield_now().await;
                    continue;
                }
                Err(_) => return,
            };
            let Ok(wire) = udp::encode_datagram(&source, &buffer[..length]) else {
                continue;
            };
            session.budget.wait(length).await;
            if socket.send_to(&wire, peer).await.is_err() {
                return;
            }
            idle.touch();
            session.activity.touch();
        }
    };
    tokio::select! { biased;
        _ = cancel.cancelled() => {},
        _ = connection.transport_ended() => {},
        _ = idle.idle(Duration::from_millis(session.limits.idle_timeout_ms)) => {},
        _ = sending => {},
        _ = replying => {},
    }
}
