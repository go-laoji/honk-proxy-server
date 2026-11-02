use super::*;
use hps_domain::{
    EventQuery, EventsOverview, TaskInfo, TaskOperation, TaskQuery, TaskSubmission, TasksOverview,
};
use hps_storage::tasks::{WorkInput, WorkResult};
use sha2::{Digest, Sha256};
use std::future::Future;
use tokio::sync::oneshot;

#[derive(Clone)]
pub struct RequestContext {
    pub request_id: String,
    pub peer: Option<std::net::SocketAddr>,
}
tokio::task_local! {static REQUEST_CONTEXT:RequestContext;}
pub fn current_request_id() -> Option<String> {
    REQUEST_CONTEXT.try_with(|c| c.request_id.clone()).ok()
}
pub async fn with_request_context<T>(
    context: RequestContext,
    future: impl Future<Output = T>,
) -> T {
    REQUEST_CONTEXT.scope(context, future).await
}
async fn scoped<T>(context: Option<RequestContext>, future: impl Future<Output = T>) -> T {
    if let Some(c) = context {
        with_request_context(c, future).await
    } else {
        future.await
    }
}

struct WorkHandle<T> {
    task: TaskInfo,
    completion: oneshot::Receiver<Result<T, ApplicationError>>,
}
fn failure(error: &ApplicationError) -> &'static str {
    match error {
        ApplicationError::Storage(StorageError::Conflict) => "configuration_changed",
        ApplicationError::Storage(StorageError::NotFound) => "configuration_not_found",
        ApplicationError::Invalid => "configuration_invalid",
        ApplicationError::Engine(EngineError::ShuttingDown) => "runtime_stopping",
        ApplicationError::Engine(EngineError::Bind) => "listen_preparation_failed",
        ApplicationError::RouteNodeUnavailable => "routing_node_unavailable",
        ApplicationError::RuleDiagnosticsCapacity => "rule_diagnostics_capacity",
        ApplicationError::Provider(error) => error.code,
        ApplicationError::Dns(_) => "dns_diagnostic_failed",
        _ => "background_operation_failed",
    }
}
fn fingerprint<T: serde::Serialize>(input: &T) -> Result<String, ApplicationError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(input).map_err(|_| ApplicationError::TaskInput)?)
    ))
}
pub(super) fn work_spec<T: serde::Serialize>(
    kind: &'static str,
    input: &T,
    revision: Option<i64>,
    request_id: Option<String>,
) -> Result<WorkInput, ApplicationError> {
    Ok(WorkInput {
        kind,
        resource_kind: revision.map(|_| "configuration".into()),
        resource_id: revision.map(|r| r.to_string()),
        request_id: request_id
            .or_else(current_request_id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        input_fingerprint: fingerprint(&(kind, revision, input))?,
        configuration_revision: revision,
    })
}
fn validation_result(value: &ConfigValidation) -> WorkResult {
    WorkResult {
        state: if value.valid { "completed" } else { "failed" },
        error_code: if value.valid {
            None
        } else {
            Some("configuration_invalid")
        },
        result_ref: None,
        result: Some(
            serde_json::json!({"valid":value.valid,"diagnostic_count":value.diagnostics.len(),"change_count":value.changes.len(),"restart_required":value.restart_required}),
        ),
    }
}
fn revision_result(value: &ConfigRevision) -> WorkResult {
    WorkResult {
        state: "completed",
        error_code: None,
        result_ref: Some(format!("config:{}", value.id)),
        result: Some(serde_json::json!({"revision":value.id})),
    }
}

impl Supervisor {
    pub async fn diagnose_dns(
        self: &Arc<Self>,
        input: hps_dns::LookupInput,
    ) -> Result<hps_dns::LookupResult, ApplicationError> {
        let current = self.current();
        let metadata = work_spec("dns_diagnostic", &input, Some(current.revision), None)?;
        self.owned_work(metadata,move|_|async move {Ok(current.dns.diagnose(input).await?)},|value|WorkResult {
            state:"completed",error_code:None,result_ref:None,
            result:Some(serde_json::json!({"upstream":value.upstream,"response_code":value.response_code,"source":value.source,"elapsed_ms":value.elapsed_ms,"answer_count":value.answers.len(),"remaining_ttl":value.remaining_ttl})),
        }).await
    }
    /// Registration and shutdown use the same mutex boundary. Dropping the caller only drops its wait.
    async fn spawn_tracked<T, F, Fut>(
        self: &Arc<Self>,
        work: F,
    ) -> Result<tokio::task::JoinHandle<T>, ApplicationError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<Self>) -> Fut + Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let permit = self
            .work_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApplicationError::TaskCapacity)?;
        let registration = self.work_registration.lock().unwrap();
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::ShuttingDown.into());
        }
        let service = self.clone();
        let context = REQUEST_CONTEXT.try_with(Clone::clone).ok();
        let task = self.provider_tasks.spawn(async move {
            let _permit = permit;
            scoped(context, work(service)).await
        });
        drop(registration);
        Ok(task)
    }
    pub(super) async fn tracked<T, F, Fut>(self: &Arc<Self>, work: F) -> Result<T, ApplicationError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<Self>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, ApplicationError>> + Send + 'static,
    {
        self.spawn_tracked(work)
            .await?
            .await
            .map_err(|_| ApplicationError::Worker)?
    }
    async fn accept_work<T, F, Fut, D>(
        self: &Arc<Self>,
        input: WorkInput,
        work: F,
        describe: D,
    ) -> Result<WorkHandle<T>, ApplicationError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<Self>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, ApplicationError>> + Send + 'static,
        D: FnOnce(&T) -> WorkResult + Send + 'static,
    {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::ShuttingDown.into());
        }
        if let Some(task) = self.store.existing_work(input.clone()).await? {
            let (tx, completion) = oneshot::channel();
            let _ = tx.send(Err(ApplicationError::TaskExisting(task.id)));
            return Ok(WorkHandle { task, completion });
        }
        let (accepted_tx, accepted_rx) = oneshot::channel::<Result<TaskInfo, ApplicationError>>();
        let (completed_tx, completion) = oneshot::channel();
        self.spawn_tracked(move |service| async move {
            let (task, new) = match service.store.begin_work(input).await {
                Ok(accepted) => accepted,
                Err(error) => {
                    let _ = accepted_tx.send(Err(error.into()));
                    return;
                }
            };
            let id = task.id;
            let _ = accepted_tx.send(Ok(task));
            if !new {
                let _ = completed_tx.send(Err(ApplicationError::TaskExisting(id)));
                return;
            }
            if let Err(error) = service.store.start_work(id).await {
                let _ = completed_tx.send(Err(error.into()));
                return;
            }
            // The tracked parent retains its permit and joins even if work panics.
            let context = REQUEST_CONTEXT.try_with(Clone::clone).ok();
            let result = tokio::spawn(scoped(context, work(service.clone())))
                .await
                .unwrap_or(Err(ApplicationError::Worker));
            let outcome = match &result {
                Ok(value) => describe(value),
                Err(error) => WorkResult {
                    state: if matches!(error, ApplicationError::Engine(EngineError::ShuttingDown)) {
                        "interrupted"
                    } else {
                        "failed"
                    },
                    error_code: Some(failure(error)),
                    result_ref: None,
                    result: None,
                },
            };
            if service.store.finish_work(id, outcome).await.is_err() {
                let _ = completed_tx.send(Err(ApplicationError::TaskPersistence));
                return;
            }
            let _ = completed_tx.send(result);
        })
        .await?;
        let task = accepted_rx.await.map_err(|_| ApplicationError::Worker)??;
        Ok(WorkHandle { task, completion })
    }
    pub(super) async fn owned_work<T, F, Fut, D>(
        self: &Arc<Self>,
        input: WorkInput,
        work: F,
        describe: D,
    ) -> Result<T, ApplicationError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<Self>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, ApplicationError>> + Send + 'static,
        D: FnOnce(&T) -> WorkResult + Send + 'static,
    {
        self.accept_work(input, work, describe)
            .await?
            .completion
            .await
            .map_err(|_| ApplicationError::Worker)?
    }
    pub async fn submit_task(
        self: &Arc<Self>,
        input: TaskSubmission,
    ) -> Result<TaskInfo, ApplicationError> {
        if !hps_domain::valid_connection_epoch(&input.expected_epoch) {
            return Err(ApplicationError::TaskInput);
        }
        if input.expected_epoch != self.store.epoch {
            return Err(ApplicationError::TaskEpoch);
        }
        match input.operation {
            TaskOperation::ConfigurationValidate(config) => {
                let input = work_spec(
                    "configuration_validate",
                    &config,
                    Some(config.expected_revision),
                    Some(input.request_id),
                )?;
                self.accept_work(
                    input,
                    move |s| async move { Ok(s.validate_inner(config).await) },
                    validation_result,
                )
                .await
                .map(|h| h.task)
            }
            TaskOperation::ConfigurationPrepare(config) => {
                let input = work_spec(
                    "configuration_prepare",
                    &config,
                    Some(config.expected_revision),
                    Some(input.request_id),
                )?;
                self.accept_work(
                    input,
                    move |s| async move { s.create_revision_inner(config).await },
                    revision_result,
                )
                .await
                .map(|h| h.task)
            }
        }
    }
    pub(super) async fn validate_owned(
        self: &Arc<Self>,
        input: ConfigInput,
    ) -> Result<ConfigValidation, ApplicationError> {
        let metadata = work_spec(
            "configuration_validate",
            &input,
            Some(input.expected_revision),
            None,
        )?;
        self.accept_work(
            metadata,
            move |s| async move { Ok(s.validate_inner(input).await) },
            validation_result,
        )
        .await?
        .completion
        .await
        .map_err(|_| ApplicationError::Worker)?
    }
    pub(super) async fn create_revision_owned(
        self: &Arc<Self>,
        input: ConfigInput,
    ) -> Result<ConfigRevision, ApplicationError> {
        let metadata = work_spec(
            "configuration_prepare",
            &input,
            Some(input.expected_revision),
            None,
        )?;
        self.accept_work(
            metadata,
            move |s| async move { s.create_revision_inner(input).await },
            revision_result,
        )
        .await?
        .completion
        .await
        .map_err(|_| ApplicationError::Worker)?
    }
    pub async fn tasks(&self, query: TaskQuery) -> Result<TasksOverview, ApplicationError> {
        if query
            .expected_epoch
            .as_ref()
            .is_some_and(|e| e != &self.store.epoch)
        {
            return Err(ApplicationError::TaskEpoch);
        }
        self.store.tasks(query).await.map_err(|e| {
            if matches!(e, StorageError::Invalid) {
                ApplicationError::TaskInput
            } else {
                e.into()
            }
        })
    }
    pub async fn task(
        &self,
        id: i64,
        expected_epoch: Option<&str>,
    ) -> Result<TaskInfo, ApplicationError> {
        if expected_epoch.is_some_and(|e| e != self.store.epoch) {
            return Err(ApplicationError::TaskEpoch);
        }
        self.store.task(id).await.map_err(|e| match e {
            StorageError::NotFound => ApplicationError::TaskUnavailable,
            StorageError::Invalid => ApplicationError::TaskInput,
            _ => e.into(),
        })
    }
    pub async fn events(&self, query: EventQuery) -> Result<EventsOverview, ApplicationError> {
        self.store.events(query).await.map_err(|e| {
            if matches!(e, StorageError::Invalid) {
                ApplicationError::TaskInput
            } else {
                e.into()
            }
        })
    }
    pub fn reserve_event_stream(
        &self,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, ApplicationError> {
        self.event_stream_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApplicationError::TaskCapacity)
    }
    pub fn event_epoch(&self) -> &str {
        &self.store.epoch
    }
    pub async fn record_management_event(
        &self,
        event: hps_storage::tasks::ManagementEvent,
    ) -> Result<(), ApplicationError> {
        self.store
            .record_management_event(event)
            .await
            .map_err(Into::into)
    }
    pub fn cancellation(&self) -> tokio_util::sync::CancellationToken {
        self.provider_cancel.clone()
    }
}

#[cfg(test)]
mod tests;
