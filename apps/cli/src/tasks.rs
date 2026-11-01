use super::{Output, client_error, read_policy_file};
use clap::{Args, Subcommand};
use hps_client::{ClientError, HpsClient};
use hps_domain::{EventFrame, EventQuery, TaskInfo, TaskOperation, TaskQuery, TaskSubmission};
use serde_json::Value;
use std::{io::Write, path::PathBuf, time::Duration};

type Result<T> = std::result::Result<T, (u8, String)>;
#[derive(Subcommand)]
pub(super) enum TaskCommand {
    List(Box<TaskListArgs>),
    Show {
        #[arg(value_parser=clap::value_parser!(i64).range(1..))]
        id: i64,
        #[arg(long)]
        expected_epoch: Option<String>,
    },
    Wait {
        #[arg(value_parser=clap::value_parser!(i64).range(1..))]
        id: i64,
        #[arg(long)]
        expected_epoch: Option<String>,
        #[arg(long,default_value_t=60,value_parser=clap::value_parser!(u64).range(1..=3600))]
        wait_secs: u64,
    },
    /// Accept recoverable configuration validation (or preparation of an unapplied revision).
    SubmitConfig {
        path: PathBuf,
        #[arg(long,value_parser=request_id)]
        request_id: String,
        #[arg(long,value_parser=clap::value_parser!(i64).range(1..))]
        expected_revision: i64,
        #[arg(long)]
        prepare: bool,
    },
}
#[derive(Args)]
pub(super) struct TaskListArgs {
    #[arg(long)]
    source_ref: Option<String>,
    #[arg(long)]
    expected_epoch: Option<String>,
    #[arg(long,value_parser=clap::value_parser!(i64).range(1..))]
    before: Option<i64>,
    #[arg(long,value_parser=clap::builder::PossibleValuesParser::new(hps_domain::TASK_KINDS.iter().copied()))]
    kind: Option<String>,
    #[arg(long,value_parser=clap::builder::PossibleValuesParser::new(hps_domain::TASK_STATES.iter().copied()))]
    state: Option<String>,
    #[arg(long)]
    resource_kind: Option<String>,
    #[arg(long)]
    resource_id: Option<String>,
    #[arg(long,value_parser=request_id)]
    request_id: Option<String>,
    #[arg(long)]
    created_after_unix_ms: Option<u64>,
    #[arg(long,default_value_t=50,value_parser=clap::value_parser!(u32).range(1..=200))]
    limit: u32,
}
impl TaskListArgs {
    fn query(&self) -> TaskQuery {
        TaskQuery {
            source_ref: self.source_ref.clone(),
            expected_epoch: self.expected_epoch.clone(),
            before: self.before,
            kind: self.kind.clone(),
            state: self.state.clone(),
            resource_kind: self.resource_kind.clone(),
            resource_id: self.resource_id.clone(),
            request_id: self.request_id.clone(),
            created_after_unix_ms: self.created_after_unix_ms,
            limit: Some(self.limit),
        }
    }
}
#[derive(Subcommand)]
pub(super) enum EventCommand {
    List(EventFilters),
    Follow {
        #[command(flatten)]
        filters: EventFilters,
        /// Stop after this many protocol frames (including snapshots/checkpoints).
        #[arg(long,value_parser=clap::value_parser!(u32).range(1..=10000))]
        max_frames: Option<u32>,
        #[arg(long,default_value_t=5,value_parser=clap::value_parser!(u32).range(0..=20))]
        reconnect_attempts: u32,
    },
}
#[derive(Args)]
pub(super) struct EventFilters {
    #[arg(long)]
    cursor: Option<String>,
    #[arg(long,value_parser=["info","warn","error"])]
    level: Option<String>,
    #[arg(long,value_parser=clap::builder::PossibleValuesParser::new(hps_domain::EVENT_MODULES.iter().copied()))]
    module: Option<String>,
    #[arg(long,value_parser=clap::value_parser!(i64).range(1..))]
    task_id: Option<i64>,
    #[arg(long,value_parser=request_id)]
    request_id: Option<String>,
    #[arg(long)]
    node_id: Option<String>,
    #[arg(long)]
    provider_id: Option<String>,
    #[arg(long)]
    after_unix_ms: Option<u64>,
    #[arg(long,default_value_t=100,value_parser=clap::value_parser!(u32).range(1..=200))]
    limit: u32,
}
impl EventFilters {
    fn query(&self) -> EventQuery {
        EventQuery {
            cursor: self.cursor.clone(),
            level: self.level.clone(),
            module: self.module.clone(),
            task_id: self.task_id,
            request_id: self.request_id.clone(),
            node_id: self.node_id.clone(),
            provider_id: self.provider_id.clone(),
            after_unix_ms: self.after_unix_ms,
            limit: Some(self.limit),
        }
    }
}
fn request_id(value: &str) -> std::result::Result<String, String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
    {
        Err("request ID must contain 1-64 ASCII letters, digits or _.:-".into())
    } else {
        Ok(value.into())
    }
}
fn value<T: serde::Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|_| (5, "cannot encode task/event response".into()))
}
pub(super) async fn run_task(client: &HpsClient, command: &TaskCommand) -> Result<Value> {
    match command {
        TaskCommand::List(input) => {
            value(client.tasks(&input.query()).await.map_err(client_error)?)
        }
        TaskCommand::Show { id, expected_epoch } => value(
            client
                .task(*id, expected_epoch.as_deref())
                .await
                .map_err(client_error)?,
        ),
        TaskCommand::Wait {
            id,
            expected_epoch,
            wait_secs,
        } => value(wait(client, *id, expected_epoch.as_deref(), *wait_secs).await?),
        TaskCommand::SubmitConfig {
            path,
            request_id,
            expected_revision,
            prepare,
        } => {
            let config = hps_domain::ConfigInput {
                toml: read_policy_file(path)?,
                expected_revision: *expected_revision,
                expected_draft_version: None,
                note: if *prepare {
                    "CLI asynchronous revision"
                } else {
                    "CLI asynchronous validation"
                }
                .into(),
                use_draft_credentials: false,
            };
            value(
                client
                    .submit_task(&TaskSubmission {
                        request_id: request_id.clone(),
                        operation: if *prepare {
                            TaskOperation::ConfigurationPrepare(config)
                        } else {
                            TaskOperation::ConfigurationValidate(config)
                        },
                    })
                    .await
                    .map_err(client_error)?,
            )
        }
    }
}
async fn wait(client: &HpsClient, id: i64, epoch: Option<&str>, seconds: u64) -> Result<TaskInfo> {
    tokio::time::timeout(Duration::from_secs(seconds),async {
        loop {let task=client.task(id,epoch).await.map_err(client_error)?;if task.terminal {return Ok(task);}tokio::time::sleep(Duration::from_millis(250)).await;}
    }).await.map_err(|_|(6,"task has not reached a terminal state before the wait deadline; inspect the same task ID before resubmitting".into()))?
}
pub(super) async fn run_events(
    client: &HpsClient,
    command: &EventCommand,
    output: Output,
) -> Result<Value> {
    match command {
        EventCommand::List(input) => {
            value(client.events(&input.query()).await.map_err(client_error)?)
        }
        EventCommand::Follow {
            filters,
            max_frames,
            reconnect_attempts,
        } => {
            follow(
                client,
                filters.query(),
                *max_frames,
                *reconnect_attempts,
                output,
            )
            .await
        }
    }
}
fn emit(value: &impl serde::Serialize, output: Output) -> Result<()> {
    let text = match output {
        Output::Json => serde_json::to_string(value),
        Output::Text => serde_json::to_string_pretty(value),
    }
    .map_err(|_| (5, "cannot encode event frame".into()))?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{text}")
        .and_then(|_| stdout.flush())
        .map_err(|_| (3, "cannot write event output".into()))
}
fn end(frames: u32, cursor: &Option<String>, reason: &str) -> Value {
    serde_json::json!({"kind":"end","state":"stopped","frames":frames,"cursor":cursor,"reason":reason})
}
fn retryable(error: &ClientError) -> bool {
    !matches!(
        error,
        ClientError::InvalidEndpoint
            | ClientError::InvalidCertificate
            | ClientError::InvalidTaskId
            | ClientError::InvalidEventCursor
            | ClientError::InvalidConnectionId
            | ClientError::EventProtocol
    ) && !matches!(error,ClientError::Api {status,..} if (400..500).contains(status) && *status!=429)
}
async fn follow(
    client: &HpsClient,
    mut query: EventQuery,
    max_frames: Option<u32>,
    maximum_retries: u32,
    output: Output,
) -> Result<Value> {
    let mut frames = 0u32;
    let mut retries = 0u32;
    loop {
        let opening = tokio::select! {
            signal=tokio::signal::ctrl_c()=>{signal.map_err(|_|(3,"cannot observe interrupt signal".into()))?;return Ok(end(frames,&query.cursor,"interrupted"));},
            response=client.subscribe_events(&query)=>response,
        };
        let ((code, message), retry) = match opening {
            Err(error) => {
                let retry = retryable(&error);
                (client_error(error), retry)
            }
            Ok(mut stream) => loop {
                let next = tokio::select! {
                    signal=tokio::signal::ctrl_c()=>{signal.map_err(|_|(3,"cannot observe interrupt signal".into()))?;return Ok(end(frames,&query.cursor,"interrupted"));},
                    frame=stream.next()=>frame,
                };
                match next {
                    Err(error) => {
                        let retry = retryable(&error);
                        break (client_error(error), retry);
                    }
                    Ok(None) => break ((3, "event transport ended".into()), true),
                    Ok(Some(frame)) => {
                        if let Some(cursor) = stream.cursor() {
                            query.cursor = Some(cursor.into());
                        }
                        emit(&frame, output)?;
                        frames = frames.saturating_add(1);
                        if matches!(frame, EventFrame::Unavailable { .. }) {
                            break ((5, "event source unavailable".into()), true);
                        }
                        if max_frames.is_some_and(|n| frames >= n) {
                            return Ok(end(frames, &query.cursor, "frame_limit"));
                        }
                    }
                }
            },
        };
        if !retry || retries >= maximum_retries {
            return Err((
                code,
                format!(
                    "{message}; last complete cursor: {}",
                    query.cursor.as_deref().unwrap_or("none")
                ),
            ));
        }
        retries += 1;
        emit(
            &serde_json::json!({"kind":"transport","data":{"state":"reconnecting","attempt":retries,"cursor":query.cursor}}),
            output,
        )?;
        let delay = Duration::from_millis(500 * (1u64 << retries.saturating_sub(1).min(4)));
        tokio::select! {
            signal=tokio::signal::ctrl_c()=>{signal.map_err(|_|(3,"cannot observe interrupt signal".into()))?;return Ok(end(frames,&query.cursor,"interrupted"));},
            _=tokio::time::sleep(delay)=>{},
        }
    }
}
