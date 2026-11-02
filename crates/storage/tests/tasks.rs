use hps_config::{DaemonConfig, ProviderConfig, ProviderKind};
use hps_domain::{EventQuery, TaskQuery};
use hps_storage::{
    StorageError, Store,
    health::ProbeReservation,
    tasks::{ManagementEvent, WorkInput, WorkResult},
};

fn work(request: &str) -> WorkInput {
    WorkInput {
        kind: "configuration_prepare",
        resource_kind: Some("configuration".into()),
        resource_id: Some("1".into()),
        request_id: request.into(),
        input_fingerprint: "a".repeat(64),
        configuration_revision: Some(1),
    }
}
fn done() -> WorkResult {
    WorkResult {
        state: "completed",
        error_code: None,
        result_ref: Some("config:2".into()),
        result: Some(serde_json::json!({"valid":true})),
    }
}

#[tokio::test]
async fn idempotent_work_has_one_monotonic_history_and_bounded_safe_results() {
    let store = Store::open(None, DaemonConfig::default()).unwrap();
    let (task, new) = store.begin_work(work("one")).await.unwrap();
    assert!(new);
    assert_eq!(task.state, "queued");
    assert_eq!(store.begin_work(work("one")).await.unwrap().0.id, task.id);
    assert!(!store.begin_work(work("one")).await.unwrap().1);
    let mut changed = work("one");
    changed.input_fingerprint = "b".repeat(64);
    assert!(matches!(
        store.begin_work(changed).await,
        Err(StorageError::Conflict)
    ));
    let running = store.start_work(task.id).await.unwrap();
    assert_eq!(running.version, task.version + 1);
    let complete = store.finish_work(task.id, done()).await.unwrap();
    assert_eq!(complete.version, running.version + 1);
    assert!(complete.terminal);
    assert_eq!(complete.result.as_ref().unwrap()["valid"], true);
    let list = store.tasks(TaskQuery::default()).await.unwrap();
    assert_eq!(list.records.len(), 1);
    assert!(list.records[0].result.is_none());
    assert_eq!(store.task(task.id).await.unwrap().result, complete.result);
    let events = store.events(EventQuery::default()).await.unwrap();
    assert_eq!(
        events
            .records
            .iter()
            .map(|e| e.code.as_str())
            .collect::<Vec<_>>(),
        vec!["task_queued", "task_running", "task_completed"]
    );
    assert!(
        events
            .records
            .iter()
            .all(|e| e.task_id == Some(task.id) && e.recorded_epoch == store.epoch)
    );
    assert!(matches!(
        store.finish_work(task.id, done()).await,
        Err(StorageError::Conflict)
    ));
    let (large, _) = store.begin_work(work("large")).await.unwrap();
    let mut result = done();
    result.result = Some(serde_json::json!({"oversized":"x".repeat(65536)}));
    assert!(matches!(
        store.finish_work(large.id, result).await,
        Err(StorageError::Invalid)
    ));
    store.finish_work(large.id, done()).await.unwrap();
    store.close().await.unwrap();
}

#[tokio::test]
async fn source_tasks_project_actual_provider_health_and_activation_without_credentials() {
    let store = Store::open(None, DaemonConfig::default()).unwrap();
    let provider = ProviderConfig {
        id: "source".into(),
        enabled: true,
        kind: ProviderKind::Text,
        endpoint: "http://127.0.0.1/list?token=private-provider-token".into(),
        ttl_secs: Some(300),
        ..Default::default()
    };
    let (source, _) = store
        .begin_extraction(
            provider.clone(),
            "extract-one".into(),
            hps_provider::fingerprint(&provider),
        )
        .await
        .unwrap();
    store
        .annotate_task_source(format!("provider:{}", source.id), 1)
        .await
        .unwrap();
    store.extraction_requesting(source.id).await.unwrap();
    store
        .fail_extraction(
            source.id,
            hps_provider::FetchError {
                code: "transport_error",
                outcome_unknown: true,
                retry_after_secs: None,
            },
        )
        .await
        .unwrap();
    let health = store
        .begin_health_task(
            "health-one".into(),
            "manual".into(),
            vec![ProbeReservation {
                node_id: "lease:1".into(),
                identity: "identity".into(),
                profile_fingerprint: "profile".into(),
            }],
            8,
            100,
        )
        .await
        .unwrap();
    store
        .annotate_task_source(format!("health:{}", health.id), 1)
        .await
        .unwrap();
    let before = store
        .tasks(TaskQuery {
            kind: Some("health".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .records
        .remove(0);
    assert_eq!(before.phase, "queued");
    assert_eq!(before.total, 1);
    assert!(!before.terminal);
    assert!(store.claim_health_run(health.runs[0].id).await.unwrap());
    let probing = store.task(before.id).await.unwrap();
    assert!(probing.version > before.version);
    assert_eq!(probing.phase, "probing");
    store.discard_health_run(health.runs[0].id).await.unwrap();
    let ended = store.task(before.id).await.unwrap();
    assert!(ended.terminal);
    assert_eq!(ended.completed, 1);
    assert_eq!(ended.skipped, 1);
    assert_eq!(ended.configuration_revision, Some(1));
    let empty = store
        .begin_health_task("empty".into(), "manual".into(), vec![], 8, 100)
        .await
        .unwrap();
    let records = store.tasks(TaskQuery::default()).await.unwrap().records;
    assert!(records.iter().any(|t| t.source_ref.as_deref()
        == Some(&format!("health:{}", empty.id))
        && t.terminal
        && t.total == 0));
    let provider = records.iter().find(|t| t.kind == "provider").unwrap();
    let exact = store
        .tasks(TaskQuery {
            source_ref: Some(format!("provider:{}", source.id)),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(exact.records.len(), 1);
    assert_eq!(exact.records[0].id, provider.id);
    assert!(matches!(
        store
            .tasks(TaskQuery {
                source_ref: Some("provider:01".into()),
                ..Default::default()
            })
            .await,
        Err(StorageError::Invalid)
    ));
    assert_eq!(provider.state, "outcome_unknown");
    assert_eq!(provider.configuration_revision, Some(1));
    let activation = store.activate(1, 1, false).await.unwrap();
    let a = store
        .tasks(TaskQuery {
            kind: Some("activation".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .records
        .remove(0);
    assert_eq!(a.phase, "prepared");
    assert!(!a.terminal);
    store.mark_applied(1, Some(activation)).await.unwrap();
    let a = store.task(a.id).await.unwrap();
    assert_eq!(a.state, "completed");
    assert!(a.terminal);
    assert!(a.finished_at_unix_ms.is_some());
    let all = store.events(EventQuery::default()).await.unwrap();
    assert!(
        !serde_json::to_string(&all)
            .unwrap()
            .contains("private-provider-token")
    );
    let health_events = store
        .events(EventQuery {
            node_id: Some("lease:1".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(health_events.records.len(), 2);
    assert!(
        health_events
            .records
            .iter()
            .all(|e| e.task_id == Some(before.id) && e.configuration_revision == Some(1))
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn management_metadata_has_actual_peer_and_rejects_raw_url_resource_values() {
    let store = Store::open(None, DaemonConfig::default()).unwrap();
    let input = |resource: &str| ManagementEvent {
        request_id: "request-one".into(),
        peer: Some("127.0.0.1:49152".parse().unwrap()),
        action: "provider_save",
        outcome: "succeeded",
        resource_kind: Some("provider".into()),
        resource_id: Some(resource.into()),
        node_id: None,
        provider_id: Some("source".into()),
        configuration_revision: Some(1),
    };
    assert!(matches!(
        store
            .record_management_event(input("https://host?token=secret"))
            .await,
        Err(StorageError::Invalid)
    ));
    store
        .record_management_event(input("source"))
        .await
        .unwrap();
    let events = store
        .events(EventQuery {
            request_id: Some("request-one".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(events.records.len(), 1);
    assert_eq!(events.records[0].peer.as_deref(), Some("127.0.0.1:49152"));
    assert_eq!(events.records[0].action.as_deref(), Some("provider_save"));
    assert_eq!(events.records[0].provider_id.as_deref(), Some("source"));
    store.close().await.unwrap();
}

#[tokio::test]
async fn restart_marks_owned_work_interrupted_and_old_epoch_cursor_resets_without_relabeling_history()
 {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
    let (task, _) = store.begin_work(work("unfinished")).await.unwrap();
    store.start_work(task.id).await.unwrap();
    let old = store.events(EventQuery::default()).await.unwrap();
    store.close().await.unwrap();
    let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
    assert_ne!(store.epoch, old.epoch);
    let interrupted = store.task(task.id).await.unwrap();
    assert_eq!(interrupted.state, "interrupted");
    assert!(interrupted.terminal);
    let new = store
        .events(EventQuery {
            cursor: Some(old.cursor),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(new.reset_reason.as_deref(), Some("instance_changed"));
    assert_eq!(new.records.last().unwrap().recorded_epoch, store.epoch);
    assert_eq!(new.records.first().unwrap().recorded_epoch, old.epoch);
    assert!(matches!(
        store
            .tasks(TaskQuery {
                expected_epoch: Some(old.epoch),
                ..Default::default()
            })
            .await,
        Err(StorageError::Conflict)
    ));
    store.close().await.unwrap();
}

#[tokio::test]
async fn work_capacity_events_retention_pagination_and_filtered_cursors_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
    let mut ids = vec![];
    for n in 0..64 {
        ids.push(
            store
                .begin_work(work(&format!("work-{n}")))
                .await
                .unwrap()
                .0
                .id,
        );
    }
    assert!(matches!(
        store.begin_work(work("overflow")).await,
        Err(StorageError::TaskCapacity)
    ));
    for id in ids {
        store.finish_work(id, done()).await.unwrap();
    }
    let first = store
        .tasks(TaskQuery {
            limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    let second = store
        .tasks(TaskQuery {
            limit: Some(1),
            before: first.next_before,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(first.records[0].id > second.records[0].id);
    let old = store.events(EventQuery::default()).await.unwrap();
    let db = rusqlite::Connection::open(dir.path().join("hps.sqlite3")).unwrap();
    db.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<10100) INSERT INTO service_events(recorded_epoch,occurred_at,level,module,code) SELECT (SELECT epoch FROM event_context),unixepoch()*1000,'info','runtime','bounded_fixture' FROM n").unwrap();
    let reset = store
        .events(EventQuery {
            cursor: Some(old.cursor),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(reset.reset_reason.as_deref(), Some("cursor_expired"));
    assert_eq!(
        db.query_row("SELECT count(*) FROM service_events", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        10000
    );
    let start = hps_domain::event_cursor(&store.epoch, reset.oldest_available_sequence - 1);
    let one = store
        .events(EventQuery {
            cursor: Some(start),
            limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(one.records.len(), 1);
    assert!(one.has_more);
    let two = store
        .events(EventQuery {
            cursor: Some(one.cursor.clone()),
            limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(two.records[0].sequence, one.records[0].sequence + 1);
    let excluded = store
        .events(EventQuery {
            cursor: Some(one.cursor),
            module: Some("dns".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(excluded.records.is_empty());
    assert!(!excluded.has_more);
    assert_eq!(
        excluded.cursor,
        hps_domain::event_cursor(&store.epoch, excluded.head_sequence)
    );
    assert!(matches!(
        store
            .events(EventQuery {
                cursor: Some(format!("{}:01", store.epoch)),
                ..Default::default()
            })
            .await,
        Err(StorageError::Invalid)
    ));
    store.close().await.unwrap();
}

#[tokio::test]
async fn task_event_migration_is_reversible_without_dropping_source_facts() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
    store.close().await.unwrap();
    let db = rusqlite::Connection::open(dir.path().join("hps.sqlite3")).unwrap();
    db.busy_timeout(std::time::Duration::from_secs(3)).unwrap();
    db.execute_batch(include_str!("../migrations/0008_tasks_events.down.sql"))
        .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM config_revisions", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        1
    );
    db.execute("INSERT INTO health_tasks(request_id,trigger,created_at) VALUES ('old-health','manual',100)", []).unwrap();
    db.execute("INSERT INTO provider_tasks(provider_id,request_id,fingerprint,state,requested_count,started_at,finished_at) VALUES ('source','new-provider',?1,'succeeded',1,200,201)", ["a".repeat(64)]).unwrap();
    db.execute_batch(include_str!("../migrations/0008_tasks_events.up.sql"))
        .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM task_records", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        2
    );
    let sources = db
        .prepare("SELECT source_ref FROM task_records ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(sources, ["health:1", "provider:1"]);
}
