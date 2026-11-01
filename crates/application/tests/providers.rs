#[path = "../../engine/tests/support/mod.rs"]
mod support;
use hps_application::{StartupOverrides, Supervisor};
use hps_config::{
    DaemonConfig, ProviderConfig, ProviderKind, ProxyConfig, ProxyProtocol, UpstreamCredentials,
};
use hps_domain::{ExtractionInput, ExtractionTask};
use hps_storage::Store;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{io::AsyncWriteExt, net::TcpListener, sync::Notify};
async fn extraction_server(
    upstream: std::net::SocketAddr,
    gate: Option<Arc<Notify>>,
    calls: Arc<AtomicUsize>,
) -> support::Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let gate = gate.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                let mut stream: hps_engine::BoxIo = Box::new(stream);
                let _ = support::head(&mut stream).await;
                calls.fetch_add(1, Ordering::Relaxed);
                if let Some(gate) = gate {
                    gate.notified().await;
                }
                let body = format!("{upstream}\n");
                let _=stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await;
            });
        }
    });
    support::Server {
        address,
        task,
        requests: Default::default(),
    }
}
fn config(endpoint: std::net::SocketAddr) -> DaemonConfig {
    DaemonConfig {
        health: hps_config::HealthConfig {
            enabled: false,
            ..Default::default()
        },
        providers: vec![ProviderConfig {
            id: "vendor".into(),
            name: "vendor".into(),
            enabled: true,
            kind: ProviderKind::Text,
            endpoint: format!("http://{endpoint}/extract"),
            ttl_secs: Some(300),
            upstream_credentials: Some(UpstreamCredentials {
                username: "vendor".into(),
                password: "private".into(),
            }),
            ..Default::default()
        }],
        proxy: ProxyConfig {
            inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
            ..Default::default()
        },
        ..Default::default()
    }
}
async fn finished(app: &Arc<Supervisor>, id: i64) -> ExtractionTask {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let task = app.extraction_task(id).await.unwrap();
            if !["accepted", "requesting"].contains(&task.state.as_str()) {
                return task;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn extracted_authenticated_upstream_is_live_without_restart_and_disable_revokes_new_admission()
 {
    let upstream = support::http_proxy(None, Some("Basic dmVuZG9yOnByaXZhdGU="), 200).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let extraction = extraction_server(upstream.address, None, calls.clone()).await;
    let config = config(extraction.address);
    let app = Supervisor::boot(
        Store::open(None, config.clone()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    app.confirm_startup().await.unwrap();
    assert!(!app.current().engine.ready());
    let task = app
        .extract(
            "vendor".into(),
            ExtractionInput {
                request_id: "request-1".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(finished(&app, task.id).await.state, "succeeded");
    // Publication follows persistence in the same background operation.
    tokio::time::timeout(Duration::from_secs(3), async {
        while !app.current().engine.ready() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let origin = support::origin(None).await;
    let proxy = format!(
        "http://{}",
        app.current().engine.status().inbounds[0]
            .listen
            .as_ref()
            .unwrap()
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(&proxy).unwrap())
        .build()
        .unwrap();
    let response = client
        .get(format!("http://{}/data", origin.address))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let dedup = app
        .extract(
            "vendor".into(),
            ExtractionInput {
                request_id: "request-1".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(dedup.id, task.id);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let before = app.provider_detail("vendor").await.unwrap();
    assert_eq!(before.provider.active_leases, 1);
    let serialized = serde_json::to_string(&before).unwrap();
    assert!(!serialized.contains("private"));
    assert!(!serialized.contains("password\":\"private"));
    let mut provider = config.providers[0].clone();
    provider.enabled = false;
    let result = app
        .edit_provider("vendor".into(), Some(provider), 1)
        .await
        .unwrap();
    assert_eq!(result.state, "applied");
    assert!(!app.current().engine.ready());
    assert_eq!(
        app.provider_leases(None, None, 50).await.unwrap()[0].state,
        "source_disabled"
    );
    app.shutdown().await.unwrap();
}
#[tokio::test]
async fn inflight_merge_and_config_change_do_not_publish_old_response_and_shutdown_is_bounded() {
    let gate = Arc::new(Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let extraction = extraction_server(
        "127.0.0.1:8080".parse().unwrap(),
        Some(gate.clone()),
        calls.clone(),
    )
    .await;
    let config = config(extraction.address);
    let app = Supervisor::boot(
        Store::open(None, config.clone()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    app.confirm_startup().await.unwrap();
    let a = app
        .extract(
            "vendor".into(),
            ExtractionInput {
                request_id: "first".into(),
            },
        )
        .await
        .unwrap();
    let b = app
        .extract(
            "vendor".into(),
            ExtractionInput {
                request_id: "second".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(a.id, b.id);
    let mut changed = config.providers[0].clone();
    changed.params.insert("region".into(), "other".into());
    app.edit_provider("vendor".into(), Some(changed), 1)
        .await
        .unwrap();
    gate.notify_one();
    assert_eq!(finished(&app, a.id).await.state, "discarded");
    assert!(!app.current().engine.ready());
    assert!(
        app.provider_leases(None, None, 50)
            .await
            .unwrap()
            .is_empty()
    );
    app.shutdown().await.unwrap();
    let app = Supervisor::boot(
        Store::open(None, config).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    let _ = app
        .extract(
            "vendor".into(),
            ExtractionInput {
                request_id: "shutdown".into(),
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), app.shutdown())
        .await
        .unwrap()
        .unwrap();
}
