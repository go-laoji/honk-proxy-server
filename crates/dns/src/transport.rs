use crate::{DnsError, Result, wire};
use bytes::Bytes;
use hickory_proto::op::Message;
use hps_config::{DnsProtocol, DnsUpstream};
use hps_net::{BoxIo, DnsConnector};
use http_body_util::{BodyExt, Full, Limited};
use sha2::{Digest, Sha256};
use std::{
    io::{BufReader, Read},
    net::SocketAddr,
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    time::Instant,
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, RootCertStore, pki_types::ServerName},
};

pub(crate) struct PreparedUpstream {
    pub config: DnsUpstream,
    pub fingerprint: String,
    tls: Option<Arc<ClientConfig>>,
    connector: Option<Arc<dyn DnsConnector>>,
}
impl PreparedUpstream {
    pub fn prepare(config: DnsUpstream, connector: Option<Arc<dyn DnsConnector>>) -> Result<Self> {
        let mut hash = Sha256::new();
        hash.update(serde_json::to_vec(&config).map_err(|_| DnsError::Configuration)?);
        if let Some(id) = &config.proxy_id {
            hash.update(
                connector
                    .as_ref()
                    .and_then(|c| c.fingerprint(id))
                    .ok_or(DnsError::Configuration)?,
            );
        }
        let tls = if matches!(config.protocol, DnsProtocol::Tls | DnsProtocol::Https) {
            ServerName::try_from(config.server_name.clone().ok_or(DnsError::Configuration)?)
                .map_err(|_| DnsError::Configuration)?;
            let mut roots =
                RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            if let Some(path) = &config.ca_certificate {
                let mut options = std::fs::OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.custom_flags(libc::O_NONBLOCK);
                }
                let file = options.open(path).map_err(|_| DnsError::Configuration)?;
                let metadata = file.metadata().map_err(|_| DnsError::Configuration)?;
                if !metadata.is_file() || metadata.len() > 1024 * 1024 {
                    return Err(DnsError::Configuration);
                }
                let mut data = vec![];
                file.take(1024 * 1024 + 1)
                    .read_to_end(&mut data)
                    .map_err(|_| DnsError::Configuration)?;
                if data.len() > 1024 * 1024 {
                    return Err(DnsError::Configuration);
                }
                hash.update(&data);
                let mut count = 0;
                for cert in rustls_pemfile::certs(&mut BufReader::new(data.as_slice())) {
                    roots
                        .add(cert.map_err(|_| DnsError::Configuration)?)
                        .map_err(|_| DnsError::Configuration)?;
                    count += 1;
                }
                if count == 0 {
                    return Err(DnsError::Configuration);
                }
            }
            let mut tls = ClientConfig::builder_with_provider(Arc::new(
                tokio_rustls::rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .map_err(|_| DnsError::Configuration)?
            .with_root_certificates(roots)
            .with_no_client_auth();
            if config.protocol == DnsProtocol::Https {
                tls.alpn_protocols = vec![b"http/1.1".to_vec()];
            }
            Some(Arc::new(tls))
        } else {
            None
        };
        Ok(Self {
            config,
            fingerprint: format!("{:x}", hash.finalize()),
            tls,
            connector,
        })
    }
    pub async fn exchange(&self, query: &Message, deadline: Instant) -> Result<Message> {
        if self.config.protocol == DnsProtocol::System {
            return self.system(query).await;
        }
        let mut last = DnsError::Transport;
        for address in &self.config.addresses {
            let outcome = match self.config.protocol {
                DnsProtocol::Udp => self.udp(query, *address, deadline).await,
                DnsProtocol::Tcp | DnsProtocol::Tls => self.tcp(query, *address, deadline).await,
                DnsProtocol::Https => self.https(query, *address, deadline).await,
                DnsProtocol::System => unreachable!(),
            };
            match outcome {
                Ok(response) => return Ok(response),
                Err(error) => last = error,
            }
            if Instant::now() >= deadline {
                return Err(DnsError::Timeout);
            }
        }
        Err(last)
    }
    async fn system(&self, query: &Message) -> Result<Message> {
        use hickory_proto::{
            op::OpCode,
            rr::{
                RData, Record, RecordType,
                rdata::{A, AAAA},
            },
        };
        let question = &query.queries[0];
        if !matches!(question.query_type(), RecordType::A | RecordType::AAAA) {
            return Err(DnsError::SystemQueryType);
        }
        let host = question.name().to_ascii();
        let addresses = crate::system::lookup(host).await?;
        let mut response = Message::response(query.metadata.id, OpCode::Query);
        response.queries = query.queries.clone();
        response.metadata.recursion_desired = query.metadata.recursion_desired;
        response.metadata.recursion_available = true;
        let mut seen = std::collections::HashSet::new();
        for ip in addresses.into_iter().filter(|ip| seen.insert(*ip)).take(64) {
            let data = match (ip, question.query_type()) {
                (std::net::IpAddr::V4(ip), RecordType::A) => RData::A(A(ip)),
                (std::net::IpAddr::V6(ip), RecordType::AAAA) => RData::AAAA(AAAA(ip)),
                _ => continue,
            };
            response
                .answers
                .push(Record::from_rdata(question.name().clone(), 0, data));
        }
        Ok(response)
    }
    async fn stream(&self, address: SocketAddr, deadline: Instant) -> Result<BoxIo> {
        let io: BoxIo = if let Some(id) = &self.config.proxy_id {
            self.connector
                .as_ref()
                .ok_or(DnsError::Configuration)?
                .connect(id.clone(), address, deadline)
                .await
                .map_err(|_| DnsError::Egress)?
        } else {
            Box::new(
                TcpStream::connect(address)
                    .await
                    .map_err(|_| DnsError::Transport)?,
            )
        };
        if let Some(tls) = &self.tls {
            let name = ServerName::try_from(
                self.config
                    .server_name
                    .clone()
                    .ok_or(DnsError::Configuration)?,
            )
            .map_err(|_| DnsError::Configuration)?;
            Ok(Box::new(
                TlsConnector::from(tls.clone())
                    .connect(name, io)
                    .await
                    .map_err(|_| DnsError::Tls)?,
            ))
        } else {
            Ok(io)
        }
    }
    async fn udp(
        &self,
        query: &Message,
        address: SocketAddr,
        deadline: Instant,
    ) -> Result<Message> {
        let socket = UdpSocket::bind(if address.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })
        .await
        .map_err(|_| DnsError::Transport)?;
        socket
            .connect(address)
            .await
            .map_err(|_| DnsError::Transport)?;
        let wire = query.to_vec().map_err(|_| DnsError::InvalidQuery)?;
        socket.send(&wire).await.map_err(|_| DnsError::Transport)?;
        let mut buffer = vec![0; 65535];
        loop {
            let size = socket
                .recv(&mut buffer)
                .await
                .map_err(|_| DnsError::Transport)?;
            let Ok(response) = Message::from_vec(&buffer[..size]) else {
                continue;
            };
            if wire::validate_response(query, &response).is_err() {
                continue;
            }
            if response.metadata.truncation {
                return self.tcp(query, address, deadline).await;
            }
            return Ok(response);
        }
    }
    async fn tcp(
        &self,
        query: &Message,
        address: SocketAddr,
        deadline: Instant,
    ) -> Result<Message> {
        let mut io = self.stream(address, deadline).await?;
        let wire = query.to_vec().map_err(|_| DnsError::InvalidQuery)?;
        io.write_u16(wire.len().try_into().map_err(|_| DnsError::InvalidQuery)?)
            .await
            .map_err(|_| DnsError::Transport)?;
        io.write_all(&wire).await.map_err(|_| DnsError::Transport)?;
        let len = io.read_u16().await.map_err(|_| DnsError::Transport)? as usize;
        let mut buffer = vec![0; len];
        io.read_exact(&mut buffer)
            .await
            .map_err(|_| DnsError::Transport)?;
        let response = Message::from_vec(&buffer).map_err(|_| DnsError::InvalidResponse)?;
        wire::validate_response(query, &response)?;
        if response.metadata.truncation {
            return Err(DnsError::InvalidResponse);
        }
        Ok(response)
    }
    async fn https(
        &self,
        query: &Message,
        address: SocketAddr,
        deadline: Instant,
    ) -> Result<Message> {
        let io = self.stream(address, deadline).await?;
        let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
            .max_buf_size(16384)
            .handshake(hyper_util::rt::TokioIo::new(io))
            .await
            .map_err(|_| DnsError::Http)?;
        let driver = tokio::spawn(connection);
        struct Driver(tokio::task::JoinHandle<std::result::Result<(), hyper::Error>>);
        impl Drop for Driver {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _driver = Driver(driver);
        let host = self
            .config
            .server_name
            .as_ref()
            .ok_or(DnsError::Configuration)?;
        let authority = if host.contains(':') {
            format!("[{host}]:{}", address.port())
        } else {
            format!("{host}:{}", address.port())
        };
        let request =
            http::Request::post(self.config.https_path.as_deref().unwrap_or("/dns-query"))
                .header("host", authority)
                .header("content-type", "application/dns-message")
                .header("accept", "application/dns-message")
                .body(Full::new(Bytes::from(
                    query.to_vec().map_err(|_| DnsError::InvalidQuery)?,
                )))
                .map_err(|_| DnsError::Configuration)?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|_| DnsError::Http)?;
        if response.status() != http::StatusCode::OK
            || response
                .headers()
                .get("content-type")
                .and_then(|s| s.to_str().ok())
                .is_none_or(|s| {
                    !s.split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .eq_ignore_ascii_case("application/dns-message")
                })
        {
            return Err(DnsError::Http);
        }
        // RFC 8484 section 5.1: a cached HTTP response has already consumed this
        // much of every DNS RR's lifetime, including negative SOA lifetime.
        let age = match response.headers().get(http::header::AGE) {
            None => 0,
            Some(value) => value
                .to_str()
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or(DnsError::Http)?
                .min(u32::MAX as u64) as u32,
        };
        let data = Limited::new(response.into_body(), 65535)
            .collect()
            .await
            .map_err(|_| DnsError::Http)?
            .to_bytes();
        let response = Message::from_vec(&data).map_err(|_| DnsError::InvalidResponse)?;
        wire::validate_response(query, &response)?;
        if response.metadata.truncation {
            return Err(DnsError::InvalidResponse);
        }
        Ok(wire::age(&response, age, response.metadata.id))
    }
}
