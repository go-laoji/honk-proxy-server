#[path = "../../engine/tests/support/mod.rs"]
mod support;
use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use hps_application::{StartupOverrides, Supervisor};
use hps_config::{
    DaemonConfig, ProviderConfig, ProviderKind, ProxyConfig, ProxyProtocol, RefillMode,
};
use hps_domain::ExtractionInput;
use hps_storage::Store;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
#[derive(Clone, Copy)]
enum Mode {
    Good(u64),
    Malformed,
    Rate,
    Empty,
}
struct Mock {
    mode: Mode,
    calls: AtomicUsize,
    quantities: Mutex<Vec<u32>>,
    first_gate: Option<Arc<Notify>>,
}
struct Server {
    address: std::net::SocketAddr,
    mock: Arc<Mock>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(mode: Mode, first_gate: Option<Arc<Notify>>) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mock = Arc::new(Mock {
        mode,
        calls: AtomicUsize::new(0),
        quantities: Mutex::new(vec![]),
        first_gate,
    });
    async fn extract(
        State(mock): State<Arc<Mock>>,
        Query(query): Query<HashMap<String, String>>,
    ) -> axum::response::Response {
        let count = query.get("num").unwrap().parse::<u32>().unwrap();
        let call = mock.calls.fetch_add(1, Ordering::Relaxed) + 1;
        mock.quantities.lock().unwrap().push(count);
        if call == 1
            && let Some(gate) = &mock.first_gate
        {
            gate.notified().await;
        }
        match mock.mode {
            Mode::Good(ttl)=>Json(serde_json::json!({"data":{"proxies":(1..=count).map(|n|serde_json::json!({"ip":format!("192.0.2.{}",(call-1)*20+n as usize),"port":8080,"ttl":ttl})).collect::<Vec<_>>()}})).into_response(),
            Mode::Malformed=>"not-json-private-vendor-body".into_response(),
            Mode::Rate=>(StatusCode::TOO_MANY_REQUESTS,[("retry-after","60")],"").into_response(),
            Mode::Empty=>Json(serde_json::json!({"data":{"proxies":[]}})).into_response(),
        }
    }
    let router = Router::new()
        .route("/extract", get(extract))
        .with_state(mock.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Server {
        address,
        mock,
        task,
    }
}
fn config(server: &Server, mode: RefillMode) -> DaemonConfig {
    let mut provider = ProviderConfig {
        id: "source".into(),
        name: "source".into(),
        enabled: true,
        kind: ProviderKind::Json,
        endpoint: format!("http://{}/extract", server.address),
        count: 2,
        count_parameter: Some("num".into()),
        min_remaining_secs: 0,
        min_interval_secs: 1,
        ..Default::default()
    };
    provider.refill.enabled = true;
    provider.refill.mode = mode;
    provider.refill.min_nodes = 1;
    provider.refill.target_nodes = 3;
    provider.refill.max_nodes = 4;
    provider.refill.batch_size = 2;
    provider.refill.retire_before_secs = 0;
    provider.refill.forecast_secs = 0;
    provider.refill.idle_timeout_secs = 30;
    provider.refill.backoff_initial_secs = 1;
    provider.refill.backoff_max_secs = 1;
    provider.refill.failure_limit = 2;
    provider.refill.daily_request_budget = 3;
    provider.refill.daily_node_budget = 6;
    let mut config = DaemonConfig {
        providers: vec![provider],
        proxy: ProxyConfig {
            inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
            ..Default::default()
        },
        ..Default::default()
    };
    config.proxy.limits.min_remaining_lifetime_secs = 0;
    config
}
async fn app(config: DaemonConfig) -> Arc<Supervisor> {
    let app = Supervisor::boot(
        Store::open(None, config).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    app.confirm_startup().await.unwrap();
    app
}
async fn until(mut condition: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(7), async {
        loop {
            if condition().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
async fn demand(app: &Arc<Supervisor>) {
    let endpoint = format!(
        "http://{}",
        app.current().engine.status().inbounds[0]
            .listen
            .as_ref()
            .unwrap()
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(endpoint).unwrap())
        .build()
        .unwrap();
    let response = client.get("http://target.invalid/").send().await.unwrap();
    assert_eq!(response.status(), 503);
    let _ = response.bytes().await.unwrap();
    drop(client);
}
#[tokio::test]
async fn on_demand_refill_uses_real_dial_signal_hysteresis_inflight_and_requested_quantities() {
    let gate = Arc::new(Notify::new());
    let vendor = server(Mode::Good(300), Some(gate.clone())).await;
    let app = app(config(&vendor, RefillMode::OnDemand)).await;
    assert_eq!(
        app.provider_controller("source").await.unwrap().decision,
        "idle"
    );
    assert_eq!(vendor.mock.calls.load(Ordering::Relaxed), 0);
    demand(&app).await;
    until(async || vendor.mock.calls.load(Ordering::Relaxed) == 1).await;
    let snapshot = app.provider_controller("source").await.unwrap();
    assert_eq!(
        (
            snapshot.used_requests,
            snapshot.reserved_nodes,
            snapshot.inflight_nodes
        ),
        (1, 2, 2)
    );
    let mut jobs = vec![];
    for _ in 0..8 {
        let app = app.clone();
        jobs.push(tokio::spawn(async move {
            app.evaluate_controller("source".into()).await.unwrap()
        }));
    }
    for job in jobs {
        assert!(job.await.unwrap().task.is_none());
    }
    let manual = app
        .extract(
            "source".into(),
            ExtractionInput {
                request_id: "while-inflight".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(Some(manual.id), snapshot.last_task_id);
    assert_eq!(vendor.mock.calls.load(Ordering::Relaxed), 1);
    gate.notify_one();
    until(async || {
        app.provider_controller("source")
            .await
            .unwrap()
            .projected_inventory
            == 3
    })
    .await;
    let status = app.provider_controller("source").await.unwrap();
    assert_eq!((status.used_requests, status.reserved_nodes), (2, 3));
    assert_eq!(*vendor.mock.quantities.lock().unwrap(), vec![2, 1]);
    assert_eq!(status.decision, "target_satisfied");
    assert!(app.current().engine.ready());
    app.shutdown().await.unwrap();
}
#[tokio::test]
async fn idle_source_does_not_purchase_again_after_its_short_leases_expire() {
    let vendor = server(Mode::Good(2), None).await;
    let mut settings = config(&vendor, RefillMode::OnDemand);
    settings.providers[0].refill.target_nodes = 2;
    settings.providers[0].refill.idle_timeout_secs = 2;
    let app = app(settings).await;
    demand(&app).await;
    until(async || vendor.mock.calls.load(Ordering::Relaxed) == 1).await;
    until(async || app.provider_detail("source").await.unwrap().tasks[0].state == "succeeded")
        .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let status = app.provider_controller("source").await.unwrap();
    assert_eq!(status.decision, "idle");
    assert_eq!(vendor.mock.calls.load(Ordering::Relaxed), 1);
    assert!(!app.current().engine.ready());
    app.shutdown().await.unwrap();
}
#[tokio::test]
async fn malformed_outcome_pauses_and_rate_limit_respects_retry_after_without_replay() {
    for mode in [Mode::Malformed, Mode::Rate] {
        let vendor = server(mode, None).await;
        let app = app(config(&vendor, RefillMode::Maintain)).await;
        until(async || {
            app.provider_detail("source")
                .await
                .unwrap()
                .tasks
                .first()
                .is_some_and(|t| !["accepted", "requesting"].contains(&t.state.as_str()))
        })
        .await;
        let snapshot = app.provider_controller("source").await.unwrap();
        match mode {
            Mode::Malformed => {
                assert_eq!(
                    snapshot.paused_reason.as_deref(),
                    Some("invalid_vendor_payload")
                );
                assert_eq!(snapshot.decision, "paused");
            }
            Mode::Rate => {
                assert!(snapshot.paused_reason.is_none());
                assert_eq!(snapshot.decision, "cooldown");
                assert!(snapshot.next_allowed_at >= hps_storage::unix_now() + 59);
            }
            _ => unreachable!(),
        }
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(vendor.mock.calls.load(Ordering::Relaxed), 1);
        assert_eq!(snapshot.used_requests, 1);
        app.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn repeated_empty_results_stop_at_failure_limit_and_preserve_conservative_budget() {
    let vendor = server(Mode::Empty, None).await;
    let app = app(config(&vendor, RefillMode::Maintain)).await;
    until(async || {
        app.provider_controller("source")
            .await
            .unwrap()
            .paused_reason
            .is_some()
    })
    .await;
    let status = app.provider_controller("source").await.unwrap();
    assert_eq!(
        status.paused_reason.as_deref(),
        Some("consecutive_failures")
    );
    assert_eq!(status.failure_count, 2);
    assert_eq!((status.used_requests, status.reserved_nodes), (2, 4));
    assert_eq!(vendor.mock.calls.load(Ordering::Relaxed), 2);
    app.shutdown().await.unwrap();
}
