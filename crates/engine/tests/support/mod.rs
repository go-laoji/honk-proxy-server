#![allow(dead_code)]
use hps_config::{ProxyProtocol, ServerTls, UpstreamConfig};
use hps_engine::BoxIo;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

pub struct Server {
    pub address: SocketAddr,
    pub task: JoinHandle<()>,
    pub requests: Arc<Mutex<Vec<Vec<u8>>>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub struct Certificate {
    pub directory: tempfile::TempDir,
    pub config: ServerTls,
}
pub fn certificate() -> Certificate {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let certificate = directory.path().join("certificate.pem");
    let private_key = directory.path().join("private-key.pem");
    std::fs::write(&certificate, cert.cert.pem()).unwrap();
    std::fs::write(&private_key, cert.signing_key.serialize_pem()).unwrap();
    Certificate {
        directory,
        config: ServerTls {
            certificate,
            private_key,
        },
    }
}
pub fn upstream(address: SocketAddr, protocol: ProxyProtocol) -> UpstreamConfig {
    UpstreamConfig {
        id: "fixture".into(),
        protocol,
        host: address.ip().to_string(),
        port: address.port(),
        enabled: true,
        credentials: None,
        expires_at_unix: None,
        ca_certificate: None,
        tls_server_name: None,
    }
}
pub async fn head(io: &mut BoxIo) -> Vec<u8> {
    let mut bytes = Vec::new();
    while bytes.len() < 64 * 1024 {
        match io.read_u8().await {
            Ok(byte) => bytes.push(byte),
            Err(_) => break,
        }
        if bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    bytes
}
pub async fn echo(ipv6: bool) -> Server {
    let listener = TcpListener::bind(if ipv6 { "[::1]:0" } else { "127.0.0.1:0" })
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn(async move {
        while let Ok((mut client, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut body = Vec::new();
                if client.read_to_end(&mut body).await.is_ok() {
                    let _ = client.write_all(&body).await;
                    let _ = client.shutdown().await;
                }
            });
        }
    });
    Server {
        address,
        task,
        requests,
    }
}

pub async fn http_proxy(
    tls: Option<&Certificate>,
    required_auth: Option<&str>,
    status: u16,
) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let acceptor =
        tls.map(|c| tokio_rustls::TlsAcceptor::from(hps_engine::tls::server(&c.config).unwrap().0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let required_auth = required_auth.map(str::to_owned);
    let task = tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let captured = captured.clone();
            let required_auth = required_auth.clone();
            tokio::spawn(async move {
                let mut io: BoxIo = match acceptor {
                    Some(acceptor) => match acceptor.accept(tcp).await {
                        Ok(tls) => Box::new(tls),
                        Err(_) => return,
                    },
                    None => Box::new(tcp),
                };
                let bytes = head(&mut io).await;
                captured.lock().unwrap().push(bytes.clone());
                let text = String::from_utf8_lossy(&bytes);
                if status != 200
                    || required_auth.as_ref().is_some_and(|a| {
                        !text
                            .to_lowercase()
                            .contains(&format!("proxy-authorization: {}", a.to_lowercase()))
                    })
                {
                    let code = if status == 200 { 407 } else { status };
                    let _ = io
                        .write_all(
                            format!("HTTP/1.1 {code} Failed\r\nContent-Length: 0\r\n\r\n")
                                .as_bytes(),
                        )
                        .await;
                    let _ = io.flush().await;
                    return;
                }
                if let Some(line) = text.lines().next() {
                    let parts: Vec<_> = line.split_whitespace().collect();
                    if parts.first() == Some(&"CONNECT") {
                        if let Ok(mut target) = TcpStream::connect(parts[1]).await {
                            let _ = io
                                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                                .await;
                            let _ = io.flush().await;
                            let _ = tokio::io::copy_bidirectional(&mut io, &mut target).await;
                        }
                    } else {
                        // The fixture forwards absolute-form requests to a real local origin.
                        let uri: http::Uri = parts[1].parse().unwrap();
                        let host = uri.authority().unwrap().as_str();
                        let mut target = TcpStream::connect(host).await.unwrap();
                        let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
                        let mut rewritten = format!("{} {} HTTP/1.1\r\n", parts[0], path);
                        for header in text.lines().skip(1) {
                            if header.is_empty() {
                                break;
                            }
                            if !header.to_lowercase().starts_with("proxy-") {
                                rewritten.push_str(header);
                                rewritten.push_str("\r\n");
                            }
                        }
                        rewritten.push_str("\r\n");
                        target.write_all(rewritten.as_bytes()).await.unwrap();
                        let _ = tokio::io::copy_bidirectional(&mut io, &mut target).await;
                    }
                }
            });
        }
    });
    Server {
        address,
        task,
        requests,
    }
}

pub async fn socks_proxy(credentials: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut io, _)) = listener.accept().await {
            let captured = captured.clone();
            tokio::spawn(async move {
                assert_eq!(io.read_u8().await.unwrap(), 5);
                let n = io.read_u8().await.unwrap();
                let mut methods = vec![0; n as usize];
                io.read_exact(&mut methods).await.unwrap();
                io.write_all(if credentials { &[5, 2] } else { &[5, 0] })
                    .await
                    .unwrap();
                if credentials {
                    assert_eq!(io.read_u8().await.unwrap(), 1);
                    let len = io.read_u8().await.unwrap();
                    let mut user = vec![0; len as usize];
                    io.read_exact(&mut user).await.unwrap();
                    let len = io.read_u8().await.unwrap();
                    let mut password = vec![0; len as usize];
                    io.read_exact(&mut password).await.unwrap();
                    assert_eq!(user, b"supplier");
                    assert_eq!(password, b"secret");
                    io.write_all(&[1, 0]).await.unwrap();
                }
                let mut prefix = [0; 4];
                io.read_exact(&mut prefix).await.unwrap();
                assert_eq!(prefix[..3], [5, 1, 0]);
                let host = match prefix[3] {
                    1 => {
                        let mut b = [0; 4];
                        io.read_exact(&mut b).await.unwrap();
                        std::net::Ipv4Addr::from(b).to_string()
                    }
                    4 => {
                        let mut b = [0; 16];
                        io.read_exact(&mut b).await.unwrap();
                        std::net::Ipv6Addr::from(b).to_string()
                    }
                    3 => {
                        let len = io.read_u8().await.unwrap();
                        let mut b = vec![0; len as usize];
                        io.read_exact(&mut b).await.unwrap();
                        String::from_utf8(b).unwrap()
                    }
                    _ => panic!("address type"),
                };
                let port = io.read_u16().await.unwrap();
                captured.lock().unwrap().push(host.as_bytes().to_vec());
                let mut target = TcpStream::connect((host.as_str(), port)).await.unwrap();
                io.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                    .await
                    .unwrap();
                let _ = tokio::io::copy_bidirectional(&mut io, &mut target).await;
            });
        }
    });
    Server {
        address,
        task,
        requests,
    }
}
