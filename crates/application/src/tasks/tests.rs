use super::*;
use std::time::Duration;

fn input(config: &DaemonConfig) -> ConfigInput {
    ConfigInput {
        toml: toml::to_string(config).unwrap(),
        expected_revision: 1,
        expected_draft_version: None,
        note: "private-note-value".into(),
        use_draft_credentials: false,
    }
}
async fn fixture() -> Arc<Supervisor> {
    let store = Store::open(None, DaemonConfig::default()).unwrap();
    Supervisor::boot(store, StartupOverrides::default())
        .await
        .unwrap()
}
async fn terminal(service: &Arc<Supervisor>, id: i64) -> TaskInfo {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let task = service.task(id, None).await.unwrap();
            if task.terminal {
                return task;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn public_preparation_survives_lost_wait_and_submission_is_idempotent_without_storing_input()
{
    let service = fixture().await;
    let lock = service.operation.lock().await;
    let config = input(&service.current().base_config);
    assert!(!format!("{config:?}").contains("private-note-value"));
    let caller_service = service.clone();
    let caller = tokio::spawn(async move { caller_service.create_revision(config).await });
    let task = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let records = service.tasks(TaskQuery::default()).await.unwrap().records;
            if let Some(t) = records.first() {
                return t.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    caller.abort();
    drop(lock);
    let complete = terminal(&service, task.id).await;
    assert_eq!(complete.state, "completed");
    assert_eq!(complete.result_ref.as_deref(), Some("config:2"));
    assert!(
        !serde_json::to_string(&complete)
            .unwrap()
            .contains("private-note-value")
    );
    let submission = TaskSubmission {
        expected_epoch: service.event_epoch().to_owned(),
        request_id: "recoverable-prepare".into(),
        operation: TaskOperation::ConfigurationPrepare(input(&service.current().base_config)),
    };
    let mut stale = submission.clone();
    stale.expected_epoch = uuid::Uuid::new_v4().to_string();
    assert!(matches!(
        service.submit_task(stale).await,
        Err(ApplicationError::TaskEpoch)
    ));
    assert_eq!(
        service
            .tasks(TaskQuery::default())
            .await
            .unwrap()
            .records
            .len(),
        1
    );
    let one = service.submit_task(submission.clone()).await.unwrap();
    let two = service.submit_task(submission.clone()).await.unwrap();
    assert_eq!(one.id, two.id);
    let done = terminal(&service, one.id).await;
    assert_eq!(done.result_ref.as_deref(), Some("config:3"));
    let three = service.submit_task(submission.clone()).await.unwrap();
    assert_eq!(three.id, one.id);
    let mut changed_revision = submission.clone();
    let TaskOperation::ConfigurationPrepare(ref mut config) = changed_revision.operation else {
        unreachable!()
    };
    config.expected_revision = 2;
    assert!(matches!(
        service.submit_task(changed_revision).await,
        Err(ApplicationError::Storage(StorageError::Conflict))
    ));
    assert!(matches!(
        service
            .submit_task(TaskSubmission {
                expected_epoch: service.event_epoch().to_owned(),
                request_id: submission.request_id.clone(),
                operation: TaskOperation::ConfigurationValidate(input(
                    &service.current().base_config
                ))
            })
            .await,
        Err(ApplicationError::Storage(StorageError::Conflict))
    ));
    let full = service
        .work_slots
        .clone()
        .acquire_many_owned(16)
        .await
        .unwrap();
    assert_eq!(
        service.submit_task(submission.clone()).await.unwrap().id,
        one.id
    );
    drop(full);
    let mut changed = submission;
    let TaskOperation::ConfigurationPrepare(ref mut config) = changed.operation else {
        unreachable!()
    };
    config.note = "different".into();
    assert!(matches!(
        service.submit_task(changed).await,
        Err(ApplicationError::Storage(StorageError::Conflict))
    ));
    assert_eq!(service.store.workspace().await.unwrap().revisions.len(), 3);
    service.shutdown().await.unwrap();
}

struct Release(Option<std::sync::mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[tokio::test]
async fn blocking_preflight_keeps_capacity_lock_and_shutdown_ownership_after_caller_abort() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
    let service = Supervisor::boot(store, StartupOverrides::default())
        .await
        .unwrap();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release = Release(Some(release_tx));
    let config = service.current().base_config.clone();
    let metadata = work_spec(
        "configuration_prepare",
        &config,
        Some(1),
        Some("blocking-preflight".into()),
    )
    .unwrap();
    let worker = service.clone();
    let caller = tokio::spawn(async move {
        worker
            .owned_work(
                metadata,
                move |s| async move {
                    let _operation = s.operation.lock().await;
                    let preparing = config.clone();
                    tokio::task::spawn_blocking(move || {
                        let _ = entered_tx.send(());
                        let _ = release_rx.recv();
                        prepare_engine(&preparing, None)
                    })
                    .await
                    .map_err(|_| ApplicationError::Worker)??;
                    Ok(revision_view(
                        s.store
                            .create_revision(1, config, "owned preflight".into())
                            .await?,
                    ))
                },
                revision_result,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), entered_rx)
        .await
        .unwrap()
        .unwrap();
    let other = service
        .work_slots
        .clone()
        .acquire_many_owned(15)
        .await
        .unwrap();
    caller.abort();
    assert_eq!(service.work_slots.available_permits(), 0);
    assert!(service.operation.try_lock().is_err());
    assert!(matches!(
        service
            .submit_task(TaskSubmission {
                expected_epoch: service.event_epoch().to_owned(),
                request_id: "full".into(),
                operation: TaskOperation::ConfigurationPrepare(input(
                    &service.current().base_config
                ))
            })
            .await,
        Err(ApplicationError::TaskCapacity)
    ));
    let stopping = service.clone();
    let shutdown = tokio::spawn(async move { stopping.shutdown().await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!shutdown.is_finished());
    drop(release);
    drop(other);
    tokio::time::timeout(Duration::from_secs(3), shutdown)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("hps.sqlite3")).unwrap();
    let (state, reference): (String, String) = db
        .query_row(
            "SELECT state,result_ref FROM task_records WHERE request_id='blocking-preflight'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "completed");
    assert_eq!(reference, "config:2");
    assert!(matches!(
        service
            .submit_task(TaskSubmission {
                expected_epoch: service.event_epoch().to_owned(),
                request_id: "after-close".into(),
                operation: TaskOperation::ConfigurationPrepare(input(
                    &service.current().base_config
                ))
            })
            .await,
        Err(ApplicationError::Engine(EngineError::ShuttingDown))
    ));
}

#[tokio::test]
async fn diagnostic_panic_has_fixed_persisted_terminal_and_releases_capacity() {
    let service = fixture().await;
    let metadata = work_spec(
        "rules_diagnostic",
        &"panic-fixture",
        Some(1),
        Some("panic-fixture".into()),
    )
    .unwrap();
    let result: Result<(), ApplicationError> = service
        .owned_work(
            metadata,
            |_| async {
                panic!("intentional worker failure");
            },
            |_| WorkResult {
                state: "completed",
                error_code: None,
                result_ref: None,
                result: None,
            },
        )
        .await;
    assert!(matches!(result, Err(ApplicationError::Worker)));
    let row = service
        .tasks(TaskQuery {
            request_id: Some("panic-fixture".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .records
        .remove(0);
    assert_eq!(row.state, "failed");
    assert_eq!(
        row.error_code.as_deref(),
        Some("background_operation_failed")
    );
    assert!(
        !serde_json::to_string(&row)
            .unwrap()
            .contains("intentional worker failure")
    );
    assert_eq!(service.work_slots.available_permits(), 16);
    service.shutdown().await.unwrap();
}
