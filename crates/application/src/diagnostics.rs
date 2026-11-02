//! Read-only, bounded diagnostic work. Purchase/configuration tasks are never replayed.
use super::*;
use hps_domain::{
    DiagnosticBundle, DiagnosticInput, TaskControlInput, TaskControls, TaskInfo, TaskRetryInput,
    VersionInfo,
};
use hps_storage::tasks::{WorkInput, WorkResult};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
const RETENTION: Duration = Duration::from_secs(600);
const CONTROLS: usize = 128;
struct Control {
    cancel: CancellationToken,
    query: hps_domain::ObservationQuery,
    version: VersionInfo,
    revision: i64,
    retry_of: Option<i64>,
    finished: Mutex<Option<Instant>>,
    persistence_failed: std::sync::atomic::AtomicBool,
}
#[derive(Default)]
pub(super) struct Runtime {
    controls: Mutex<BTreeMap<i64, Arc<Control>>>,
    persistence_failures: std::sync::atomic::AtomicU64,
}
impl Runtime {
    fn mark_persistence_failure(&self, control: &Control) {
        control
            .persistence_failed
            .store(true, std::sync::atomic::Ordering::Release);
        self.persistence_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn prune(&self) {
        self.controls.lock().unwrap().retain(|_, c| {
            c.finished
                .lock()
                .unwrap()
                .is_none_or(|at| at.elapsed() <= RETENTION)
        });
    }
    fn get(&self, id: i64) -> Option<Arc<Control>> {
        self.prune();
        self.controls.lock().unwrap().get(&id).cloned()
    }
    fn insert(&self, id: i64, control: Arc<Control>) -> bool {
        self.prune();
        let mut controls = self.controls.lock().unwrap();
        if controls.len() >= CONTROLS {
            let oldest = controls
                .iter()
                .find(|(_, c)| c.finished.lock().unwrap().is_some())
                .map(|(id, _)| *id);
            if let Some(oldest) = oldest {
                controls.remove(&oldest);
            } else {
                return false;
            }
        }
        controls.insert(id, control);
        true
    }
}
struct Job {
    metadata: WorkInput,
    query: hps_domain::ObservationQuery,
    version: VersionInfo,
    revision: i64,
    retry_of: Option<i64>,
}
impl Supervisor {
    async fn collect_diagnostic(
        self: &Arc<Self>,
        query: hps_domain::ObservationQuery,
        version: VersionInfo,
        revision: i64,
    ) -> Result<DiagnosticBundle, ApplicationError> {
        let started = hps_storage::tasks::unix_ms();
        let current = {
            let _operation = self.operation.lock().await;
            self.current()
        };
        if current.revision != revision {
            return Err(ApplicationError::DiagnosticSourceChanged);
        }
        let observed = self.observation_overview(query.clone()).await?;
        let usage = self.store.health_usage().await?;
        let (runs, _, _) = self
            .store
            .health_history(hps_domain::HealthHistoryQuery {
                since_unix: Some(hps_storage::unix_now() - i64::from(query.window_secs)),
                limit: Some(100),
                ..Default::default()
            })
            .await?;
        let mut verdicts = BTreeMap::new();
        for result in runs.iter().filter_map(|r| r.result.as_ref()) {
            let label = serde_json::to_value(result.verdict)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".into());
            *verdicts.entry(label).or_default() += 1;
        }
        let capture = self.linux_capture_status().await;
        if self.current().revision != revision {
            return Err(ApplicationError::DiagnosticSourceChanged);
        }
        Ok(DiagnosticBundle {
            format_version:1,
            version,
            platform:std::env::consts::OS.into(),
            configuration_revision:revision,
            epoch:self.event_epoch().into(),
            collection_started_unix_ms:started,
            collection_finished_unix_ms:hps_storage::tasks::unix_ms(),
            consistent_database_backup:false,
            redaction:"whitelisted_counts_only_no_addresses_names_paths_urls_credentials_or_business_content".into(),
            window_secs:query.window_secs,
            counter_scope:"current_daemon_epoch_cumulative".into(),
            counter_started_unix_ms:observed.statistics.started_at_unix_ms,
            sample_count:observed.samples.len(),
            sample_interval_secs:observed.sample_interval_secs,
            bytes_up:observed.statistics.bytes_up,
            bytes_down:observed.statistics.bytes_down,
            active_connections:observed.statistics.active,
            connection_limit:current.config.proxy.limits.max_connections,
            inbounds_running:current.engine.status().inbounds.iter().filter(|i|i.running).count(),
            health_probes_used:usage.used,
            health_daily_budget:current.config.health.daily_probe_budget,
            recent_probe_verdicts:verdicts,
            recent_probes_examined:runs.len(),
            recent_probe_limit:100,
            error_counts:observed.statistics.errors,
            dns:current.dns.statistics(),
            linux_capture_configured:current.config.linux_capture.is_some(),
            linux_capture_ready:capture.ready,
            unrepresented_closed_flows:observed.correlations.unrepresented_closed_flows,
            rule_counter_overflow_hits:observed.rule_hits.untracked_hits,
            background_result_persistence_failures:self.diagnostics.persistence_failures.load(std::sync::atomic::Ordering::Relaxed),
        })
    }
    pub async fn submit_diagnostic(
        self: &Arc<Self>,
        input: DiagnosticInput,
        version: VersionInfo,
    ) -> Result<TaskInfo, ApplicationError> {
        self.submit_diagnostic_inner(input, version, None, None)
            .await
    }
    async fn submit_diagnostic_inner(
        self: &Arc<Self>,
        input: DiagnosticInput,
        version: VersionInfo,
        retry_of: Option<i64>,
        expected_revision: Option<i64>,
    ) -> Result<TaskInfo, ApplicationError> {
        if input.expected_epoch != self.event_epoch() {
            return Err(ApplicationError::TaskEpoch);
        }
        if !(60..=3600).contains(&input.window_secs) {
            return Err(ApplicationError::ObservationInput);
        }
        let revision = self.current().revision;
        if expected_revision.is_some_and(|expected| expected != revision) {
            return Err(ApplicationError::DiagnosticSourceChanged);
        }
        let query = hps_domain::ObservationQuery {
            window_secs: input.window_secs,
        };
        let metadata = WorkInput {
            kind: "diagnostic_export",
            resource_kind: retry_of.map(|_| "task".into()),
            resource_id: retry_of.map(|id| id.to_string()),
            request_id: input.request_id,
            input_fingerprint: format!(
                "{:x}",
                Sha256::digest(
                    serde_json::to_vec(&(revision, &query, retry_of))
                        .map_err(|_| ApplicationError::TaskInput)?
                )
            ),
            configuration_revision: Some(revision),
        };
        if let Some(task) = self.store.existing_work(metadata.clone()).await? {
            return Ok(task);
        }
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let job = Job {
            metadata,
            query,
            version,
            revision,
            retry_of,
        };
        self.spawn_tracked(
            move |service| async move { service.run_diagnostic_job(job, sender).await },
        )
        .await?;
        receiver.await.map_err(|_| ApplicationError::Worker)?
    }
    async fn run_diagnostic_job(
        self: Arc<Self>,
        job: Job,
        sender: tokio::sync::oneshot::Sender<Result<TaskInfo, ApplicationError>>,
    ) {
        let Job {
            metadata,
            query,
            version,
            revision,
            retry_of,
        } = job;
        let (task, new) = match self.store.begin_work(metadata).await {
            Ok(value) => value,
            Err(error) => {
                let _ = sender.send(Err(error.into()));
                return;
            }
        };
        let id = task.id;
        if !new {
            let _ = sender.send(Ok(task));
            return;
        }
        let control = Arc::new(Control {
            cancel: CancellationToken::new(),
            query: query.clone(),
            version: version.clone(),
            revision,
            retry_of,
            finished: Mutex::new(None),
            persistence_failed: std::sync::atomic::AtomicBool::new(false),
        });
        if !self.diagnostics.insert(id, control.clone()) {
            let _ = self
                .store
                .finish_work(
                    id,
                    WorkResult {
                        state: "failed",
                        error_code: Some("diagnostic_control_capacity"),
                        result_ref: None,
                        result: None,
                    },
                )
                .await;
            let _ = sender.send(Err(ApplicationError::TaskCapacity));
            return;
        }
        let _ = sender.send(Ok(task));
        if let Err(error) = self.store.start_work(id).await {
            if !matches!(error, StorageError::Conflict) {
                self.diagnostics.mark_persistence_failure(&control);
            }
            *control.finished.lock().unwrap() = Some(Instant::now());
            return;
        }
        let service = self.clone();
        let diagnostic = tokio::spawn(async move {
            tokio::time::timeout(
                Duration::from_secs(30),
                service.collect_diagnostic(query, version, revision),
            )
            .await
        });
        let mut diagnostic = AbortDiagnostic(diagnostic);
        let result = tokio::select! {biased;
            _=control.cancel.cancelled()=>None,
            _=self.provider_cancel.cancelled()=>Some(Err("runtime_stopping")),
            result=&mut diagnostic.0=>Some(match result {
                Ok(Ok(Ok(bundle)))=>serde_json::to_value(bundle).map_err(|_|"diagnostic_serialization_failed"),
                Ok(Ok(Err(ApplicationError::DiagnosticSourceChanged)))=>Err("diagnostic_source_changed"),
                Ok(Err(_))=>Err("diagnostic_timeout"),_=>Err("diagnostic_collection_failed"),
            }),
        };
        if let Some(result) = result {
            let outcome = match result {
                Ok(value) => WorkResult {
                    state: "completed",
                    error_code: None,
                    result_ref: Some(format!("diagnostic:{id}")),
                    result: Some(value),
                },
                Err(code) => WorkResult {
                    state: if code == "runtime_stopping" {
                        "interrupted"
                    } else {
                        "failed"
                    },
                    error_code: Some(code),
                    result_ref: None,
                    result: None,
                },
            };
            // A concurrent versioned cancellation may already own the terminal row.
            match self.store.finish_work(id, outcome).await {
                Ok(_) | Err(StorageError::Conflict) => {}
                Err(_) => self.diagnostics.mark_persistence_failure(&control),
            }
        }
        *control.finished.lock().unwrap() = Some(Instant::now());
    }
    pub async fn diagnostic(
        &self,
        id: i64,
        epoch: Option<&str>,
    ) -> Result<DiagnosticBundle, ApplicationError> {
        let task = self.task(id, epoch).await?;
        if task.kind != "diagnostic_export" || task.state != "completed" {
            return Err(ApplicationError::DiagnosticNotReady);
        }
        serde_json::from_value(task.result.ok_or(ApplicationError::DiagnosticNotReady)?)
            .map_err(|_| ApplicationError::TaskPersistence)
    }
    pub async fn task_controls(
        &self,
        id: i64,
        epoch: Option<&str>,
    ) -> Result<TaskControls, ApplicationError> {
        let task = self.task(id, epoch).await?;
        let control = self.diagnostics.get(id);
        let supported = task.kind == "diagnostic_export" && control.is_some();
        let can_cancel = supported && !task.terminal;
        let collection_finished = control
            .as_ref()
            .map_or(task.terminal, |c| c.finished.lock().unwrap().is_some());
        let result_persistence_failed = control.as_ref().is_some_and(|c| {
            c.persistence_failed
                .load(std::sync::atomic::Ordering::Acquire)
        });
        let same_revision = control
            .as_ref()
            .is_some_and(|c| c.revision == self.current().revision);
        let can_retry = supported
            && same_revision
            && matches!(task.state.as_str(), "discarded" | "failed" | "interrupted");
        Ok(TaskControls {
            task_id: id,
            epoch: self.event_epoch().into(),
            version: task.version,
            can_cancel,
            can_retry,
            collection_finished,
            result_persistence_failed,
            reason: if task.kind != "diagnostic_export" {
                "operation_not_a_cancellable_read_only_diagnostic"
            } else if control.is_none() {
                "retry_input_expired_or_daemon_restarted"
            } else if result_persistence_failed {
                "diagnostic_result_persistence_failed_inspect_and_cancel_before_retry"
            } else if !same_revision {
                "diagnostic_source_configuration_changed"
            } else if task.state == "completed" {
                "completed_collect_a_new_snapshot_for_new_data"
            } else {
                "only_read_only_collection_is_cancellable_no_committed_operation_is_rolled_back"
            }
            .into(),
            retry_input_retention_secs: RETENTION.as_secs() as u32,
            retry_of: control.and_then(|c| c.retry_of),
        })
    }
    pub async fn cancel_task(
        &self,
        id: i64,
        input: TaskControlInput,
    ) -> Result<TaskInfo, ApplicationError> {
        let task = self.task(id, Some(&input.expected_epoch)).await?;
        if task.version != input.expected_version {
            return Err(ApplicationError::TaskVersion);
        }
        if task.kind != "diagnostic_export" || task.terminal {
            return Err(ApplicationError::TaskControlUnsupported);
        }
        let control = self
            .diagnostics
            .get(id)
            .ok_or(ApplicationError::TaskRetryExpired)?;
        let result = self
            .store
            .cancel_diagnostic_work(id, input.expected_version)
            .await
            .map_err(|e| {
                if matches!(e, StorageError::Conflict) {
                    ApplicationError::TaskVersion
                } else {
                    e.into()
                }
            })?;
        control.cancel.cancel();
        Ok(result)
    }
    pub async fn retry_task(
        self: &Arc<Self>,
        id: i64,
        input: TaskRetryInput,
    ) -> Result<TaskInfo, ApplicationError> {
        let task = self.task(id, Some(&input.expected_epoch)).await?;
        if task.version != input.expected_version {
            return Err(ApplicationError::TaskVersion);
        }
        if task.kind != "diagnostic_export"
            || !matches!(task.state.as_str(), "discarded" | "failed" | "interrupted")
        {
            return Err(ApplicationError::TaskControlUnsupported);
        }
        let control = self
            .diagnostics
            .get(id)
            .ok_or(ApplicationError::TaskRetryExpired)?;
        if control.revision != self.current().revision {
            return Err(ApplicationError::DiagnosticSourceChanged);
        }
        self.submit_diagnostic_inner(
            DiagnosticInput {
                expected_epoch: input.expected_epoch,
                request_id: input.request_id,
                window_secs: control.query.window_secs,
            },
            control.version.clone(),
            Some(id),
            Some(control.revision),
        )
        .await
    }
}
struct AbortDiagnostic(
    tokio::task::JoinHandle<
        Result<Result<DiagnosticBundle, ApplicationError>, tokio::time::error::Elapsed>,
    >,
);
impl Drop for AbortDiagnostic {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn version() -> VersionInfo {
        VersionInfo {
            service: "honk-proxy-server".into(),
            version: "test".into(),
            api_version: "v1".into(),
            web_build_id: None,
        }
    }
    async fn fixture() -> Arc<Supervisor> {
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        config.instance_name = "never-export-instance-name".into();
        Supervisor::boot(
            Store::open(None, config).unwrap(),
            StartupOverrides::default(),
        )
        .await
        .unwrap()
    }
    async fn terminal(service: &Arc<Supervisor>, id: i64) -> TaskInfo {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let task = service.task(id, None).await.unwrap();
                if task.terminal {
                    break task;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn actual_read_only_job_cancels_while_waiting_for_publication_and_retry_is_idempotent() {
        let service = fixture().await;
        let lock = service.operation.lock().await;
        let job = service
            .submit_diagnostic(
                DiagnosticInput {
                    expected_epoch: service.event_epoch().into(),
                    request_id: "read-only".into(),
                    window_secs: 300,
                },
                version(),
            )
            .await
            .unwrap();
        let controls = service.task_controls(job.id, None).await.unwrap();
        assert!(controls.can_cancel && !controls.can_retry);
        assert!(matches!(
            service
                .cancel_task(
                    job.id,
                    TaskControlInput {
                        expected_epoch: service.event_epoch().into(),
                        expected_version: 0
                    }
                )
                .await,
            Err(ApplicationError::TaskVersion)
        ));
        let cancelled = service
            .cancel_task(
                job.id,
                TaskControlInput {
                    expected_epoch: service.event_epoch().into(),
                    expected_version: controls.version,
                },
            )
            .await
            .unwrap();
        assert_eq!(cancelled.state, "discarded");
        assert_eq!(cancelled.phase, "cancelled");
        assert_eq!(
            cancelled.error_code.as_deref(),
            Some("diagnostic_cancelled")
        );
        drop(lock);
        let input = TaskRetryInput {
            expected_epoch: service.event_epoch().into(),
            expected_version: cancelled.version,
            request_id: "retry".into(),
        };
        let retried = service.retry_task(job.id, input.clone()).await.unwrap();
        assert_eq!(
            service.retry_task(job.id, input).await.unwrap().id,
            retried.id
        );
        assert_ne!(job.id, retried.id);
        assert_eq!(terminal(&service, retried.id).await.state, "completed");
        assert_eq!(
            service
                .task_controls(retried.id, None)
                .await
                .unwrap()
                .retry_of,
            Some(job.id)
        );
        let exported = service.diagnostic(retried.id, None).await.unwrap();
        let json = serde_json::to_string(&exported).unwrap();
        assert!(!json.contains("never-export-instance-name"));
        assert!(!exported.consistent_database_backup);
        assert_eq!(service.store.workspace().await.unwrap().revisions.len(), 1);
        assert!(matches!(
            service
                .cancel_task(
                    retried.id,
                    TaskControlInput {
                        expected_epoch: uuid::Uuid::new_v4().to_string(),
                        expected_version: 1
                    }
                )
                .await,
            Err(ApplicationError::TaskEpoch)
        ));
        service.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn configuration_work_is_never_cancellable_and_changed_or_expired_retry_sources_are_rejected()
     {
        let service = fixture().await;
        let lock = service.operation.lock().await;
        let job = service
            .submit_diagnostic(
                DiagnosticInput {
                    expected_epoch: service.event_epoch().into(),
                    request_id: "source".into(),
                    window_secs: 300,
                },
                version(),
            )
            .await
            .unwrap();
        let controls = service.task_controls(job.id, None).await.unwrap();
        let cancelled = service
            .cancel_task(
                job.id,
                TaskControlInput {
                    expected_epoch: service.event_epoch().into(),
                    expected_version: controls.version,
                },
            )
            .await
            .unwrap();
        let config = service
            .submit_task(hps_domain::TaskSubmission {
                expected_epoch: service.event_epoch().into(),
                request_id: "configuration-still-runs".into(),
                operation: hps_domain::TaskOperation::ConfigurationPrepare(ConfigInput {
                    toml: toml::to_string(&service.current().base_config).unwrap(),
                    expected_revision: 1,
                    expected_draft_version: None,
                    note: "not cancellable".into(),
                    use_draft_credentials: false,
                }),
            })
            .await
            .unwrap();
        let hints = service.task_controls(config.id, None).await.unwrap();
        assert!(!hints.can_cancel && !hints.can_retry);
        assert!(matches!(
            service
                .cancel_task(
                    config.id,
                    TaskControlInput {
                        expected_epoch: service.event_epoch().into(),
                        expected_version: hints.version
                    }
                )
                .await,
            Err(ApplicationError::TaskControlUnsupported)
        ));
        drop(lock);
        assert_eq!(terminal(&service, config.id).await.state, "completed");
        service
            .activate(hps_domain::ConfigActivate {
                revision: 2,
                expected_revision: 1,
                defer_until_restart: false,
            })
            .await
            .unwrap();
        let input = TaskRetryInput {
            expected_epoch: service.event_epoch().into(),
            expected_version: cancelled.version,
            request_id: "stale-retry".into(),
        };
        assert!(matches!(
            service.retry_task(job.id, input.clone()).await,
            Err(ApplicationError::DiagnosticSourceChanged)
        ));
        let control = service.diagnostics.get(job.id).unwrap();
        *control.finished.lock().unwrap() =
            Some(Instant::now() - RETENTION - Duration::from_secs(1));
        assert!(matches!(
            service.retry_task(job.id, input).await,
            Err(ApplicationError::TaskRetryExpired)
        ));
        service.shutdown().await.unwrap();
    }
}
