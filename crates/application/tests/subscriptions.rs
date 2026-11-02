use hps_application::{ApplicationError, StartupOverrides, Supervisor};
use hps_config::{DaemonConfig, SubscriptionConfig, SubscriptionFormat, SubscriptionRefresh};
use hps_domain::{NodeQuery, SelectionQuery};
use hps_storage::{StorageError, Store};
use std::sync::Arc;
fn config() -> DaemonConfig {
    let mut config = DaemonConfig::default();
    config.health.enabled = false;
    config.subscriptions.push(SubscriptionConfig {
        id: "standard".into(),
        name: "标准订阅".into(),
        url: "https://example.test/sub?token=private-token".into(),
        ..Default::default()
    });
    config
}
async fn boot(config: DaemonConfig) -> (Store, Arc<Supervisor>) {
    let store = Store::open(None, config).unwrap();
    let service = Supervisor::boot(store.clone(), StartupOverrides::default())
        .await
        .unwrap();
    (store, service)
}
#[tokio::test]
async fn snapshots_publish_atomically_without_leases_and_keep_old_snapshot_on_failure() {
    let (store, service) = boot(config()).await;
    let revision = service.subscriptions().running_revision;
    let result = service
        .refresh_subscription(
            "standard".into(),
            SubscriptionRefresh {
                expected_revision: revision,
                document: Some(
                    "socks5://user:private-password@127.0.0.1:1080#node\nhttp://127.0.0.1:8080"
                        .into(),
                ),
            },
        )
        .await
        .unwrap();
    assert_eq!(result.state, "applied");
    let view = service.subscriptions();
    let serialized = serde_json::to_string(&view).unwrap();
    assert!(!serialized.contains("private-password"));
    assert!(!serialized.contains("private-token"));
    assert_eq!(store.active_leases().await.unwrap().len(), 0);
    assert_eq!(store.extraction_tasks(None).await.unwrap().len(), 0);
    let snapshot = view.subscriptions[0].snapshot.as_ref().unwrap();
    assert_eq!(snapshot.nodes.len(), 2);
    assert!(snapshot.nodes.iter().all(|n| n.expires_at_unix.is_none()));
    let inventory = service.nodes(NodeQuery::default()).await.unwrap();
    assert_eq!(inventory.nodes.len(), 2);
    assert!(inventory.nodes.iter().all(|n| n.kind == "subscription"
        && n.provider_id.as_deref() == Some("subscription:standard")
        && n.lease_id.is_none()
        && n.expiry_basis.is_none()
        && n.eligible));
    let decision = service.simulate_group(SelectionQuery::default()).unwrap();
    assert_eq!(decision.eligible, 2);
    let old_id = snapshot.nodes[0].id.clone();
    for document in ["", "ss://unsupported", "proxies: []"] {
        assert!(
            service
                .refresh_subscription(
                    "standard".into(),
                    SubscriptionRefresh {
                        expected_revision: result.running_revision,
                        document: Some(document.into())
                    }
                )
                .await
                .is_err()
        );
        assert_eq!(
            service.subscriptions().subscriptions[0]
                .snapshot
                .as_ref()
                .unwrap()
                .nodes[0]
                .id,
            old_id
        );
        assert_eq!(
            service.subscriptions().running_revision,
            result.running_revision
        );
    }
    assert!(matches!(
        service
            .refresh_subscription(
                "standard".into(),
                SubscriptionRefresh {
                    expected_revision: revision,
                    document: Some("http://127.0.0.1:9999".into())
                }
            )
            .await,
        Err(ApplicationError::Storage(StorageError::Conflict))
    ));
    // Editing the definition reuses the secret URL, cannot inject cache nodes, and disable drains new selection.
    let mut edit = view.subscriptions[0].clone();
    edit.enabled = false;
    let disabled = service
        .edit_subscription("standard".into(), Some(edit), result.running_revision)
        .await
        .unwrap();
    assert_eq!(
        service
            .simulate_group(SelectionQuery::default())
            .unwrap()
            .eligible,
        0
    );
    assert!(disabled.running_revision > result.running_revision);
    service.shutdown().await.unwrap();
}
#[tokio::test]
async fn successful_subscription_and_exact_source_survive_sqlite_restart() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(Some(directory.path()), config()).unwrap();
    let service = Supervisor::boot(store.clone(), StartupOverrides::default())
        .await
        .unwrap();
    let revision = service.subscriptions().running_revision;
    service
        .refresh_subscription(
            "standard".into(),
            SubscriptionRefresh {
                expected_revision: revision,
                document: Some("http://127.0.0.1:8080".into()),
            },
        )
        .await
        .unwrap();
    let before = service.subscriptions();
    service.shutdown().await.unwrap();
    drop(service);
    drop(store);
    let store = Store::open(Some(directory.path()), DaemonConfig::default()).unwrap();
    let service = Supervisor::boot(store.clone(), StartupOverrides::default())
        .await
        .unwrap();
    assert_eq!(
        service.subscriptions().subscriptions[0]
            .snapshot
            .as_ref()
            .unwrap()
            .nodes[0]
            .id,
        before.subscriptions[0].snapshot.as_ref().unwrap().nodes[0].id
    );
    assert_eq!(
        service
            .simulate_group(SelectionQuery::default())
            .unwrap()
            .eligible,
        1
    );
    service.shutdown().await.unwrap();
}
#[tokio::test]
async fn remote_clash_get_uses_real_http_and_rejects_redirect_and_bad_refresh() {
    use axum::{Router, routing::get};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app=Router::new().route("/subscription",get(||async{"proxies:\n - {name: fixture, type: socks5, server: 127.0.0.1, port: 1080, udp: true}"})).route("/redirect",get(||async{axum::response::Redirect::temporary("/subscription")}));
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut config = config();
    config.subscriptions[0].url = format!("http://{addr}/subscription");
    config.subscriptions[0].format = SubscriptionFormat::Clash;
    let (_, service) = boot(config).await;
    let r = service.subscriptions().running_revision;
    let applied = service
        .refresh_subscription(
            "standard".into(),
            SubscriptionRefresh {
                expected_revision: r,
                document: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        service.subscriptions().subscriptions[0]
            .snapshot
            .as_ref()
            .unwrap()
            .origin,
        "remote_get"
    );
    let mut edit = service.subscriptions().subscriptions[0].clone();
    edit.url = format!("http://{addr}/redirect");
    let changed = service
        .edit_subscription("standard".into(), Some(edit), applied.running_revision)
        .await
        .unwrap();
    assert!(
        service
            .refresh_subscription(
                "standard".into(),
                SubscriptionRefresh {
                    expected_revision: changed.running_revision,
                    document: None
                }
            )
            .await
            .is_err()
    );
    service.shutdown().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn subscription_commit_does_not_depend_on_post_publication_lease_reload() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(Some(directory.path()), config()).unwrap();
    let database = store.database_path.clone().unwrap();
    let service = Supervisor::boot(store.clone(), Default::default())
        .await
        .unwrap();
    let revision = service.subscriptions().running_revision;
    let first = service
        .refresh_subscription(
            "standard".into(),
            SubscriptionRefresh {
                expected_revision: revision,
                document: Some("http://127.0.0.1:8080".into()),
            },
        )
        .await
        .unwrap();
    // Fault only the optional, unrelated lease inventory read. Configuration
    // persistence and the prebuilt subscription generation remain functional.
    let fault = rusqlite::Connection::open(database).unwrap();
    fault
        .execute_batch("ALTER TABLE provider_leases RENAME TO provider_leases_fault")
        .unwrap();
    assert!(store.active_leases().await.is_err());
    let applied = service
        .refresh_subscription(
            "standard".into(),
            SubscriptionRefresh {
                expected_revision: first.running_revision,
                document: Some("http://127.0.0.1:8081".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(applied.state, "applied");
    let view = service.subscriptions();
    let expected = &view.subscriptions[0].snapshot.as_ref().unwrap().nodes[0].id;
    assert_eq!(
        service
            .simulate_group(SelectionQuery::default())
            .unwrap()
            .selected
            .as_ref(),
        Some(expected)
    );
    assert_eq!(
        store.workspace().await.unwrap().desired.revision.id,
        applied.running_revision
    );
    fault
        .execute_batch("ALTER TABLE provider_leases_fault RENAME TO provider_leases")
        .unwrap();
    service.shutdown().await.unwrap();
}
