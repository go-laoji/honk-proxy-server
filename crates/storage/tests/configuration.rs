use hps_config::DaemonConfig;
use hps_storage::{StorageError, Store};

#[tokio::test]
async fn revisions_survive_restart_lock_is_exclusive_and_stale_writes_conflict() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(Some(directory.path()), DaemonConfig::default()).unwrap();
    assert!(matches!(
        Store::open(Some(directory.path()), DaemonConfig::default()),
        Err(StorageError::AlreadyRunning)
    ));
    let state = store.workspace().await.unwrap();
    assert_eq!(state.desired.revision.id, 1);
    assert!(state.last_applied.is_none());
    let mut config = state.desired.config;
    config.instance_name = "new-instance".into();
    store.save_draft(1, 1, config.clone()).await.unwrap();
    assert!(matches!(
        store.save_draft(1, 1, config.clone()).await,
        Err(StorageError::Conflict)
    ));
    let revision = store
        .create_revision(1, config, "validated change".into())
        .await
        .unwrap();
    let activation = store.activate(revision.id, 1, false).await.unwrap();
    store
        .mark_applied(revision.id, Some(activation))
        .await
        .unwrap();
    assert!(matches!(
        store.activate(1, 1, false).await,
        Err(StorageError::Conflict)
    ));
    store.close().await.unwrap();
    let restored = Store::open(Some(directory.path()), DaemonConfig::default()).unwrap();
    let state = restored.workspace().await.unwrap();
    assert_eq!(state.desired.revision.id, revision.id);
    assert_eq!(state.desired.config.instance_name, "new-instance");
    assert_eq!(state.last_applied, Some(revision.id));
    assert_eq!(state.activations[0].state, "applied");
    let rollback = restored.activate(1, revision.id, false).await.unwrap();
    restored.mark_applied(1, Some(rollback)).await.unwrap();
    assert_eq!(restored.workspace().await.unwrap().desired.revision.id, 1);
    restored.close().await.unwrap();
}
#[tokio::test]
async fn simultaneous_edit_has_one_winner_and_interrupted_activation_is_not_applied() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(Some(directory.path()), DaemonConfig::default()).unwrap();
    let a = store.clone();
    let b = store.clone();
    let (a, b) = tokio::join!(
        a.save_draft(1, 1, DaemonConfig::default()),
        b.save_draft(1, 1, DaemonConfig::default())
    );
    assert!(a.is_ok() ^ b.is_ok());
    let revision = store
        .create_revision(1, DaemonConfig::default(), "candidate".into())
        .await
        .unwrap();
    store.activate(revision.id, 1, false).await.unwrap();
    store.close().await.unwrap();
    let restored = Store::open(Some(directory.path()), DaemonConfig::default()).unwrap();
    let state = restored.workspace().await.unwrap();
    assert_eq!(state.activations[0].state, "interrupted");
    assert!(state.last_applied.is_none());
    restored.close().await.unwrap();
}
#[test]
fn reversible_empty_schema_and_newer_schema_rejection() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch(include_str!("../migrations/0001_configuration.up.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0002_known_good.up.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0003_provider_leases.up.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0003_provider_leases.down.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0002_known_good.down.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0001_configuration.down.sql"))
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let connection = rusqlite::Connection::open(directory.path().join("hps.sqlite3")).unwrap();
    connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at INTEGER NOT NULL);INSERT INTO schema_migrations VALUES(99,0)").unwrap();
    drop(connection);
    assert!(matches!(
        Store::open(Some(directory.path()), DaemonConfig::default()),
        Err(StorageError::SchemaTooNew)
    ));
}
