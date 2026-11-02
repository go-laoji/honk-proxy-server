use hps_config::DaemonConfig;
use hps_domain::{HealthTransport, HealthVerdict, IpFamily, ProbePhase, ProbeResult};
use hps_storage::{StorageError, Store, health::ProbeReservation};
fn probe(node: &str) -> ProbeReservation {
    ProbeReservation {
        node_id: node.into(),
        identity: "identity".into(),
        profile_fingerprint: "profile".into(),
    }
}
fn result() -> ProbeResult {
    ProbeResult {
        node_id: "lease:1".into(),
        identity: "identity".into(),
        profile_fingerprint: "profile".into(),
        transport: HealthTransport::Tcp,
        family: IpFamily::Ipv4,
        verdict: HealthVerdict::Healthy,
        phase: ProbePhase::Complete,
        error_code: None,
        endpoint_reachable: true,
        authenticated: Some(true),
        tunnel_established: true,
        http_status: Some(200),
        target_ok: Some(true),
        latency_ms: 10,
        checked_at: hps_storage::unix_now(),
    }
}
#[tokio::test]
async fn atomic_queue_budget_identity_coalescing_and_restart_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
    let one = store
        .begin_health_task(
            "first".into(),
            "manual".into(),
            vec![probe("lease:1")],
            16,
            2,
        )
        .await
        .unwrap();
    let two = store
        .begin_health_task(
            "second".into(),
            "automatic".into(),
            vec![probe("lease:2")],
            16,
            2,
        )
        .await
        .unwrap();
    assert_eq!(one.runs[0].id, two.runs[0].id);
    assert_eq!(two.merged, 1);
    assert_eq!(store.health_usage().await.unwrap().used, 1);
    assert_eq!(
        store
            .begin_health_task("first".into(), "manual".into(), vec![], 16, 2)
            .await
            .unwrap()
            .id,
        one.id
    );
    let run = one.runs[0].id;
    assert!(store.claim_health_run(run).await.unwrap());
    assert!(!store.claim_health_run(run).await.unwrap());
    store
        .finish_health_run(run, result(), hps_storage::unix_now() + 60, 0)
        .await
        .unwrap();
    let task = store.health_task(two.id).await.unwrap();
    assert_eq!(task.completed, 1);
    assert_eq!(task.healthy, 1);
    let three = store
        .begin_health_task(
            "third".into(),
            "manual".into(),
            vec![probe("lease:3")],
            16,
            2,
        )
        .await
        .unwrap();
    assert!(store.claim_health_run(three.runs[0].id).await.unwrap());
    let mut other = probe("static2");
    other.identity = "other".into();
    assert!(matches!(
        store
            .begin_health_task("over".into(), "manual".into(), vec![other.clone()], 16, 2)
            .await,
        Err(StorageError::HealthCapacity)
    ));
    store.close().await.unwrap();
    let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
    assert_eq!(
        store.health_task(three.id).await.unwrap().state,
        "interrupted"
    );
    assert_eq!(
        store.health_states().await.unwrap()[0].result.checked_at,
        result().checked_at
    );
    assert_eq!(
        store.health_states().await.unwrap()[0].result.verdict,
        HealthVerdict::Unknown
    );
    assert_eq!(store.health_usage().await.unwrap().used, 2);
    assert_eq!(store.health_usage().await.unwrap().probing, 0);
    assert!(matches!(
        store
            .begin_health_task("after-restart".into(), "manual".into(), vec![other], 16, 2)
            .await,
        Err(StorageError::HealthCapacity)
    ));
    store.close().await.unwrap();
}
#[tokio::test]
async fn full_queue_rejects_whole_task_without_reserving_budget() {
    let store = Store::open(None, DaemonConfig::default()).unwrap();
    let probes = (0..17)
        .map(|i| ProbeReservation {
            identity: format!("node{i}"),
            ..probe("test")
        })
        .collect();
    assert!(matches!(
        store
            .begin_health_task("overflow".into(), "manual".into(), probes, 16, 100)
            .await,
        Err(StorageError::HealthCapacity)
    ));
    let usage = store.health_usage().await.unwrap();
    assert_eq!((usage.used, usage.queued), (0, 0));
    assert!(store.health_tasks().await.unwrap().is_empty());
    store.close().await.unwrap();
}
#[test]
fn health_migration_is_reversible() {
    let c = rusqlite::Connection::open_in_memory().unwrap();
    c.execute_batch(include_str!("../migrations/0006_health.up.sql"))
        .unwrap();
    c.execute_batch(include_str!("../migrations/0006_health.down.sql"))
        .unwrap();
    let count: i64 = c
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name LIKE 'health_%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn repeated_joining_of_inflight_runs_cannot_create_unbounded_active_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
    let first = store
        .begin_health_task(
            "first".into(),
            "manual".into(),
            vec![probe("lease:1")],
            16,
            10,
        )
        .await
        .unwrap();
    let c = rusqlite::Connection::open(store.database_path.as_ref().unwrap()).unwrap();
    c.execute_batch("BEGIN;WITH RECURSIVE ids(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ids WHERE n<4999) INSERT INTO health_tasks(request_id,trigger,created_at) SELECT 'join-'||n,'manual',1 FROM ids;INSERT OR IGNORE INTO health_task_runs(task_id,run_id) SELECT id,1 FROM health_tasks;COMMIT;").unwrap();
    assert!(matches!(
        store
            .begin_health_task(
                "another".into(),
                "manual".into(),
                vec![probe("lease:2")],
                16,
                10
            )
            .await,
        Err(StorageError::HealthCapacity)
    ));
    assert_eq!(store.health_usage().await.unwrap().used, 1);
    store.claim_health_run(first.runs[0].id).await.unwrap();
    store
        .finish_health_run(first.runs[0].id, result(), hps_storage::unix_now() + 60, 0)
        .await
        .unwrap();
    store
        .begin_health_task(
            "after-completion".into(),
            "manual".into(),
            vec![probe("lease:3")],
            16,
            10,
        )
        .await
        .unwrap();
    let tasks: u32 = c
        .query_row("SELECT count(*) FROM health_tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tasks, 5000);
    drop(c);
    store.close().await.unwrap();
}
