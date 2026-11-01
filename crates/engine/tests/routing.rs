mod support;
use hps_config::{
    GroupConfig, PortRange, ProxyConfig, ProxyProtocol, RoutingConfig, RuleConditions, RuleConfig,
    SelectionConfig,
};
use hps_domain::RouteAction;
use hps_engine::runtime::Engine;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

fn port_rule(id: &str, ports: Vec<u16>, action: RouteAction) -> RuleConfig {
    RuleConfig {
        id: id.into(),
        enabled: true,
        conditions: RuleConditions {
            destination_ports: ports
                .into_iter()
                .map(|p| PortRange { start: p, end: p })
                .collect(),
            source_cidrs: vec!["127.0.0.0/8".into()],
            ..Default::default()
        },
        action,
    }
}
#[tokio::test]
async fn real_three_inlets_share_first_match_node_group_direct_reject_and_do_not_buy_on_direct_or_reject()
 {
    let cert = support::certificate();
    let direct = support::origin(None).await;
    let tls = support::origin(Some(&cert)).await;
    let via_a = support::origin(None).await;
    let via_b = support::origin(None).await;
    let a = support::http_proxy(None, None, 200).await;
    let b = support::http_proxy(None, None, 200).await;
    let mut up_a = support::upstream(a.address, ProxyProtocol::Http);
    up_a.id = "a".into();
    let mut up_b = support::upstream(b.address, ProxyProtocol::Http);
    up_b.id = "b".into();
    let engine = Engine::new(ProxyConfig {
        upstreams: vec![up_a, up_b],
        inbounds: vec![
            support::inbound("http", ProxyProtocol::Http, None),
            support::inbound("https", ProxyProtocol::Https, Some(&cert)),
            support::inbound("socks", ProxyProtocol::Socks5, None),
        ],
        selection: SelectionConfig {
            default_group: Some("empty".into()),
            groups: vec![
                GroupConfig {
                    id: "empty".into(),
                    ..Default::default()
                },
                GroupConfig {
                    id: "b-group".into(),
                    members: vec!["b".into()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        routing: RoutingConfig {
            default_action: RouteAction::Reject,
            rules: vec![
                port_rule(
                    "direct",
                    vec![direct.address.port(), tls.address.port()],
                    RouteAction::Direct,
                ),
                port_rule(
                    "node",
                    vec![via_a.address.port()],
                    RouteAction::Node { id: "a".into() },
                ),
                port_rule(
                    "group",
                    vec![via_b.address.port()],
                    RouteAction::Group {
                        id: "b-group".into(),
                    },
                ),
            ],
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    engine.set_generation(42);
    engine.start_enabled().await.unwrap();
    assert!(engine.ready());
    let mut clients = vec![];
    for inlet in engine.status().inbounds {
        let address: std::net::SocketAddr = inlet.listen.unwrap().parse().unwrap();
        let proxy = match inlet.protocol.as_str() {
            "https" => format!("https://localhost:{}", address.port()),
            "socks5" => format!("socks5h://{address}"),
            _ => format!("http://{address}"),
        };
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(proxy).unwrap())
            .add_root_certificate(
                reqwest::Certificate::from_pem(&std::fs::read(&cert.config.certificate).unwrap())
                    .unwrap(),
            )
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        for url in [
            format!("http://{}/echo", direct.address),
            format!("https://localhost:{}/echo", tls.address.port()),
        ] {
            assert_eq!(
                client
                    .post(url)
                    .body("direct-bytes")
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
                "direct-bytes"
            );
        }
        let denied = client.get("http://127.0.0.1:9/denied").send().await;
        if inlet.protocol == "socks5" {
            assert!(denied.is_err());
        } else {
            assert_eq!(denied.unwrap().status(), 403);
        }
        clients.push(client);
    }
    assert!(engine.demand_age().is_none());
    assert!(!engine.has_active_demand());
    assert!(a.requests.lock().unwrap().is_empty());
    assert!(b.requests.lock().unwrap().is_empty());
    for client in clients {
        for origin in [&via_a, &via_b] {
            assert_eq!(
                client
                    .post(format!("http://{}/echo", origin.address))
                    .body("selected-bytes")
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
                "selected-bytes"
            );
        }
    }
    assert_eq!(a.requests.lock().unwrap().len(), 3);
    assert_eq!(b.requests.lock().unwrap().len(), 3);
    assert!(engine.demand_age().is_some());
    engine.shutdown().await;
}

#[tokio::test]
async fn explicit_node_cannot_escape_to_a_healthy_unrelated_node_and_catchall_reject_masks_direct_default()
 {
    let a = support::http_proxy(None, None, 200).await;
    let b = support::http_proxy(None, None, 200).await;
    let mut up_a = support::upstream(a.address, ProxyProtocol::Http);
    up_a.id = "a".into();
    up_a.enabled = false;
    let mut up_b = support::upstream(b.address, ProxyProtocol::Http);
    up_b.id = "b".into();
    let mut config = ProxyConfig {
        upstreams: vec![up_a, up_b],
        inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
        routing: RoutingConfig {
            default_action: RouteAction::Node { id: "a".into() },
            ..Default::default()
        },
        ..Default::default()
    };
    let engine = Engine::new(config.clone()).unwrap();
    engine.start_enabled().await.unwrap();
    assert!(!engine.ready());
    let client = reqwest::Client::builder()
        .proxy(
            reqwest::Proxy::all(format!(
                "http://{}",
                engine.status().inbounds[0].listen.as_ref().unwrap()
            ))
            .unwrap(),
        )
        .build()
        .unwrap();
    assert_eq!(
        client
            .get("http://127.0.0.1:9/")
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    assert!(b.requests.lock().unwrap().is_empty());
    engine.shutdown().await;
    config.routing.default_action = RouteAction::Direct;
    config.routing.rules = vec![RuleConfig {
        id: "deny-all".into(),
        enabled: true,
        conditions: RuleConditions::default(),
        action: RouteAction::Reject,
    }];
    let engine = Engine::new(config).unwrap();
    engine.start_enabled().await.unwrap();
    assert!(!engine.ready());
    engine.shutdown().await;
}

struct LocalResolver(Arc<Mutex<Vec<hps_net::ResolvePurpose>>>);
impl hps_net::NameResolver for LocalResolver {
    fn lookup(
        &self,
        _host: String,
        purpose: hps_net::ResolvePurpose,
    ) -> hps_net::NetworkFuture<'_, Vec<std::net::IpAddr>> {
        self.0.lock().unwrap().push(purpose);
        Box::pin(async { Ok(vec!["127.0.0.1".parse().unwrap()]) })
    }
}
#[tokio::test]
async fn direct_uses_shared_business_dns_and_keeps_http_host_without_pool_demand() {
    let origin = support::origin(None).await;
    let seen = Arc::new(Mutex::new(vec![]));
    let engine = Engine::new(ProxyConfig {
        inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
        routing: RoutingConfig {
            default_action: RouteAction::Direct,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    engine.configure_resolver(Some(Arc::new(LocalResolver(seen.clone()))));
    engine.start_enabled().await.unwrap();
    assert!(engine.ready());
    let client = reqwest::Client::builder()
        .proxy(
            reqwest::Proxy::all(format!(
                "http://{}",
                engine.status().inbounds[0].listen.as_ref().unwrap()
            ))
            .unwrap(),
        )
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    assert_eq!(
        client
            .post(format!(
                "http://does-not-resolve.invalid:{}/echo",
                origin.address.port()
            ))
            .body("host-proof")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "host-proof"
    );
    assert_eq!(
        *seen.lock().unwrap(),
        vec![hps_net::ResolvePurpose::Business]
    );
    assert!(engine.demand_age().is_none());
    {
        let requests = origin.requests.lock().unwrap();
        assert!(
            String::from_utf8_lossy(&requests[0])
                .to_ascii_lowercase()
                .contains(&format!(
                    "\"host\": \"does-not-resolve.invalid:{}\"",
                    origin.address.port()
                ))
        );
    }
    engine.shutdown().await;
}
