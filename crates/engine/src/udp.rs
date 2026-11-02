//! SOCKS5 UDP wire transport. Rules, pool admission and the bounded transparent
//! association owner are layered on this transport; CONNECT is never UDP.
use crate::{BoxIo, ProxyError, target::Target};
use hps_config::UpstreamCredentials;
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    sync::Mutex,
    time::Instant,
};
const MAX_WIRE: usize = 65507;

pub fn encode_datagram(target: &Target, payload: &[u8]) -> Result<Vec<u8>, ProxyError> {
    // Target fields are public: validate before its domain-length u8 encoding.
    let target = Target::new(&target.host, target.port)?;
    let mut bytes = vec![0, 0, 0];
    bytes.extend(target.socks_bytes());
    if payload.len() > MAX_WIRE - bytes.len() {
        return Err(ProxyError::Protocol);
    }
    bytes.extend_from_slice(payload);
    Ok(bytes)
}
pub fn decode_datagram(bytes: &[u8]) -> Result<(Target, &[u8]), ProxyError> {
    if bytes.len() < 4 || bytes.len() > MAX_WIRE || bytes[..3] != [0, 0, 0] {
        return Err(ProxyError::Protocol);
    }
    let (host, offset) = match bytes[3] {
        1 if bytes.len() >= 10 => (
            std::net::Ipv4Addr::from(<[u8; 4]>::try_from(&bytes[4..8]).unwrap()).to_string(),
            8,
        ),
        4 if bytes.len() >= 22 => (
            std::net::Ipv6Addr::from(<[u8; 16]>::try_from(&bytes[4..20]).unwrap()).to_string(),
            20,
        ),
        3 if bytes.len() >= 5 => {
            let length = usize::from(bytes[4]);
            if length == 0 || bytes.len() < 5 + length + 2 {
                return Err(ProxyError::Protocol);
            }
            (
                std::str::from_utf8(&bytes[5..5 + length])
                    .map_err(|_| ProxyError::Protocol)?
                    .to_owned(),
                5 + length,
            )
        }
        _ => return Err(ProxyError::Protocol),
    };
    let port = u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap());
    let target = Target::new(&host, port).map_err(|_| ProxyError::Protocol)?;
    Ok((target, &bytes[offset + 2..]))
}

pub struct SocksUdpAssociation {
    socket: UdpSocket,
    control: Mutex<BoxIo>,
    closed: AtomicBool,
    max_payload: usize,
}
impl SocksUdpAssociation {
    /// The caller opens the selected SOCKS5 TCP endpoint within the same overall
    /// deadline. No business datagram is sent or retried by this handshake.
    pub async fn associate(
        control: TcpStream,
        credentials: Option<&UpstreamCredentials>,
        resolver: Option<Arc<dyn hps_net::NameResolver>>,
        max_payload: usize,
        deadline: Instant,
    ) -> Result<Self, ProxyError> {
        Self::associate_with_progress(control, credentials, resolver, max_payload, deadline, || {})
            .await
    }
    pub(crate) async fn associate_with_progress(
        control: TcpStream,
        credentials: Option<&UpstreamCredentials>,
        resolver: Option<Arc<dyn hps_net::NameResolver>>,
        max_payload: usize,
        deadline: Instant,
        on_authenticated: impl FnOnce(),
    ) -> Result<Self, ProxyError> {
        if max_payload == 0 || max_payload > MAX_WIRE {
            return Err(ProxyError::Protocol);
        }
        tokio::time::timeout_at(
            deadline,
            Self::prepare(
                control,
                credentials,
                resolver,
                max_payload,
                on_authenticated,
            ),
        )
        .await
        .map_err(|_| ProxyError::Timeout)?
    }
    async fn prepare(
        control: TcpStream,
        credentials: Option<&UpstreamCredentials>,
        resolver: Option<Arc<dyn hps_net::NameResolver>>,
        max_payload: usize,
        on_authenticated: impl FnOnce(),
    ) -> Result<Self, ProxyError> {
        let peer = control.peer_addr()?;
        let local = control.local_addr()?;
        let socket = UdpSocket::bind(SocketAddr::new(local.ip(), 0)).await?;
        let mut control: BoxIo = Box::new(control);
        crate::socks_client::authenticate(&mut control, credentials).await?;
        on_authenticated();
        // External/NAT endpoint is not known. RFC 1928 requires all zeros when
        // the expected client UDP endpoint cannot be supplied reliably.
        let mut request = vec![5, 3, 0, if peer.is_ipv4() { 1 } else { 4 }];
        request.resize(if peer.is_ipv4() { 10 } else { 22 }, 0);
        control.write_all(&request).await?;
        control.flush().await?;
        let mut head = [0; 4];
        control.read_exact(&mut head).await?;
        if head[..3] != [5, 0, 0] {
            return Err(ProxyError::TargetConnect);
        }
        let host = match head[3] {
            1 => {
                let mut bytes = [0; 4];
                control.read_exact(&mut bytes).await?;
                std::net::Ipv4Addr::from(bytes).to_string()
            }
            4 => {
                let mut bytes = [0; 16];
                control.read_exact(&mut bytes).await?;
                std::net::Ipv6Addr::from(bytes).to_string()
            }
            3 => {
                let length = usize::from(control.read_u8().await?);
                if length == 0 {
                    return Err(ProxyError::Protocol);
                }
                let mut bytes = vec![0; length];
                control.read_exact(&mut bytes).await?;
                String::from_utf8(bytes).map_err(|_| ProxyError::Protocol)?
            }
            _ => return Err(ProxyError::Protocol),
        };
        let port = control.read_u16().await?;
        if port == 0 || !hps_config::valid_host(&host) {
            return Err(ProxyError::Protocol);
        }
        let ips = if let Ok(ip) = host.parse::<IpAddr>() {
            vec![if ip.is_unspecified() { peer.ip() } else { ip }]
        } else if let Some(resolver) = resolver {
            resolver
                .lookup(host, hps_net::ResolvePurpose::Bootstrap)
                .await
                .map_err(|_| ProxyError::Dns)?
        } else {
            tokio::net::lookup_host((host, port))
                .await
                .map_err(|_| ProxyError::Dns)?
                .map(|a| a.ip())
                .collect()
        };
        let ip = ips
            .into_iter()
            .find(|ip| ip.is_ipv4() == peer.is_ipv4() && !ip.is_unspecified())
            .ok_or(ProxyError::Protocol)?;
        // A connected UDP descriptor enforces the negotiated relay's source
        // address AND port in the kernel. Unsolicited endpoints are discarded.
        socket.connect(SocketAddr::new(ip, port)).await?;
        Ok(Self {
            socket,
            control: Mutex::new(control),
            closed: AtomicBool::new(false),
            max_payload,
        })
    }
    fn check_open(&self) -> Result<(), ProxyError> {
        if self.closed.load(Ordering::Acquire) {
            Err(ProxyError::Disconnected)
        } else {
            Ok(())
        }
    }
    pub(crate) async fn control_ended(&self) -> ProxyError {
        let mut control = self.control.lock().await;
        let mut byte = [0];
        let result = control.read(&mut byte).await;
        self.closed.store(true, Ordering::Release);
        match result {
            Ok(0) => ProxyError::Disconnected,
            Ok(_) => ProxyError::Protocol,
            Err(e) => ProxyError::Io(e),
        }
    }
    pub async fn send(&self, target: &Target, payload: &[u8]) -> Result<usize, ProxyError> {
        self.check_open()?;
        if payload.len() > self.max_payload {
            return Err(ProxyError::Protocol);
        }
        let bytes = encode_datagram(target, payload)?;
        let length = tokio::select! {biased;error=self.control_ended()=>return Err(error),result=self.socket.send(&bytes)=>result?};
        if length != bytes.len() {
            return Err(ProxyError::Protocol);
        }
        Ok(payload.len())
    }
    /// Invalid fragments/headers or a too-small caller buffer return Protocol;
    /// the datagram is consumed without truncating or replaying business data.
    pub async fn recv(&self, buffer: &mut [u8]) -> Result<(usize, Target), ProxyError> {
        self.check_open()?;
        // Large enough for every non-jumbo UDP payload, so Tokio cannot silently
        // truncate an oversized datagram into an apparently valid SOCKS frame.
        let mut bytes = vec![0; 65536];
        let length = tokio::select! {biased;error=self.control_ended()=>return Err(error),result=self.socket.recv(&mut bytes)=>result?};
        let (source, payload) = decode_datagram(&bytes[..length])?;
        if payload.len() > buffer.len() || payload.len() > self.max_payload {
            return Err(ProxyError::Protocol);
        }
        buffer[..payload.len()].copy_from_slice(payload);
        Ok((payload.len(), source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socks_datagrams_keep_dual_stack_domain_empty_payloads_and_reject_ambiguous_frames() {
        for (host, port) in [
            ("203.0.113.77", 53),
            ("2001:db8::77", 443),
            ("target.example", 8443),
        ] {
            let target = Target::new(host, port).unwrap();
            for payload in [b"".as_slice(), b"opaque-business".as_slice()] {
                let bytes = encode_datagram(&target, payload).unwrap();
                let (decoded, data) = decode_datagram(&bytes).unwrap();
                assert_eq!(decoded, target);
                assert_eq!(data, payload);
                for index in 0..bytes.len() - payload.len() {
                    assert!(decode_datagram(&bytes[..index]).is_err());
                }
                let mut fragmented = bytes.clone();
                fragmented[2] = 1;
                assert!(decode_datagram(&fragmented).is_err());
            }
        }
        let target = Target::new("2001:db8::77", 53).unwrap();
        assert!(encode_datagram(&target, &vec![0; MAX_WIRE - 22]).is_ok());
        assert!(encode_datagram(&target, &vec![0; MAX_WIRE - 21]).is_err());
        assert!(
            encode_datagram(
                &Target {
                    host: "x".repeat(256),
                    port: 53
                },
                b""
            )
            .is_err()
        );
        assert!(decode_datagram(&[0, 0, 0, 3, 0, 0, 53]).is_err());
        assert!(decode_datagram(&[0, 0, 0, 1, 127, 0, 0, 1, 0, 0]).is_err());
    }
}
