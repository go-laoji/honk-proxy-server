mod support;
use hps_config::{ProxyConfig, ProxyProtocol, UpstreamCredentials};
use hps_engine::{BoxIo, runtime::Engine, target::Target};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

async fn wait_for(mut condition: impl FnMut() -> bool) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(condition(), "condition did not become true");
}

#[tokio::test]
async fn all_nine_ingress_egress_pairs_stream_http_and_end_to_end_https() {
    let certificate = support::certificate();
    let http_origin = support::origin(None).await;
    let https_origin = support::origin(Some(&certificate)).await;
    for upstream_protocol in [
        ProxyProtocol::Http,
        ProxyProtocol::Https,
        ProxyProtocol::Socks5,
    ] {
        let upstream = if upstream_protocol == ProxyProtocol::Socks5 {
            support::socks_proxy(true).await
        } else {
            support::http_proxy(
                (upstream_protocol == ProxyProtocol::Https).then_some(&certificate),
                Some("Basic c3VwcGxpZXI6c2VjcmV0"),
                200,
            )
            .await
        };
        let mut upstream_config = support::upstream(upstream.address, upstream_protocol);
        upstream_config.credentials = Some(UpstreamCredentials {
            username: "supplier".into(),
            password: "secret".into(),
        });
        if upstream_protocol == ProxyProtocol::Https {
            upstream_config.ca_certificate = Some(certificate.config.certificate.clone());
            upstream_config.tls_server_name = Some("localhost".into());
        }
        let engine = Engine::new(ProxyConfig {
            upstreams: vec![upstream_config],
            inbounds: vec![
                support::inbound("http", ProxyProtocol::Http, None),
                support::inbound("https", ProxyProtocol::Https, Some(&certificate)),
                support::inbound("socks", ProxyProtocol::Socks5, None),
            ],
            ..Default::default()
        })
        .unwrap();
        engine.start_enabled().await.unwrap();
        assert!(engine.ready());
        for inbound in engine.status().inbounds {
            let address: std::net::SocketAddr = inbound.listen.unwrap().parse().unwrap();
            let proxy = match inbound.protocol.as_str() {
                "http" => format!("http://{address}"),
                "https" => format!("https://localhost:{}", address.port()),
                "socks5" => format!("socks5h://{address}"),
                _ => unreachable!(),
            };
            let client = reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(&proxy).unwrap())
                .add_root_certificate(
                    reqwest::Certificate::from_pem(
                        &std::fs::read(&certificate.config.certificate).unwrap(),
                    )
                    .unwrap(),
                )
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap();
            for url in [
                format!("http://{}/echo?value=1", http_origin.address),
                format!(
                    "https://localhost:{}/echo?value=2",
                    https_origin.address.port()
                ),
            ] {
                let data = vec![42; 2 * 1024 * 1024];
                let inspect_http = url.starts_with("http://") && inbound.protocol != "socks5";
                let request = client.post(url);
                let request = if inspect_http {
                    request
                        .header("Proxy-Authorization", "Basic browser-secret")
                        .header("Connection", "X-Remove")
                        .header("X-Remove", "private-hop")
                } else {
                    request
                };
                let response = request.body(data.clone()).send().await.unwrap();
                assert_eq!(
                    response.status(),
                    200,
                    "{upstream_protocol:?} -> {}",
                    inbound.protocol
                );
                if inspect_http {
                    assert!(!response.headers().contains_key("X-Origin-Hop"));
                    assert!(!response.headers().contains_key("Proxy-Authenticate"));
                }
                assert_eq!(response.bytes().await.unwrap().as_ref(), data);
            }
            if inbound.protocol == "https" {
                assert!(inbound.certificate.unwrap().valid_now);
            }
        }
        engine.shutdown().await;
        assert_eq!(engine.status().active_connections, 0);
    }
    for request in http_origin.requests.lock().unwrap().iter() {
        let request = String::from_utf8_lossy(request).to_lowercase();
        assert!(!request.contains("proxy-authorization"));
        assert!(!request.contains("browser-secret"));
        assert!(!request.contains("private-hop"));
    }
}

#[tokio::test]
async fn no_upstream_is_503_and_unsupported_socks_methods_and_commands_are_explicit() {
    let engine = Engine::new(ProxyConfig {
        inbounds: vec![
            support::inbound("http", ProxyProtocol::Http, None),
            support::inbound("socks", ProxyProtocol::Socks5, None),
        ],
        ..Default::default()
    })
    .unwrap();
    engine.start_enabled().await.unwrap();
    assert!(!engine.ready());
    let statuses = engine.status().inbounds;
    let http = statuses
        .iter()
        .find(|i| i.id == "http")
        .unwrap()
        .listen
        .as_ref()
        .unwrap();
    let mut io: BoxIo = Box::new(TcpStream::connect(http).await.unwrap());
    io.write_all(b"GET http://127.0.0.1:1/ HTTP/1.1\r\nHost: ignored\r\n\r\n")
        .await
        .unwrap();
    let response = String::from_utf8(support::head(&mut io).await).unwrap();
    assert!(response.starts_with("HTTP/1.1 503"));
    assert!(!response.contains("407"));
    let socks = statuses
        .iter()
        .find(|i| i.id == "socks")
        .unwrap()
        .listen
        .as_ref()
        .unwrap();
    let mut io = TcpStream::connect(socks).await.unwrap();
    io.write_all(&[5, 1, 2]).await.unwrap();
    let mut bytes = [0; 2];
    io.read_exact(&mut bytes).await.unwrap();
    assert_eq!(bytes, [5, 255]);
    for command in [2, 3] {
        let mut io = TcpStream::connect(socks).await.unwrap();
        io.write_all(&[5, 1, 0]).await.unwrap();
        io.read_exact(&mut bytes).await.unwrap();
        assert_eq!(bytes, [5, 0]);
        io.write_all(&[5, command, 0, 1, 127, 0, 0, 1, 0, 80])
            .await
            .unwrap();
        let mut reply = [0; 10];
        io.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[1], 7);
    }
    engine.shutdown().await;
}

#[tokio::test]
async fn start_stop_reclaims_ports_connections_and_global_or_entry_admission() {
    for global in [true, false] {
        let mut inbound = support::inbound("http", ProxyProtocol::Http, None);
        inbound.max_connections = if global { 8 } else { 1 };
        let mut config = ProxyConfig {
            inbounds: vec![inbound],
            ..Default::default()
        };
        config.limits.max_connections = if global { 1 } else { 8 };
        let engine = Engine::new(config).unwrap();
        engine.start_enabled().await.unwrap();
        let address = engine.status().inbounds[0].listen.clone().unwrap();
        let mut first = TcpStream::connect(&address).await.unwrap();
        first.write_all(b"G").await.unwrap();
        wait_for(|| engine.status().active_connections == 1).await;
        let mut rejected = TcpStream::connect(&address).await.unwrap();
        let mut bytes = [0; 1];
        assert_eq!(rejected.read(&mut bytes).await.unwrap(), 0);
        let status = engine.status().inbounds.remove(0);
        assert_eq!(
            if global {
                status.rejected_global_limit
            } else {
                status.rejected_inbound_limit
            },
            1
        );
        let stopped = engine.stop("http").await.unwrap();
        assert!(!stopped.running);
        assert_eq!(engine.status().active_connections, 0);
        assert_eq!(first.read(&mut bytes).await.unwrap(), 0);
        assert!(TcpStream::connect(&address).await.is_err());
        assert!(engine.start("http").await.unwrap().running);
        engine.shutdown().await;
        assert!(engine.start("http").await.is_err());
    }
}

#[tokio::test]
async fn idle_timeout_connect_timeout_rate_limit_and_ipv6_half_close_are_real() {
    let destination = support::echo(true).await;
    let upstream = support::http_proxy(None, None, 200).await;
    let mut inbound = support::inbound("http", ProxyProtocol::Http, None);
    inbound.bytes_per_second = 64 * 1024;
    let mut config = ProxyConfig {
        inbounds: vec![inbound],
        upstreams: vec![support::upstream(upstream.address, ProxyProtocol::Http)],
        ..Default::default()
    };
    config.limits.idle_timeout_ms = 80;
    config.limits.handshake_timeout_ms = 1000;
    let engine = Engine::new(config).unwrap();
    engine.start_enabled().await.unwrap();
    let address = engine.status().inbounds[0].listen.clone().unwrap();
    let mut idle = TcpStream::connect(&address).await.unwrap();
    let mut buf = [0; 1024];
    tokio::time::timeout(Duration::from_secs(2), idle.read(&mut buf))
        .await
        .unwrap()
        .unwrap();
    wait_for(|| engine.status().active_connections == 0).await;
    assert_eq!(engine.status().inbounds[0].idle_timeouts, 1);
    let target = Target::new("::1", destination.address.port()).unwrap();
    let mut io: BoxIo = Box::new(TcpStream::connect(&address).await.unwrap());
    io.write_all(
        format!(
            "CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n",
            target.authority(),
            target.authority()
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    assert!(support::head(&mut io).await.starts_with(b"HTTP/1.1 200"));
    let started = tokio::time::Instant::now();
    let data = vec![23; 16 * 1024];
    io.write_all(&data).await.unwrap();
    io.shutdown().await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), io.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response, data);
    assert!(started.elapsed() >= Duration::from_millis(450));
    engine.shutdown().await;
}

#[tokio::test]
async fn upstream_handshake_timeout_and_lost_post_response_never_replay() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let blackhole = tokio::spawn(async move {
        let mut sockets = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            sockets.push(socket);
        }
    });
    let mut config = ProxyConfig {
        inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
        upstreams: vec![support::upstream(address, ProxyProtocol::Http)],
        ..Default::default()
    };
    config.limits.connect_timeout_ms = 50;
    let engine = Engine::new(config).unwrap();
    engine.start_enabled().await.unwrap();
    let address = engine.status().inbounds[0].listen.clone().unwrap();
    let mut io: BoxIo = Box::new(TcpStream::connect(address).await.unwrap());
    io.write_all(b"CONNECT target.test:443 HTTP/1.1\r\nHost: target.test:443\r\n\r\n")
        .await
        .unwrap();
    let head = tokio::time::timeout(Duration::from_secs(2), support::head(&mut io))
        .await
        .unwrap();
    assert!(head.starts_with(b"HTTP/1.1 504"));
    engine.shutdown().await;
    blackhole.abort();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let captured = count.clone();
    let broken = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let captured = captured.clone();
            tokio::spawn(async move {
                let mut io: BoxIo = Box::new(socket);
                let head = support::head(&mut io).await;
                assert!(head.starts_with(b"POST http://origin.test/"));
                let mut data = [0; 3];
                io.read_exact(&mut data).await.unwrap();
                assert_eq!(&data, b"pay");
                captured.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // A charge may have committed. Close without sending a response.
            });
        }
    });
    let unused = support::http_proxy(None, None, 200).await;
    let mut second = support::upstream(unused.address, ProxyProtocol::Http);
    second.id = "unused".into();
    let engine = Engine::new(ProxyConfig {
        inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
        upstreams: vec![support::upstream(address, ProxyProtocol::Http), second],
        ..Default::default()
    })
    .unwrap();
    engine.start_enabled().await.unwrap();
    let address = engine.status().inbounds[0].listen.clone().unwrap();
    let mut io: BoxIo = Box::new(TcpStream::connect(address).await.unwrap());
    io.write_all(b"POST http://origin.test/charge HTTP/1.1\r\nHost: wrong.test\r\nContent-Length: 3\r\n\r\npay").await.unwrap();
    let head = tokio::time::timeout(Duration::from_secs(2), support::head(&mut io))
        .await
        .unwrap();
    assert!(head.starts_with(b"HTTP/1.1 502"));
    assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(unused.requests.lock().unwrap().is_empty());
    engine.shutdown().await;
    broken.abort();
}

#[tokio::test]
async fn chunked_framing_host_rewrite_and_header_rejection_are_bounded() {
    let origin = support::origin(None).await;
    let upstream = support::http_proxy(None, None, 200).await;
    let mut config = ProxyConfig {
        inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
        upstreams: vec![support::upstream(upstream.address, ProxyProtocol::Http)],
        ..Default::default()
    };
    config.limits.max_header_bytes = 8192;
    let engine = Engine::new(config).unwrap();
    engine.start_enabled().await.unwrap();
    let address = engine.status().inbounds[0].listen.clone().unwrap();
    let mut io: BoxIo = Box::new(TcpStream::connect(&address).await.unwrap());
    io.write_all(format!("POST http://{}/stream HTTP/1.1\r\nHost: forged.test\r\nTransfer-Encoding: chunked\r\nTrailer: X-End\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\nX-End: done\r\n\r\n",origin.address).as_bytes()).await.unwrap();
    let head = support::head(&mut io).await;
    assert!(head.starts_with(b"HTTP/1.1 200"));
    let mut body = [0; 5];
    io.read_exact(&mut body).await.unwrap();
    assert_eq!(&body, b"abcde");
    assert!(!String::from_utf8_lossy(&origin.requests.lock().unwrap()[0]).contains("forged.test"));
    for (request,status) in [
        ("GET http://127.0.0.1:99999/ HTTP/1.1\r\nHost: example.test\r\n\r\n".to_string(),400),
        ("POST http://origin.test/ HTTP/1.1\r\nHost: origin.test\r\nContent-Length: 3\r\nContent-Length: 5\r\n\r\nabcde".to_string(),400),
        ("GET http://origin.test/ HTTP/1.1\r\nHost: origin.test\r\nConnection: upgrade\r\nUpgrade: websocket\r\n\r\n".to_string(),501),
        (format!("GET http://origin.test/ HTTP/1.1\r\nHost: origin.test\r\nX-Large: {}\r\n\r\n","a".repeat(16000)),431),
    ] {
        let mut io:BoxIo=Box::new(TcpStream::connect(&address).await.unwrap());let _=io.write_all(request.as_bytes()).await;
        let head=tokio::time::timeout(Duration::from_secs(2),support::head(&mut io)).await.unwrap();
        assert!(head.starts_with(format!("HTTP/1.1 {status}").as_bytes()),"{}",String::from_utf8_lossy(&head));
    }
    assert_eq!(upstream.requests.lock().unwrap().len(), 1);
    engine.shutdown().await;
}

#[tokio::test]
async fn cancellation_and_partial_start_failure_release_all_runtime_resources() {
    let blocked = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first = support::inbound("a", ProxyProtocol::Http, None);
    let mut second = support::inbound("b", ProxyProtocol::Http, None);
    second.listen = blocked.local_addr().unwrap();
    let engine = Engine::new(ProxyConfig {
        inbounds: vec![first, second],
        ..Default::default()
    })
    .unwrap();
    assert!(engine.start_enabled().await.is_err());
    assert!(
        engine
            .status()
            .inbounds
            .iter()
            .all(|i| !i.running && i.listen.is_none())
    );
    assert_eq!(engine.status().active_connections, 0);
    let destination = support::echo(false).await;
    let upstream = support::http_proxy(None, None, 200).await;
    let engine = Engine::new(ProxyConfig {
        inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
        upstreams: vec![support::upstream(upstream.address, ProxyProtocol::Http)],
        ..Default::default()
    })
    .unwrap();
    engine.start_enabled().await.unwrap();
    let mut io: BoxIo = Box::new(
        TcpStream::connect(engine.status().inbounds[0].listen.as_ref().unwrap())
            .await
            .unwrap(),
    );
    io.write_all(
        format!(
            "CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n",
            destination.address, destination.address
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    assert!(support::head(&mut io).await.starts_with(b"HTTP/1.1 200"));
    drop(io);
    wait_for(|| engine.status().active_connections == 0).await;
    engine.shutdown().await;
}

#[tokio::test]
async fn global_bandwidth_is_shared_by_simultaneous_connections() {
    let destination = support::echo(false).await;
    let upstream = support::http_proxy(None, None, 200).await;
    let mut config = ProxyConfig {
        inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
        upstreams: vec![support::upstream(upstream.address, ProxyProtocol::Http)],
        ..Default::default()
    };
    config.limits.bytes_per_second = 64 * 1024;
    let engine = Engine::new(config).unwrap();
    engine.start_enabled().await.unwrap();
    let address = engine.status().inbounds[0].listen.clone().unwrap();
    let started = tokio::time::Instant::now();
    let transfer = || async {
        let mut io: BoxIo = Box::new(TcpStream::connect(&address).await.unwrap());
        io.write_all(
            format!(
                "CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n",
                destination.address, destination.address
            )
            .as_bytes(),
        )
        .await
        .unwrap();
        assert!(support::head(&mut io).await.starts_with(b"HTTP/1.1 200"));
        let data = vec![5; 16 * 1024];
        io.write_all(&data).await.unwrap();
        io.shutdown().await.unwrap();
        let mut body = Vec::new();
        io.read_to_end(&mut body).await.unwrap();
        assert_eq!(body, data);
    };
    tokio::join!(transfer(), transfer());
    assert!(started.elapsed() >= Duration::from_millis(900));
    engine.shutdown().await;
}
