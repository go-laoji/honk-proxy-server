use bytes::Bytes;
use hickory_proto::{
    op::{Message, OpCode},
    rr::{RData, Record, rdata::A},
};
use hps_config::{DnsConfig, DnsProtocol, ProxyConfig, ServerTls};
use hps_dns::{DnsError, LookupInput, Resolver};
use hps_net::ResolvePurpose;
use http_body_util::{BodyExt, Full};
use std::convert::Infallible;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::TlsAcceptor;

fn response(query: Message) -> Message {
    let mut response = Message::response(query.metadata.id, OpCode::Query);
    response.answers.push(Record::from_rdata(
        query.queries[0].name().clone(),
        30,
        RData::A(A::new(203, 0, 113, 42)),
    ));
    response.queries = query.queries;
    response
}
#[tokio::test]
async fn dot_and_doh_verify_real_tls_and_never_accept_a_redirect_or_wrong_type() {
    let directory = tempfile::tempdir().unwrap();
    let cert = rcgen::generate_simple_self_signed(vec!["dns.test".into()]).unwrap();
    let ca = directory.path().join("ca.pem");
    let key = directory.path().join("key.pem");
    std::fs::write(&ca, cert.cert.pem()).unwrap();
    std::fs::write(&key, cert.signing_key.serialize_pem()).unwrap();
    let server = hps_engine::tls::prepare_server(&ServerTls {
        certificate: ca.clone(),
        private_key: key,
    })
    .unwrap();
    let acceptor = TlsAcceptor::from(server.config);
    for protocol in [DnsProtocol::Tls, DnsProtocol::Https] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let acceptor = acceptor.clone();
        let task = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(mut io) = acceptor.accept(stream).await else {
                        return;
                    };
                    if protocol == DnsProtocol::Tls {
                        let len = io.read_u16().await.unwrap();
                        let mut data = vec![0; len as usize];
                        io.read_exact(&mut data).await.unwrap();
                        let data = response(Message::from_vec(&data).unwrap())
                            .to_vec()
                            .unwrap();
                        io.write_u16(data.len() as u16).await.unwrap();
                        io.write_all(&data).await.unwrap();
                    } else {
                        let service = hyper::service::service_fn(
                            |request: hyper::Request<hyper::body::Incoming>| async move {
                                assert_eq!(request.method(), "POST");
                                assert_eq!(
                                    request.headers()["content-type"],
                                    "application/dns-message"
                                );
                                let path = request.uri().path().to_owned();
                                let data = request.into_body().collect().await.unwrap().to_bytes();
                                let query = Message::from_vec(&data).unwrap();
                                let mut reply = hyper::Response::builder();
                                if path == "/redirect" {
                                    reply = reply
                                        .status(302)
                                        .header("location", "https://elsewhere.test/dns-query");
                                }
                                let reply = reply
                                    .header(
                                        "content-type",
                                        if path == "/wrong" {
                                            "text/plain"
                                        } else {
                                            "application/dns-message"
                                        },
                                    )
                                    .body(Full::new(Bytes::from(response(query).to_vec().unwrap())))
                                    .unwrap();
                                Ok::<_, Infallible>(reply)
                            },
                        );
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(hyper_util::rt::TokioIo::new(io), service)
                            .await;
                    }
                });
            }
        });
        let mut config = DnsConfig::default();
        let up = &mut config.upstreams[0];
        up.protocol = protocol;
        up.addresses = vec![address];
        up.server_name = Some("dns.test".into());
        up.ca_certificate = Some(ca.clone());
        let resolver = Resolver::prepare(config.clone(), &ProxyConfig::default(), None).unwrap();
        let result = resolver
            .diagnose(LookupInput {
                name: "encrypted.example".into(),
                query_type: "A".into(),
                purpose: ResolvePurpose::Bootstrap,
            })
            .await
            .unwrap();
        assert_eq!(result.answers[0].value, "203.0.113.42");
        assert!(!result.dnssec_validated_locally);
        config.upstreams[0].server_name = Some("wrong.test".into());
        let wrong = Resolver::prepare(config.clone(), &ProxyConfig::default(), None).unwrap();
        assert_eq!(
            wrong
                .diagnose(LookupInput {
                    name: "encrypted.example".into(),
                    query_type: "A".into(),
                    purpose: ResolvePurpose::Bootstrap
                })
                .await
                .unwrap_err(),
            DnsError::Tls
        );
        assert_eq!(wrong.cache(None, 50).unwrap().total, 0);
        if protocol == DnsProtocol::Https {
            config.upstreams[0].server_name = Some("dns.test".into());
            for path in ["/redirect", "/wrong"] {
                config.upstreams[0].https_path = Some(path.into());
                let wrong =
                    Resolver::prepare(config.clone(), &ProxyConfig::default(), None).unwrap();
                assert_eq!(
                    wrong
                        .diagnose(LookupInput {
                            name: "encrypted.example".into(),
                            query_type: "A".into(),
                            purpose: ResolvePurpose::Bootstrap
                        })
                        .await
                        .unwrap_err(),
                    DnsError::Http
                );
            }
        }
        task.abort();
    }
    let mut config = DnsConfig::default();
    let up = &mut config.upstreams[0];
    up.protocol = DnsProtocol::Tls;
    up.addresses = vec!["127.0.0.1:853".parse().unwrap()];
    up.server_name = Some("dns.test".into());
    up.ca_certificate = Some(ca.clone());
    let before = Resolver::prepare(config.clone(), &ProxyConfig::default(), None).unwrap();
    let cert = rcgen::generate_simple_self_signed(vec!["dns.test".into()]).unwrap();
    std::fs::write(&ca, cert.cert.pem()).unwrap();
    let after = Resolver::prepare(config, &ProxyConfig::default(), None).unwrap();
    assert_ne!(before.fingerprint(), after.fingerprint());
}
