use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

async fn fixture() -> (
    Arc<Supervisor>,
    std::net::SocketAddr,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut body = vec![];
                if socket.read_to_end(&mut body).await.is_ok() {
                    let _ = socket.write_all(&body).await;
                    let _ = socket.shutdown().await;
                }
            });
        }
    });
    let mut config = DaemonConfig::default();
    config.health.enabled = false;
    config.proxy.routing.default_action = hps_domain::RouteAction::Direct;
    config.proxy.inbounds.push(hps_config::InboundConfig {
        id: "observed-http".into(),
        protocol: hps_config::ProxyProtocol::Http,
        listen: "127.0.0.1:0".parse().unwrap(),
        enabled: true,
        tls: None,
        max_connections: 32,
        bytes_per_second: 0,
    });
    let service = Supervisor::boot(
        Store::open(None, config).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    service.confirm_startup().await.unwrap();
    (service, target, task)
}
async fn tunnel(service: &Arc<Supervisor>, target: std::net::SocketAddr) -> TcpStream {
    let address = service.current().engine.status().inbounds[0]
        .listen
        .clone()
        .unwrap();
    let mut socket = TcpStream::connect(address).await.unwrap();
    socket
        .write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut head = vec![];
    while !head.ends_with(b"\r\n\r\n") {
        head.push(socket.read_u8().await.unwrap());
    }
    assert!(head.starts_with(b"HTTP/1.1 200"));
    socket
}
#[tokio::test]
async fn immutable_pages_and_confirmed_disconnect_never_expand_to_new_matching_flows() {
    let (service, target, origin) = fixture().await;
    let mut first = tunnel(&service, target).await;
    let mut second = tunnel(&service, target).await;
    let query = ConnectionQuery {
        limit: Some(1),
        sort: Some("oldest".into()),
        ..Default::default()
    };
    let page = service.connections(query.clone()).await.unwrap();
    assert_eq!(page.filtered_total, 2);
    assert_eq!(page.next_offset, Some(1));
    let mut third = tunnel(&service, target).await;
    let next = service
        .connections(ConnectionQuery {
            snapshot: Some(page.snapshot.clone()),
            expected_epoch: Some(page.epoch.clone()),
            offset: 1,
            ..query.clone()
        })
        .await
        .unwrap();
    assert_eq!(next.filtered_total, 2);
    assert_ne!(next.records[0].id, page.records[0].id);
    assert_eq!(next.statistics.started, 2);
    let changed = service
        .connections(ConnectionQuery {
            snapshot: Some(page.snapshot.clone()),
            node: Some("@direct".into()),
            ..query.clone()
        })
        .await;
    assert!(matches!(changed, Err(ApplicationError::ConnectionFilters)));
    let source = service
        .connections(ConnectionQuery {
            source: Some("::ffff:127.0.0.0/104".into()),
            source_port: Some(third.local_addr().unwrap().port()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(source.filtered_total, 1);
    let preview = service
        .preview_disconnect_connections(ConnectionDisconnectPreviewInput {
            snapshot: page.snapshot.clone(),
            expected_epoch: page.epoch.clone(),
            ids: vec![],
        })
        .await
        .unwrap();
    assert_eq!(preview.targets.len(), 2);
    let confirm = ConnectionDisconnectConfirm {
        token: preview.token.clone(),
        expected_epoch: page.epoch.clone(),
    };
    let result = service
        .confirm_disconnect_connections(confirm.clone())
        .await
        .unwrap();
    assert_eq!(result.requested, 2);
    let repeat = service
        .confirm_disconnect_connections(confirm)
        .await
        .unwrap();
    assert_eq!(result.requested_at_unix_ms, repeat.requested_at_unix_ms);
    for socket in [&mut first, &mut second] {
        let mut bytes = vec![];
        let _ = tokio::time::timeout(Duration::from_secs(1), socket.read_to_end(&mut bytes))
            .await
            .unwrap();
    }
    third.write_all(b"new-flow-not-in-preview").await.unwrap();
    third.shutdown().await.unwrap();
    let mut echo = vec![];
    third.read_to_end(&mut echo).await.unwrap();
    assert_eq!(echo, b"new-flow-not-in-preview");
    assert!(matches!(
        service
            .disconnect_connection(ConnectionDisconnectInput {
                id: page.records[0].id.clone(),
                expected_epoch: uuid::Uuid::new_v4().to_string()
            })
            .await,
        Err(ApplicationError::ConnectionEpoch)
    ));
    service.shutdown().await.unwrap();
    origin.abort();
}
#[tokio::test]
async fn snapshot_eviction_expiry_instance_and_diagnostic_capacity_are_explicit() {
    let (service, target, origin) = fixture().await;
    let _socket = tunnel(&service, target).await;
    let first = service
        .connections(ConnectionQuery::default())
        .await
        .unwrap();
    for _ in 0..MAX_SNAPSHOTS {
        service
            .connections(ConnectionQuery::default())
            .await
            .unwrap();
    }
    assert!(matches!(
        service
            .connections(ConnectionQuery {
                snapshot: Some(first.snapshot),
                ..Default::default()
            })
            .await,
        Err(ApplicationError::ConnectionSnapshot)
    ));
    let page = service
        .connections(ConnectionQuery::default())
        .await
        .unwrap();
    {
        let mut snapshots = service.observations.snapshots.lock().unwrap();
        Arc::get_mut(snapshots.get_mut(&page.snapshot).unwrap())
            .unwrap()
            .deadline = Instant::now() - Duration::from_secs(1);
    }
    assert!(matches!(
        service
            .connections(ConnectionQuery {
                snapshot: Some(page.snapshot),
                ..Default::default()
            })
            .await,
        Err(ApplicationError::ConnectionSnapshot)
    ));
    assert!(matches!(
        service
            .connections(ConnectionQuery {
                expected_epoch: Some(uuid::Uuid::new_v4().to_string()),
                ..Default::default()
            })
            .await,
        Err(ApplicationError::ConnectionEpoch)
    ));
    let permit = service
        .observations
        .slots
        .clone()
        .acquire_many_owned(4)
        .await
        .unwrap();
    assert!(matches!(
        service.connections(ConnectionQuery::default()).await,
        Err(ApplicationError::ConnectionCapacity)
    ));
    drop(permit);
    assert!(
        service
            .connections(ConnectionQuery {
                source: Some("invalid-host.example".into()),
                ..Default::default()
            })
            .await
            .is_err()
    );
    service.shutdown().await.unwrap();
    origin.abort();
}
#[tokio::test]
async fn accepted_work_keeps_its_slot_and_service_ownership_after_caller_disconnect() {
    let (service, _, origin) = fixture().await;
    let (entered, wait_entered) = tokio::sync::oneshot::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = finished.clone();
    let worker = service.clone();
    let caller = tokio::spawn(async move {
        worker
            .connection_work(move |_| {
                let _ = entered.send(());
                wait_release.recv_timeout(Duration::from_secs(2)).unwrap();
                flag.store(true, std::sync::atomic::Ordering::Release);
                Ok(())
            })
            .await
    });
    wait_entered.await.unwrap();
    caller.abort();
    assert_eq!(service.observations.slots.available_permits(), 3);
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.observations.slots.available_permits() != 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(finished.load(std::sync::atomic::Ordering::Acquire));
    service.shutdown().await.unwrap();
    origin.abort();
}

#[tokio::test]
async fn summaries_use_persisted_failures_current_provider_fingerprints_and_actual_generation() {
    let (service, target, origin) = fixture().await;
    let socket = tunnel(&service, target).await;
    let summary = service.connection_summary().await.unwrap();
    assert_eq!(summary.statistics.active, 1);
    assert_eq!(summary.proxy.active_connections, 1);
    assert!(summary.attention.is_empty());
    let mut provider = hps_config::ProviderConfig {
        id: "summary-source".into(),
        kind: hps_config::ProviderKind::Text,
        endpoint: "http://127.0.0.1:9/list".into(),
        enabled: true,
        count: 1,
        ttl_secs: Some(300),
        ..Default::default()
    };
    service
        .edit_provider(provider.id.clone(), Some(provider.clone()), 1)
        .await
        .unwrap();
    let (task, _) = service
        .store
        .begin_extraction(
            provider.clone(),
            "summary-test".into(),
            hps_provider::fingerprint(&provider),
        )
        .await
        .unwrap();
    service
        .store
        .fail_extraction(
            task.id,
            hps_provider::FetchError {
                code: "test_supplier_failure",
                outcome_unknown: false,
                retry_after_secs: None,
            },
        )
        .await
        .unwrap();
    let summary = service.connection_summary().await.unwrap();
    assert!(
        summary
            .attention
            .iter()
            .any(|a| a.kind == "provider" && a.code == "test_supplier_failure")
    );
    assert_eq!(
        service
            .connections(ConnectionQuery::default())
            .await
            .unwrap()
            .records[0]
            .routing
            .configuration_revision,
        1
    );
    provider.enabled = false;
    service
        .edit_provider(
            provider.id.clone(),
            Some(provider),
            summary.running_revision,
        )
        .await
        .unwrap();
    assert!(
        !service
            .connection_summary()
            .await
            .unwrap()
            .attention
            .iter()
            .any(|a| a.kind == "provider")
    );
    let revision = service.current().revision;
    // This tests presentation of an actual persisted failure fact; it is not a kernel test.
    service
        .store
        .record_failure(revision, revision, "dns_capture_preparation_failed")
        .await
        .unwrap();
    let summary = service.connection_summary().await.unwrap();
    assert!(
        summary
            .attention
            .iter()
            .any(|a| a.kind == "linux_dns_capture" && a.code == "dns_capture_preparation_failed")
    );
    drop(socket);
    service.shutdown().await.unwrap();
    origin.abort();
}
