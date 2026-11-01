#[path = "../../engine/tests/support/mod.rs"]
mod support;
use hickory_proto::{
    op::{Message, OpCode},
    rr::{RData, Record, rdata::A},
};
use hps_application::{StartupOverrides, Supervisor};
use hps_config::{DaemonConfig, DnsProtocol, ProxyProtocol, UpstreamCredentials};
use hps_dns::{DnsError, LookupInput};
use hps_net::ResolvePurpose;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn dns_uses_all_three_authenticated_static_egresses_without_bootstrap_recursion_or_direct_fallback()
 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let target = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let len = stream.read_u16().await.unwrap();
                let mut data = vec![0; len as usize];
                stream.read_exact(&mut data).await.unwrap();
                let query = Message::from_vec(&data).unwrap();
                let mut response = Message::response(query.metadata.id, OpCode::Query);
                response.answers.push(Record::from_rdata(
                    query.queries[0].name().clone(),
                    30,
                    RData::A(A::new(203, 0, 113, 9)),
                ));
                response.queries = query.queries;
                let data = response.to_vec().unwrap();
                stream.write_u16(data.len() as u16).await.unwrap();
                stream.write_all(&data).await.unwrap();
            });
        }
    });
    let certificate = support::certificate();
    for protocol in [
        ProxyProtocol::Http,
        ProxyProtocol::Https,
        ProxyProtocol::Socks5,
    ] {
        let proxy = if protocol == ProxyProtocol::Socks5 {
            support::socks_proxy(true).await
        } else {
            support::http_proxy(
                (protocol == ProxyProtocol::Https).then_some(&certificate),
                Some("Basic c3VwcGxpZXI6c2VjcmV0"),
                200,
            )
            .await
        };
        let mut node = support::upstream(proxy.address, protocol);
        node.host = "bootstrap.invalid".into();
        node.credentials = Some(UpstreamCredentials {
            username: "supplier".into(),
            password: "secret".into(),
        });
        if protocol == ProxyProtocol::Https {
            node.ca_certificate = Some(certificate.config.certificate.clone());
            node.tls_server_name = Some("localhost".into());
        }
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        config.proxy.upstreams = vec![node];
        let up = &mut config.dns.upstreams[0];
        up.protocol = DnsProtocol::Tcp;
        up.addresses = vec![address];
        up.proxy_id = Some("fixture".into());
        up.proxy_bootstrap_ips = vec!["127.0.0.1".parse().unwrap()];
        let service = Supervisor::boot(
            hps_storage::Store::open(None, config).unwrap(),
            StartupOverrides::default(),
        )
        .await
        .unwrap();
        service.confirm_startup().await.unwrap();
        let input = LookupInput {
            name: "proxied.example".into(),
            query_type: "A".into(),
            purpose: ResolvePurpose::Business,
        };
        let result = service.current().dns.diagnose(input.clone()).await.unwrap();
        assert_eq!(result.answers[0].value, "203.0.113.9");
        assert_eq!(result.egress.as_deref(), Some("fixture"));
        assert!(!service.current().engine.has_active_demand());
        assert!(service.providers().await.unwrap().is_empty());
        proxy.task.abort();
        tokio::task::yield_now().await;
        service.current().dns.clear(None).unwrap();
        assert_eq!(
            service.current().dns.diagnose(input).await.unwrap_err(),
            DnsError::Egress
        );
        service.shutdown().await.unwrap();
    }
    target.abort();
}
