mod support;
use hps_config::{GroupConfig, GroupStrategy, ProxyConfig, ProxyProtocol, SelectionConfig};
use hps_domain::SelectionQuery;
use hps_engine::{dial::Dialer, runtime::Engine, target::Target};
use std::{collections::BTreeMap, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn all_explicit_inlets_use_the_bound_group_and_do_not_escape_an_empty_default() {
    let cert = support::certificate();
    let origin = support::origin(None).await;
    let a = support::http_proxy(None, None, 200).await;
    let b = support::http_proxy(None, None, 200).await;
    let mut a_config = support::upstream(a.address, ProxyProtocol::Http);
    a_config.id = "a".into();
    let mut b_config = support::upstream(b.address, ProxyProtocol::Http);
    b_config.id = "b".into();
    let engine = Engine::new(ProxyConfig {
        upstreams: vec![a_config, b_config],
        inbounds: vec![
            support::inbound("http", ProxyProtocol::Http, None),
            support::inbound("https", ProxyProtocol::Https, Some(&cert)),
            support::inbound("socks", ProxyProtocol::Socks5, None),
        ],
        selection: SelectionConfig {
            groups: vec![
                GroupConfig {
                    id: "empty".into(),
                    ..Default::default()
                },
                GroupConfig {
                    id: "pinned".into(),
                    strategy: GroupStrategy::Fixed,
                    include_all: true,
                    pinned_node: Some("b".into()),
                    ..Default::default()
                },
            ],
            default_group: Some("empty".into()),
            inbound_groups: ["http", "https", "socks"]
                .into_iter()
                .map(|id| (id.into(), "pinned".into()))
                .collect::<BTreeMap<_, _>>(),
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    engine.start_enabled().await.unwrap();
    assert!(engine.ready());
    assert!(
        engine
            .selection(&SelectionQuery::default())
            .selected
            .is_none()
    );
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
        assert_eq!(
            client
                .post(format!("http://{}/echo", origin.address))
                .body("group-selected")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "group-selected"
        );
    }
    assert!(a.requests.lock().unwrap().is_empty());
    assert_eq!(b.requests.lock().unwrap().len(), 3);
    assert_eq!(
        engine
            .group_last_choices()
            .get("pinned")
            .map(String::as_str),
        Some("b")
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn real_round_robin_preview_does_not_advance_and_failed_fixed_handshake_never_retries() {
    let target = support::echo(false).await;
    let target = Target::new("127.0.0.1", target.address.port()).unwrap();
    let a = support::http_proxy(None, None, 200).await;
    let b = support::http_proxy(None, None, 200).await;
    let mut a_config = support::upstream(a.address, ProxyProtocol::Http);
    a_config.id = "a".into();
    let mut b_config = support::upstream(b.address, ProxyProtocol::Http);
    b_config.id = "b".into();
    let dialer = Dialer::new(&ProxyConfig {
        upstreams: vec![a_config, b_config.clone()],
        selection: SelectionConfig {
            default_group: Some("g".into()),
            groups: vec![GroupConfig {
                id: "g".into(),
                include_all: true,
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    for expected in ["a", "b", "a", "b"] {
        for _ in 0..2 {
            assert_eq!(
                dialer
                    .selection(&SelectionQuery::default())
                    .selected
                    .as_deref(),
                Some(expected)
            );
        }
        let mut c = dialer.connect(&target, false).await.unwrap();
        assert_eq!(c.upstream_id, expected);
        c.io.write_all(b"proof").await.unwrap();
        c.io.shutdown().await.unwrap();
        let mut data = vec![];
        c.io.read_to_end(&mut data).await.unwrap();
        assert_eq!(data, b"proof");
    }
    assert_eq!(a.requests.lock().unwrap().len(), 2);
    assert_eq!(b.requests.lock().unwrap().len(), 2);
    let rejected = support::http_proxy(None, None, 407).await;
    let mut bad = support::upstream(rejected.address, ProxyProtocol::Http);
    bad.id = "bad".into();
    let dialer = Dialer::new(&ProxyConfig {
        upstreams: vec![bad, b_config],
        selection: SelectionConfig {
            default_group: Some("fixed".into()),
            groups: vec![GroupConfig {
                id: "fixed".into(),
                include_all: true,
                strategy: GroupStrategy::Fixed,
                pinned_node: Some("bad".into()),
                fallback_nodes: vec!["b".into()],
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    assert!(matches!(
        dialer.connect(&target, false).await,
        Err(hps_engine::ProxyError::Authentication)
    ));
    assert_eq!(rejected.requests.lock().unwrap().len(), 1);
    assert_eq!(b.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn dynamic_lease_identity_minimum_lifetime_provider_and_udp_are_rechecked() {
    let proxy = support::http_proxy(None, None, 200).await;
    let mut node = support::upstream(proxy.address, ProxyProtocol::Http);
    node.id = "lease:42".into();
    let config = ProxyConfig {
        selection: SelectionConfig {
            default_group: Some("vendor".into()),
            groups: vec![GroupConfig {
                id: "vendor".into(),
                providers: vec!["source".into()],
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    let dialer = Dialer::new(&config).unwrap();
    // Public engine pool is shared with application generations.
    let engine = Engine::new(config).unwrap();
    let pool = engine.dynamic_pool();
    let now = pool.logical_now();
    node.expires_at_unix = Some(now + 60);
    let lease = |node| hps_engine::pool::PoolNode {
        provider_id: "source".into(),
        fingerprint: "v1".into(),
        upstream: node,
        min_remaining_secs: 10,
    };
    pool.replace(vec![lease(node.clone())]).unwrap();
    assert_eq!(engine.selection(&SelectionQuery::default()).eligible, 0);
    engine.set_provider_sources(std::collections::HashMap::from([(
        "source".into(),
        "v1".into(),
    )]));
    assert_eq!(
        engine
            .selection(&SelectionQuery::default())
            .selected
            .as_deref(),
        Some("lease:42")
    );
    let udp = engine.selection(&SelectionQuery {
        transport: hps_domain::SelectionTransport::Udp,
        ..Default::default()
    });
    assert_eq!(udp.eligible, 0);
    assert!(udp.candidates[0].reasons.contains(&"tcp_only".into()));
    node.expires_at_unix = Some(now + 2);
    pool.replace(vec![lease(node)]).unwrap();
    assert!(
        engine.selection(&SelectionQuery::default()).candidates[0]
            .reasons
            .contains(&"insufficient_lifetime".into())
    );
    assert_eq!(dialer.eligible_count(), 0);
    assert!(proxy.requests.lock().unwrap().is_empty());
}
