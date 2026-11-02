#[path = "../../engine/tests/support/mod.rs"]
mod support;
use hps_application::{StartupOverrides, Supervisor};
use hps_config::{
    DaemonConfig, HealthConfig, ProbeProfile, ProviderConfig, ProviderKind, ProxyConfig,
    ProxyProtocol, RefillMode,
};
use hps_domain::{HealthProbeInput, HealthVerdict, ProbePhase};
use hps_storage::Store;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
async fn until(mut predicate: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !predicate().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
fn config(proxy: &support::Server, origin: &support::Server) -> DaemonConfig {
    let mut config = DaemonConfig {
        proxy: ProxyConfig {
            inbounds: vec![
                support::inbound("http", ProxyProtocol::Http, None),
                support::inbound("socks", ProxyProtocol::Socks5, None),
            ],
            upstreams: vec![support::upstream(proxy.address, ProxyProtocol::Http)],
            ..Default::default()
        },
        health: HealthConfig {
            profiles: vec![ProbeProfile {
                url: format!("http://{}/health", origin.address),
                ..Default::default()
            }],
            timeout_ms: 2000,
            interval_secs: 60,
            max_staleness_secs: 180,
            ..Default::default()
        },
        ..Default::default()
    };
    config.proxy.limits.min_remaining_lifetime_secs = 0;
    config
}
async fn boot(config: DaemonConfig) -> Arc<Supervisor> {
    let app = Supervisor::boot(
        Store::open(None, config).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    app.confirm_startup().await.unwrap();
    app
}

#[tokio::test]
async fn all_inlets_wait_for_real_proof_probe_has_no_demand_and_real_failure_revokes() {
    let target = support::origin(None).await;
    let proxy = support::http_proxy(None, None, 200).await;
    let cert = support::certificate();
    let mut cfg = config(&proxy, &target);
    cfg.proxy
        .inbounds
        .push(support::inbound("https", ProxyProtocol::Https, Some(&cert)));
    let app = boot(cfg).await;
    until(async || app.current().engine.ready()).await;
    assert!(app.current().engine.demand_age().is_none());
    assert!(
        app.health_overview(None, 20).await.unwrap().nodes[0]
            .last_traffic
            .is_none()
    );
    assert!(!app.current().engine.has_active_demand());
    let status = app.current().engine.status();
    for inbound in &status.inbounds {
        let address: std::net::SocketAddr = inbound.listen.as_ref().unwrap().parse().unwrap();
        let url = match inbound.id.as_str() {
            "socks" => format!("socks5h://{address}"),
            "https" => format!("https://localhost:{}", address.port()),
            _ => format!("http://{address}"),
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .add_root_certificate(
                reqwest::Certificate::from_pem(&std::fs::read(&cert.config.certificate).unwrap())
                    .unwrap(),
            )
            .proxy(reqwest::Proxy::all(url).unwrap())
            .build()
            .unwrap();
        let response = client
            .get(format!("http://{}/business", target.address))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
    assert!(app.current().engine.demand_age().is_some());
    proxy.task.abort();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let inbound = status.inbounds.iter().find(|i| i.id == "http").unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(format!("http://{}", inbound.listen.as_ref().unwrap())).unwrap())
        .build()
        .unwrap();
    for _ in 0..2 {
        let response = client
            .get(format!("http://{}/business", target.address))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 502);
    }
    assert!(!app.current().engine.ready());
    until(async || {
        app.health_overview(None, 20).await.unwrap().nodes[0]
            .results
            .iter()
            .any(|r| r.phase == ProbePhase::EndpointConnect && r.verdict == HealthVerdict::Failed)
    })
    .await;
    assert_eq!(
        client
            .get(format!("http://{}/business", target.address))
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn manual_short_lease_reports_skip_and_automatic_probe_spends_nothing() {
    let target = support::origin(None).await;
    let proxy = support::http_proxy(None, None, 200).await;
    let mut cfg = config(&proxy, &target);
    cfg.proxy.upstreams[0].expires_at_unix = Some(hps_storage::unix_now() as u64 + 3);
    let app = boot(cfg).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = app.health_overview(None, 20).await.unwrap();
    assert_eq!(before.used_probes, 0);
    assert!(!before.nodes[0].eligible);
    let task = app
        .probe_health(HealthProbeInput {
            request_id: "short-lease".into(),
            node_ids: vec!["fixture".into()],
        })
        .await
        .unwrap();
    until(async || app.health_task(task.id).await.unwrap().state == "completed").await;
    let task = app.health_task(task.id).await.unwrap();
    assert_eq!(task.skipped, 1);
    assert_eq!(
        task.runs[0].result.as_ref().unwrap().error_code.as_deref(),
        Some("lease_margin_insufficient")
    );
    assert!(proxy.requests.lock().unwrap().is_empty());
    assert_eq!(app.health_overview(None, 20).await.unwrap().used_probes, 1);
    app.shutdown().await.unwrap();
}

async fn gated_origin() -> (
    support::Server,
    Arc<tokio::sync::Semaphore>,
    Arc<AtomicUsize>,
) {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let arrivals = Arc::new(AtomicUsize::new(0));
    let permits = gate.clone();
    let count = arrivals.clone();
    let task = tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let permits = permits.clone();
            let count = count.clone();
            tokio::spawn(async move {
                let mut io: hps_engine::BoxIo = Box::new(tcp);
                let _ = support::head(&mut io).await;
                count.fetch_add(1, Ordering::SeqCst);
                let permit = permits.acquire().await.unwrap();
                permit.forget();
                let _ = io
                    .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                    .await;
            });
        }
    });
    (
        support::Server {
            address,
            task,
            requests: Arc::new(std::sync::Mutex::new(vec![])),
        },
        gate,
        arrivals,
    )
}
#[tokio::test]
async fn old_generation_probe_cannot_promote_changed_credentials() {
    let (target, gate, arrivals) = gated_origin().await;
    let proxy = support::http_proxy(None, None, 200).await;
    let mut cfg = config(&proxy, &target);
    cfg.health.max_parallel = 1;
    let app = boot(cfg.clone()).await;
    until(async || arrivals.load(Ordering::SeqCst) == 1).await;
    let old_identity = app.current().engine.probe_plans()[0].identity().to_owned();
    cfg.proxy.upstreams[0].credentials = Some(hps_config::UpstreamCredentials {
        username: "new".into(),
        password: "secret".into(),
    });
    let rev = app
        .create_revision(hps_domain::ConfigInput {
            toml: toml::to_string(&cfg).unwrap(),
            expected_revision: 1,
            expected_draft_version: None,
            note: "rotate upstream credentials".into(),
            use_draft_credentials: false,
        })
        .await
        .unwrap();
    app.activate(hps_domain::ConfigActivate {
        revision: rev.id,
        expected_revision: 1,
        defer_until_restart: false,
    })
    .await
    .unwrap();
    let new_identity = app.current().engine.probe_plans()[0].identity().to_owned();
    assert_ne!(old_identity, new_identity);
    gate.add_permits(1);
    until(async || arrivals.load(Ordering::SeqCst) >= 2).await;
    assert!(!app.current().engine.ready());
    let overview = app.health_overview(None, 20).await.unwrap();
    assert!(!overview.nodes[0].eligible);
    assert!(
        overview.nodes[0]
            .results
            .iter()
            .all(|r| r.verdict != HealthVerdict::Healthy)
    );
    gate.add_permits(1);
    until(async || app.current().engine.ready()).await;
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn pending_admission_reserves_inventory_and_batch_coalesces_without_repurchase() {
    use axum::{Json, Router, routing::get};
    let (target, gate, arrivals) = gated_origin().await;
    let proxies = [
        support::http_proxy(None, None, 200).await,
        support::http_proxy(None, None, 200).await,
    ];
    let nodes = proxies
        .iter()
        .map(|p| serde_json::json!({"ip":"127.0.0.1","port":p.address.port(),"ttl":120}))
        .collect::<Vec<_>>();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let vendor = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = vendor.local_addr().unwrap();
    let router = Router::new().route(
        "/extract",
        get(move || {
            let nodes = nodes.clone();
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Json(serde_json::json!({"data":{"proxies":nodes}}))
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(vendor, router).await.unwrap();
    });
    let mut cfg = config(&proxies[0], &target);
    cfg.proxy.upstreams.clear();
    cfg.health.timeout_ms = 5000;
    let mut provider = ProviderConfig {
        id: "vendor".into(),
        name: "vendor".into(),
        enabled: true,
        kind: ProviderKind::Json,
        endpoint: format!("http://{address}/extract"),
        count: 2,
        count_parameter: Some("num".into()),
        min_remaining_secs: 0,
        min_interval_secs: 1,
        ..Default::default()
    };
    provider.refill.enabled = true;
    provider.refill.mode = RefillMode::Maintain;
    provider.refill.min_nodes = 1;
    provider.refill.target_nodes = 2;
    provider.refill.max_nodes = 2;
    provider.refill.batch_size = 2;
    provider.refill.retire_before_secs = 0;
    provider.refill.forecast_secs = 0;
    cfg.providers = vec![provider];
    let app = boot(cfg).await;
    until(async || arrivals.load(Ordering::SeqCst) == 2).await;
    let controller = app.provider_controller("vendor").await.unwrap();
    assert_eq!(controller.current_inventory, 0);
    assert_eq!(controller.pending_health_nodes, 2);
    assert_eq!(controller.used_requests, 1);
    let before = app.health_overview(None, 20).await.unwrap();
    let task = app
        .probe_health(HealthProbeInput {
            request_id: "join-batch".into(),
            node_ids: vec![],
        })
        .await
        .unwrap();
    assert_eq!(task.merged, 2);
    assert_eq!(
        app.health_overview(None, 20).await.unwrap().used_probes,
        before.used_probes
    );
    for _ in 0..3 {
        let evaluation = app.evaluate_controller("vendor".into()).await.unwrap();
        assert!(evaluation.task.is_none());
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(app.current().engine.demand_age().is_none());
    gate.add_permits(2);
    until(async || {
        app.provider_controller("vendor")
            .await
            .unwrap()
            .current_inventory
            == 2
    })
    .await;
    assert_eq!(app.health_task(task.id).await.unwrap().healthy, 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    app.shutdown().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn real_target_rejection_is_visible_without_invalidating_health_or_replaying_payload() {
    use axum::{Router, http::StatusCode, routing::get};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let router = Router::new()
        .route("/health", get(|| async { StatusCode::NO_CONTENT }))
        .route(
            "/business",
            get(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    StatusCode::TOO_MANY_REQUESTS
                }
            }),
        );
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let target = support::Server {
        address,
        task,
        requests: Default::default(),
    };
    let proxy = support::http_proxy(None, None, 200).await;
    let app = boot(config(&proxy, &target)).await;
    until(async || app.current().engine.ready()).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(
            reqwest::Proxy::all(format!(
                "http://{}",
                app.current().engine.status().inbounds[0]
                    .listen
                    .as_ref()
                    .unwrap()
            ))
            .unwrap(),
        )
        .build()
        .unwrap();
    let before = app.health_overview(None, 20).await.unwrap().nodes[0].results[0].checked_at;
    let response = client
        .get(format!("http://{address}/business"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 429);
    assert!(app.current().engine.ready());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let node = app.health_overview(None, 20).await.unwrap().nodes.remove(0);
    let traffic = node.last_traffic.unwrap();
    assert_eq!(traffic.error_code, "target_rejected");
    assert!(!traffic.invalidated);
    assert_eq!(node.results[0].checked_at, before);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn rotating_ca_contents_at_same_path_cannot_reuse_old_prepared_roots_or_health() {
    let cert_a = support::certificate();
    let cert_b = support::certificate();
    let target = support::origin(None).await;
    let proxy = support::http_proxy(Some(&cert_a), None, 200).await;
    let directory = tempfile::tempdir().unwrap();
    let trust = directory.path().join("trust.pem");
    std::fs::copy(&cert_a.config.certificate, &trust).unwrap();
    let mut cfg = config(&proxy, &target);
    cfg.proxy.upstreams[0].protocol = ProxyProtocol::Https;
    cfg.proxy.upstreams[0].tls_server_name = Some("localhost".into());
    cfg.proxy.upstreams[0].ca_certificate = Some(trust.clone());
    let app = boot(cfg.clone()).await;
    until(async || app.current().engine.ready()).await;
    let before = app.current().engine.clone();
    let identity = before.probe_plans()[0].identity().to_owned();
    std::fs::copy(&cert_b.config.certificate, &trust).unwrap();
    let revision = app
        .create_revision(hps_domain::ConfigInput {
            toml: toml::to_string(&cfg).unwrap(),
            expected_revision: 1,
            expected_draft_version: None,
            note: "rotate CA contents".into(),
            use_draft_credentials: false,
        })
        .await
        .unwrap();
    app.activate(hps_domain::ConfigActivate {
        revision: revision.id,
        expected_revision: 1,
        defer_until_restart: false,
    })
    .await
    .unwrap();
    assert!(!Arc::ptr_eq(&before, &app.current().engine));
    assert_ne!(identity, app.current().engine.probe_plans()[0].identity());
    assert!(!app.current().engine.ready());
    until(async || {
        app.health_overview(None, 20).await.unwrap().nodes[0]
            .results
            .iter()
            .any(|r| r.phase == ProbePhase::UpstreamTls && r.verdict == HealthVerdict::Failed)
    })
    .await;
    app.shutdown().await.unwrap();
}
