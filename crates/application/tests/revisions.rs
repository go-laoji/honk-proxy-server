#[path = "../../engine/tests/support/mod.rs"]
mod support;
use hps_application::{ApplicationError, StartupOverrides, Supervisor};
use hps_config::{DaemonConfig, ProxyConfig, ProxyProtocol, UpstreamCredentials};
use hps_domain::{ConfigActivate, ConfigInput};
use hps_engine::BoxIo;
use hps_storage::{StorageError, Store};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

fn input(config: &DaemonConfig, expected_revision: i64) -> ConfigInput {
    ConfigInput {
        toml: toml::to_string_pretty(config).unwrap(),
        expected_revision,
        expected_draft_version: None,
        note: "test revision".into(),
        use_draft_credentials: false,
    }
}
async fn tunnel(supervisor: &Arc<Supervisor>, destination: std::net::SocketAddr) -> BoxIo {
    let address = supervisor.current().engine.status().inbounds[0]
        .listen
        .clone()
        .unwrap();
    let mut io: BoxIo = Box::new(TcpStream::connect(address).await.unwrap());
    io.write_all(
        format!("CONNECT {destination} HTTP/1.1\r\nHost: {destination}\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    assert!(support::head(&mut io).await.starts_with(b"HTTP/1.1 200"));
    io
}
async fn eventually(mut condition: impl FnMut() -> bool) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(condition());
}

#[tokio::test]
async fn same_socket_handoff_retains_old_flow_and_counts_it_against_reduced_limits() {
    let a = support::http_proxy(None, None, 200).await;
    let b = support::http_proxy(None, None, 200).await;
    let echo = support::echo(false).await;
    let config = DaemonConfig {
        health: hps_config::HealthConfig {
            enabled: false,
            ..Default::default()
        },
        proxy: ProxyConfig {
            inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
            upstreams: vec![support::upstream(a.address, ProxyProtocol::Http)],
            ..Default::default()
        },
        ..Default::default()
    };
    let store = Store::open(None, config.clone()).unwrap();
    let supervisor = Supervisor::boot(store, StartupOverrides::default())
        .await
        .unwrap();
    supervisor.confirm_startup().await.unwrap();
    let mut old = tunnel(&supervisor, echo.address).await;
    let address = supervisor.current().engine.status().inbounds[0]
        .listen
        .clone();
    let mut next = config.clone();
    next.instance_name = "new-generation".into();
    next.proxy.upstreams[0] = support::upstream(b.address, ProxyProtocol::Http);
    next.proxy.limits.max_connections = 1;
    let revision = supervisor.create_revision(input(&next, 1)).await.unwrap();
    let result = supervisor
        .activate(ConfigActivate {
            revision: revision.id,
            expected_revision: 1,
            defer_until_restart: false,
        })
        .await
        .unwrap();
    assert_eq!(result.state, "applied");
    assert_eq!(
        supervisor.current().engine.status().inbounds[0].listen,
        address
    );
    assert_eq!(supervisor.current().engine.status().active_connections, 1);
    let mut denied = TcpStream::connect(address.as_ref().unwrap()).await.unwrap();
    let mut byte = [0; 1];
    assert_eq!(denied.read(&mut byte).await.unwrap(), 0);
    old.write_all(b"still-alive").await.unwrap();
    old.shutdown().await.unwrap();
    let mut body = Vec::new();
    old.read_to_end(&mut body).await.unwrap();
    assert_eq!(&body, b"still-alive");
    eventually(|| supervisor.current().engine.status().active_connections == 0).await;
    let mut new = tunnel(&supervisor, echo.address).await;
    new.shutdown().await.unwrap();
    new.read_to_end(&mut Vec::new()).await.unwrap();
    assert_eq!(a.requests.lock().unwrap().len(), 1);
    assert_eq!(b.requests.lock().unwrap().len(), 1);
    assert_eq!(
        supervisor.workspace().await.unwrap().running_revision,
        revision.id
    );
    supervisor.shutdown().await.unwrap();
    supervisor.shutdown().await.unwrap();
}

#[tokio::test]
async fn occupied_candidate_and_changed_tls_files_leave_both_head_and_running_generation() {
    let upstream = support::http_proxy(None, None, 200).await;
    let echo = support::echo(false).await;
    let config = DaemonConfig {
        health: hps_config::HealthConfig {
            enabled: false,
            ..Default::default()
        },
        proxy: ProxyConfig {
            inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
            upstreams: vec![support::upstream(upstream.address, ProxyProtocol::Http)],
            ..Default::default()
        },
        ..Default::default()
    };
    let supervisor = Supervisor::boot(
        Store::open(None, config.clone()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    let blocked = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut candidate = config.clone();
    candidate.proxy.inbounds[0].listen = blocked.local_addr().unwrap();
    let revision = supervisor
        .create_revision(input(&candidate, 1))
        .await
        .unwrap();
    assert!(
        supervisor
            .activate(ConfigActivate {
                revision: revision.id,
                expected_revision: 1,
                defer_until_restart: false
            })
            .await
            .is_err()
    );
    let state = supervisor.workspace().await.unwrap();
    assert_eq!((state.desired_revision, state.running_revision), (1, 1));
    assert_eq!(
        state.activations[0].error_code.as_deref(),
        Some("listen_preparation_failed")
    );
    let certificate = support::certificate();
    candidate = config.clone();
    candidate.proxy.inbounds[0].protocol = ProxyProtocol::Https;
    candidate.proxy.inbounds[0].tls = Some(certificate.config.clone());
    let revision = supervisor
        .create_revision(input(&candidate, 1))
        .await
        .unwrap();
    std::fs::write(&certificate.config.private_key, b"invalid replacement key").unwrap();
    assert!(
        supervisor
            .activate(ConfigActivate {
                revision: revision.id,
                expected_revision: 1,
                defer_until_restart: false
            })
            .await
            .is_err()
    );
    assert_eq!(supervisor.workspace().await.unwrap().desired_revision, 1);
    let mut connection = tunnel(&supervisor, echo.address).await;
    connection.shutdown().await.unwrap();
    connection.read_to_end(&mut Vec::new()).await.unwrap();
    supervisor.shutdown().await.unwrap();
}

#[tokio::test]
async fn pending_restart_recovers_desired_and_rollback_conflicts_are_explicit() {
    let directory = tempfile::tempdir().unwrap();
    let supervisor = Supervisor::boot(
        Store::open(Some(directory.path()), legacy_config()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    supervisor.confirm_startup().await.unwrap();
    let candidate = DaemonConfig {
        health: hps_config::HealthConfig {
            enabled: false,
            ..Default::default()
        },
        management_listen: "127.0.0.1:0".parse().unwrap(),
        log_filter: "warn".into(),
        ..Default::default()
    };
    let revision = supervisor
        .create_revision(input(&candidate, 1))
        .await
        .unwrap();
    let result = supervisor
        .activate(ConfigActivate {
            revision: revision.id,
            expected_revision: 1,
            defer_until_restart: false,
        })
        .await
        .unwrap();
    assert_eq!(result.state, "pending_restart");
    assert_eq!(result.running_revision, 1);
    assert_eq!(supervisor.current().config.log_filter, "info");
    assert!(matches!(
        supervisor
            .activate(ConfigActivate {
                revision: 1,
                expected_revision: 1,
                defer_until_restart: false
            })
            .await,
        Err(ApplicationError::Storage(StorageError::Conflict))
    ));
    supervisor.shutdown().await.unwrap();
    let restored = Supervisor::boot(
        Store::open(Some(directory.path()), legacy_config()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    restored.confirm_startup().await.unwrap();
    assert_eq!(restored.current().revision, revision.id);
    assert_eq!(restored.current().config.log_filter, "warn");
    assert!(
        restored
            .workspace()
            .await
            .unwrap()
            .pending_restart
            .is_empty()
    );
    let rollback = restored
        .activate(ConfigActivate {
            revision: 1,
            expected_revision: revision.id,
            defer_until_restart: false,
        })
        .await
        .unwrap();
    assert_eq!(rollback.state, "pending_restart");
    restored.shutdown().await.unwrap();
}

#[tokio::test]
async fn draft_conflict_redacted_round_trip_and_secret_change_diff_never_expose_credentials() {
    let mut config = legacy_config();
    let mut upstream = support::upstream("127.0.0.1:18080".parse().unwrap(), ProxyProtocol::Http);
    upstream.credentials = Some(UpstreamCredentials {
        username: "supplier".into(),
        password: "private-test-password".into(),
    });
    config.proxy.upstreams.push(upstream);
    let supervisor = Supervisor::boot(
        Store::open(None, config).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    let state = supervisor.workspace().await.unwrap();
    assert!(!state.current_toml.contains("private-test-password"));
    let mut draft = ConfigInput {
        toml: state.current_toml,
        expected_revision: 1,
        expected_draft_version: Some(1),
        note: "redacted import".into(),
        use_draft_credentials: false,
    };
    assert!(supervisor.validate(draft.clone()).await.valid);
    supervisor.save_draft(draft.clone()).await.unwrap();
    assert!(matches!(
        supervisor.save_draft(draft.clone()).await,
        Err(ApplicationError::Storage(StorageError::Conflict))
    ));
    draft.expected_draft_version = Some(2);
    let revision = supervisor.create_revision(draft.clone()).await.unwrap();
    supervisor
        .activate(ConfigActivate {
            revision: revision.id,
            expected_revision: 1,
            defer_until_restart: false,
        })
        .await
        .unwrap();
    assert_eq!(
        supervisor.current().config.proxy.upstreams[0]
            .credentials
            .as_ref()
            .unwrap()
            .password,
        "private-test-password"
    );
    let mut changed = supervisor.current().config.clone();
    changed.proxy.upstreams[0]
        .credentials
        .as_mut()
        .unwrap()
        .password = "new-secret-value".into();
    let validation = supervisor.validate(input(&changed, revision.id)).await;
    assert!(validation.valid);
    assert!(
        validation
            .changes
            .iter()
            .any(|change| change.path.ends_with("credentials"))
    );
    let json = serde_json::to_string(&validation).unwrap();
    assert!(!json.contains("new-secret-value") && !json.contains("private-test-password"));
    let unknown = draft.toml.replace("id = \"fixture\"", "id = \"unknown\"");
    let bad = supervisor
        .validate(ConfigInput {
            toml: unknown,
            expected_revision: revision.id,
            expected_draft_version: None,
            note: String::new(),
            use_draft_credentials: false,
        })
        .await;
    assert!(!bad.valid);
    let invalid = supervisor
        .validate(ConfigInput {
            toml: "password = ['private-test-password'\n".into(),
            expected_revision: revision.id,
            expected_draft_version: None,
            note: String::new(),
            use_draft_credentials: false,
        })
        .await;
    assert!(!invalid.valid);
    assert_eq!(invalid.diagnostics[0].path, "toml");
    assert!(
        !serde_json::to_string(&invalid)
            .unwrap()
            .contains("private-test-password")
    );
    supervisor.shutdown().await.unwrap();
}

#[tokio::test]
async fn stopping_inbound_cancels_its_connections_in_draining_generations() {
    let upstream = support::http_proxy(None, None, 200).await;
    let echo = support::echo(false).await;
    let mut config = DaemonConfig {
        health: hps_config::HealthConfig {
            enabled: false,
            ..Default::default()
        },
        proxy: ProxyConfig {
            inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
            upstreams: vec![support::upstream(upstream.address, ProxyProtocol::Http)],
            ..Default::default()
        },
        ..Default::default()
    };
    let service = Supervisor::boot(
        Store::open(None, config.clone()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    let mut connection = tunnel(&service, echo.address).await;
    config.proxy.limits.buffer_bytes = 8192;
    let revision = service.create_revision(input(&config, 1)).await.unwrap();
    service
        .activate(ConfigActivate {
            revision: revision.id,
            expected_revision: 1,
            defer_until_restart: false,
        })
        .await
        .unwrap();
    assert_eq!(service.current().engine.status().active_connections, 1);
    assert_eq!(
        service
            .stop_inbound("http")
            .await
            .unwrap()
            .active_connections,
        0
    );
    let mut byte = [0; 1];
    assert_eq!(connection.read(&mut byte).await.unwrap(), 0);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn retention_is_bounded_and_keeps_runtime_generation_and_foreign_keys_valid() {
    let mut config = legacy_config();
    config.retention.config_versions = 10;
    let service = Supervisor::boot(
        Store::open(None, config.clone()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    service.confirm_startup().await.unwrap();
    let mut head = 1;
    for number in 0..24 {
        config.instance_name = format!("revision-{number}");
        let revision = service.create_revision(input(&config, head)).await.unwrap();
        service
            .activate(ConfigActivate {
                revision: revision.id,
                expected_revision: head,
                defer_until_restart: false,
            })
            .await
            .unwrap();
        head = revision.id;
    }
    let workspace = service.workspace().await.unwrap();
    assert_eq!(workspace.desired_revision, head);
    assert!(workspace.revisions.len() <= 11);
    assert!(workspace.revisions.iter().any(|revision| revision.id == 1));
    assert!(!workspace.revisions.iter().any(|revision| revision.id == 2));
    assert!(
        workspace
            .revisions
            .iter()
            .any(|revision| revision.parent_id.is_none())
    );
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn dropped_caller_does_not_abandon_activation_queued_behind_database_lock() {
    let directory = tempfile::tempdir().unwrap();
    let service = Supervisor::boot(
        Store::open(Some(directory.path()), legacy_config()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    service.confirm_startup().await.unwrap();
    let config = DaemonConfig {
        health: hps_config::HealthConfig {
            enabled: false,
            ..Default::default()
        },
        instance_name: "survives-disconnect".into(),
        ..Default::default()
    };
    let revision = service.create_revision(input(&config, 1)).await.unwrap();
    let blocking = rusqlite::Connection::open(directory.path().join("hps.sqlite3")).unwrap();
    blocking.execute_batch("BEGIN IMMEDIATE").unwrap();
    let caller = service.clone();
    let handle = tokio::spawn(async move {
        caller
            .activate(ConfigActivate {
                revision: revision.id,
                expected_revision: 1,
                defer_until_restart: false,
            })
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    handle.abort();
    blocking.execute_batch("ROLLBACK").unwrap();
    eventually(|| service.current().config.instance_name == "survives-disconnect").await;
    assert_eq!(
        service.workspace().await.unwrap().activations[0].state,
        "applied"
    );
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn refreshed_redacted_draft_preserves_new_and_rotated_credentials_without_stale_reference() {
    let service = Supervisor::boot(
        Store::open(None, legacy_config()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    let mut config = legacy_config();
    let mut upstream = support::upstream("127.0.0.1:18080".parse().unwrap(), ProxyProtocol::Http);
    upstream.credentials = Some(UpstreamCredentials {
        username: "supplier".into(),
        password: "new-draft-secret".into(),
    });
    config.proxy.upstreams.push(upstream);
    let mut save = input(&config, 1);
    save.expected_draft_version = Some(1);
    service.save_draft(save).await.unwrap();
    let workspace = service.workspace().await.unwrap();
    let refreshed = ConfigInput {
        toml: workspace.draft.toml,
        expected_revision: 1,
        expected_draft_version: Some(workspace.draft.version),
        note: "refreshed shared draft".into(),
        use_draft_credentials: true,
    };
    let revision = service.create_revision(refreshed).await.unwrap();
    service
        .activate(ConfigActivate {
            revision: revision.id,
            expected_revision: 1,
            defer_until_restart: false,
        })
        .await
        .unwrap();
    assert_eq!(
        service.current().config.proxy.upstreams[0]
            .credentials
            .as_ref()
            .unwrap()
            .password,
        "new-draft-secret"
    );
    config.proxy.upstreams[0]
        .credentials
        .as_mut()
        .unwrap()
        .password = "rotated-draft-secret".into();
    let mut save = input(&config, revision.id);
    save.expected_draft_version = Some(workspace.draft.version);
    service.save_draft(save).await.unwrap();
    let workspace = service.workspace().await.unwrap();
    let mut refreshed = ConfigInput {
        toml: workspace.draft.toml,
        expected_revision: revision.id,
        expected_draft_version: Some(workspace.draft.version),
        note: "rotated shared draft".into(),
        use_draft_credentials: true,
    };
    let next = service.create_revision(refreshed.clone()).await.unwrap();
    service
        .activate(ConfigActivate {
            revision: next.id,
            expected_revision: revision.id,
            defer_until_restart: false,
        })
        .await
        .unwrap();
    assert_eq!(
        service.current().config.proxy.upstreams[0]
            .credentials
            .as_ref()
            .unwrap()
            .password,
        "rotated-draft-secret"
    );
    refreshed.expected_revision = next.id;
    refreshed.expected_draft_version = Some(1);
    assert!(matches!(
        service.create_revision(refreshed).await,
        Err(ApplicationError::Storage(StorageError::Conflict))
    ));
    service.shutdown().await.unwrap();
}

fn legacy_config() -> DaemonConfig {
    DaemonConfig {
        health: hps_config::HealthConfig {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    }
}
