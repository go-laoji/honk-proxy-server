#[path = "../../engine/tests/support/mod.rs"]
mod support;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::Uri,
    routing::get,
};
use hps_application::{StartupOverrides, Supervisor};
use hps_config::DaemonConfig;
use hps_domain::{ExtractionInput, LeaseInfo};
use hps_storage::Store;
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const SOURCE: &str = "kuaidaili-mock";

struct Vendor {
    endpoints: Vec<SocketAddr>,
    extractions: Mutex<Vec<SocketAddr>>,
    lookups: Mutex<Vec<String>>,
}

async fn emulate(
    State(vendor): State<Arc<Vendor>>,
    uri: Uri,
    Query(mut query): Query<BTreeMap<String, String>>,
) -> Json<serde_json::Value> {
    let signature = query.remove("signature").unwrap();
    assert_eq!(
        signature,
        hps_provider::sign("GET", uri.path(), &query, "mock-key")
    );
    assert_eq!(query["secret_id"], "mock-order");
    assert_eq!(query["sign_type"], "hmacsha1");
    match uri.path() {
        "/api/getdps" => {
            assert_eq!(query["format"], "json");
            assert_eq!(query["num"], "1");
            let mut calls = vendor.extractions.lock().unwrap();
            let endpoint = vendor.endpoints[calls.len()];
            calls.push(endpoint);
            Json(serde_json::json!({
                "code": 0,
                "msg": "",
                "data": {
                    "count": 1,
                    "order_left_count": 50,
                    "proxy_list": [endpoint.to_string()]
                }
            }))
        }
        "/api/getdpsvalidtime" => {
            let proxy = &query["proxy"];
            assert_eq!(
                vendor
                    .extractions
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .to_string(),
                *proxy
            );
            vendor.lookups.lock().unwrap().push(proxy.clone());
            Json(serde_json::json!({"code": 0, "data": { (proxy): 120 }}))
        }
        _ => unreachable!(),
    }
}

struct Fixture {
    app: Arc<Supervisor>,
    vendor: Arc<Vendor>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn boot(endpoints: Vec<SocketAddr>, edit: impl FnOnce(&mut DaemonConfig)) -> Self {
        let vendor = Arc::new(Vendor {
            endpoints,
            extractions: Default::default(),
            lookups: Default::default(),
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config =
            DaemonConfig::from_toml(include_str!("../../../configs/hps.kuaidaili.mock.toml"))
                .unwrap();
        config.providers[0].endpoint =
            format!("http://{}/api/getdps", listener.local_addr().unwrap());
        edit(&mut config);
        let router = Router::new()
            .route("/api/getdps", get(emulate))
            .route("/api/getdpsvalidtime", get(emulate))
            .with_state(vendor.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let app = Supervisor::boot(
            Store::open(None, config).unwrap(),
            StartupOverrides::default(),
        )
        .await
        .unwrap();
        app.confirm_startup().await.unwrap();
        let result = Self {
            app,
            vendor,
            server,
        };
        let task = result
            .app
            .extract(
                SOURCE.into(),
                ExtractionInput {
                    request_id: "seed".into(),
                },
            )
            .await
            .unwrap();
        until(|| async { result.app.current().engine.ready() }).await;
        let task = result.app.extraction_task(task.id).await.unwrap();
        assert_eq!(task.state, "succeeded");
        assert_eq!(task.added, 1);
        assert_eq!(task.quota, Some(50));
        let leases = result.leases().await;
        assert_eq!(leases.len(), 1);
        assert_eq!(leases[0].state, "active");
        assert_eq!(leases[0].host, result.vendor.endpoints[0].ip().to_string());
        assert_eq!(leases[0].port, result.vendor.endpoints[0].port());
        assert_eq!(
            leases[0].expiry_basis,
            "vendor_remaining_seconds_from_lookup_start"
        );
        assert_eq!(result.vendor.lookups.lock().unwrap().len(), 1);
        result
    }

    fn address(&self) -> String {
        self.app.current().engine.status().inbounds[0]
            .listen
            .clone()
            .unwrap()
    }

    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(format!("http://{}", self.address())).unwrap())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(6))
            .build()
            .unwrap()
    }

    async fn leases(&self) -> Vec<LeaseInfo> {
        self.app.provider_detail(SOURCE).await.unwrap().leases
    }

    async fn assert_replaced(&self, old: SocketAddr, new: SocketAddr) {
        let detail = self.app.provider_detail(SOURCE).await.unwrap();
        assert_eq!(detail.provider.active_leases, 1);
        assert_eq!(detail.leases.len(), 2);
        assert_eq!(detail.leases[1].state, "revoked");
        assert_eq!(detail.leases[1].port, old.port());
        assert_eq!(detail.leases[0].state, "active");
        assert_eq!(detail.leases[0].port, new.port());
        assert_eq!(detail.tasks[0].trigger, "automatic");
        assert_eq!(
            self.vendor.extractions.lock().unwrap().as_slice(),
            [old, new]
        );
    }
}

async fn until(mut condition: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn forward_failure_retires_lease_and_retries_get_on_a_replacement() {
    let old = support::http_proxy(None, None, 516).await;
    let replacement = support::http_proxy(None, None, 200).await;
    let origin = support::origin(None).await;
    let fixture = Fixture::boot(vec![old.address, replacement.address], |_| {}).await;

    let response = fixture
        .client()
        .get(format!("http://{}/get", origin.address))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.text().await.unwrap(), "origin");
    assert_eq!(old.requests.lock().unwrap().len(), 1);
    assert_eq!(replacement.requests.lock().unwrap().len(), 1);
    assert_eq!(origin.requests.lock().unwrap().len(), 1);
    fixture
        .assert_replaced(old.address, replacement.address)
        .await;
    fixture.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn expired_proxy_is_replaced_before_accepting_connect_tunnel() {
    let old = support::http_proxy(None, None, 454).await;
    let replacement = support::http_proxy(None, None, 200).await;
    let target = support::echo(false).await;
    let fixture = Fixture::boot(vec![old.address, replacement.address], |_| {}).await;

    assert_connect_tunnel(&fixture, target.address).await;
    fixture
        .assert_replaced(old.address, replacement.address)
        .await;
    fixture.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_failed_probe_keeps_the_lease_eligible_until_a_real_connect_fails() {
    let old = support::http_proxy(None, None, 407).await;
    let replacement = support::http_proxy(None, None, 200).await;
    let probe_target = support::origin(None).await;
    let target = support::echo(false).await;
    let fixture = Fixture::boot(vec![old.address, replacement.address], |config| {
        config.health = hps_config::HealthConfig {
            profiles: vec![hps_config::ProbeProfile {
                url: format!("http://{}/health", probe_target.address),
                ..Default::default()
            }],
            ..Default::default()
        };
    })
    .await;
    let lease = fixture.leases().await.remove(0);
    until(async || {
        fixture
            .app
            .node(lease.upstream_id.clone())
            .await
            .unwrap()
            .nodes[0]
            .health_results
            .iter()
            .any(|result| result.error_code.as_deref() == Some("upstream_authentication_failed"))
    })
    .await;
    let node = fixture
        .app
        .node(lease.upstream_id.clone())
        .await
        .unwrap()
        .nodes
        .remove(0);
    assert_eq!(node.health_state, "unhealthy");
    assert!(node.published && node.eligible);
    let health = fixture.app.health_overview(None, 20).await.unwrap();
    assert!(
        health
            .nodes
            .iter()
            .any(|node| node.node_id == lease.upstream_id && node.eligible)
    );
    let control = fixture.app.provider_controller(SOURCE).await.unwrap();
    assert_eq!(control.current_inventory, 1);
    assert_eq!(control.decision, "idle");
    assert_eq!(control.used_requests, 1);

    assert_connect_tunnel(&fixture, target.address).await;
    let received_client_target = |proxy: &support::Server| {
        proxy.requests.lock().unwrap().iter().any(|request| {
            request.starts_with(format!("CONNECT {} HTTP/1.1", target.address).as_bytes())
        })
    };
    assert!(received_client_target(&old));
    assert!(received_client_target(&replacement));
    fixture
        .assert_replaced(old.address, replacement.address)
        .await;
    fixture.app.shutdown().await.unwrap();
}

async fn assert_connect_tunnel(fixture: &Fixture, target: SocketAddr) {
    tokio::time::timeout(Duration::from_secs(6), async {
        let mut tunnel: hps_engine::BoxIo =
            Box::new(TcpStream::connect(fixture.address()).await.unwrap());
        tunnel
            .write_all(
                format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n", target, target).as_bytes(),
            )
            .await
            .unwrap();
        assert!(
            support::head(&mut tunnel)
                .await
                .starts_with(b"HTTP/1.1 200 ")
        );
        tunnel.write_all(b"tunnel-data").await.unwrap();
        tunnel.shutdown().await.unwrap();
        let mut response = Vec::new();
        tunnel.read_to_end(&mut response).await.unwrap();
        assert_eq!(response, b"tunnel-data");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn target_http_failure_keeps_the_lease_and_is_not_retried() {
    let upstream = support::http_proxy(None, None, 200).await;
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = origin.local_addr().unwrap();
    let fixture = Fixture::boot(vec![upstream.address], |_| {}).await;
    let server = tokio::spawn(async move {
        let (stream, _) = origin.accept().await.unwrap();
        let mut stream: hps_engine::BoxIo = Box::new(stream);
        support::head(&mut stream).await;
        stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndown").await.unwrap();
    });

    let response = fixture
        .client()
        .get(format!("http://{target}/outage"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 503);
    assert_eq!(response.text().await.unwrap(), "down");
    server.await.unwrap();
    assert_eq!(upstream.requests.lock().unwrap().len(), 1);
    assert_eq!(fixture.leases().await[0].state, "active");
    assert_eq!(fixture.vendor.extractions.lock().unwrap().len(), 1);
    fixture.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_post_retires_lease_and_refills_without_replaying_the_request() {
    let old = support::http_proxy(None, None, 517).await;
    let replacement = support::http_proxy(None, None, 200).await;
    let origin = support::origin(None).await;
    let fixture = Fixture::boot(vec![old.address, replacement.address], |_| {}).await;

    let response = fixture
        .client()
        .post(format!("http://{}/write", origin.address))
        .body("must not be replayed")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 502);
    until(|| async { fixture.app.current().engine.ready() }).await;
    fixture
        .assert_replaced(old.address, replacement.address)
        .await;
    assert_eq!(old.requests.lock().unwrap().len(), 1);
    assert!(replacement.requests.lock().unwrap().is_empty());
    assert!(origin.requests.lock().unwrap().is_empty());
    fixture.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn budget_exhaustion_does_not_readmit_a_failed_lease_or_buy_another() {
    let old = support::http_proxy(None, None, 460).await;
    let fixture = Fixture::boot(vec![old.address], |config| {
        config.providers[0].refill.daily_request_budget = 1;
        config.proxy.limits.connect_timeout_ms = 500;
    })
    .await;

    let response = fixture
        .client()
        .get("http://127.0.0.1:1/")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 503);
    assert_eq!(fixture.leases().await[0].state, "revoked");
    assert_eq!(
        fixture
            .app
            .provider_controller(SOURCE)
            .await
            .unwrap()
            .decision,
        "budget_exhausted"
    );
    assert!(!fixture.app.current().engine.ready());
    assert_eq!(old.requests.lock().unwrap().len(), 1);
    assert_eq!(fixture.vendor.extractions.lock().unwrap().len(), 1);
    fixture.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn same_request_stops_after_one_replacement_even_when_refill_can_continue() {
    let first = support::http_proxy(None, None, 515).await;
    let second = support::http_proxy(None, None, 516).await;
    let third = support::http_proxy(None, None, 200).await;
    let origin = support::origin(None).await;
    let fixture = Fixture::boot(
        vec![first.address, second.address, third.address],
        |config| {
            config.providers[0].refill.daily_request_budget = 3;
            config.providers[0].refill.daily_node_budget = 3;
        },
    )
    .await;

    let response = fixture
        .client()
        .get(format!("http://{}/once", origin.address))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 502);
    until(|| async {
        fixture
            .leases()
            .await
            .iter()
            .any(|lease| lease.port == third.address.port() && lease.state == "active")
    })
    .await;
    assert_eq!(first.requests.lock().unwrap().len(), 1);
    assert_eq!(second.requests.lock().unwrap().len(), 1);
    assert!(third.requests.lock().unwrap().is_empty());
    assert!(origin.requests.lock().unwrap().is_empty());
    fixture.app.shutdown().await.unwrap();
}

#[test]
fn plaintext_mock_config_is_restricted_to_direct_loopback_requests() {
    let mut config =
        DaemonConfig::from_toml(include_str!("../../../configs/hps.kuaidaili.mock.toml")).unwrap();
    let provider = &mut config.providers[0];
    let mock = provider.endpoint.clone();
    provider.endpoint = "http://dps.kdlapi.com/api/getdps".into();
    assert!(provider.validate().is_err());
    provider.endpoint = mock;
    provider.fetch_proxy = Some("http://127.0.0.1:12345".into());
    assert!(provider.validate().is_err());
}
