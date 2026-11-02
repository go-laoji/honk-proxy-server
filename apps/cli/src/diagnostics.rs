use super::{client_error, tasks};
use clap::Subcommand;
use hps_client::HpsClient;
use hps_domain::{DiagnosticInput, TaskControlInput, TaskRetryInput};
use serde_json::Value;
type Result<T> = std::result::Result<T, (u8, String)>;

#[derive(Subcommand)]
pub(super) enum DiagnosticCommand {
    /// Read bounded alert transitions; there is no account-specific acknowledgement state.
    Alerts {
        #[arg(long)]
        after: Option<u64>,
        #[arg(long,default_value_t=50,value_parser=clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Collect a bounded metadata-only diagnostic bundle as a daemon-owned task.
    Start {
        #[arg(long, default_value_t = 300, value_parser=clap::value_parser!(u32).range(60..=3600))]
        window_secs: u32,
        #[arg(long, value_parser=tasks::request_id)]
        request_id: Option<String>,
        #[arg(long)]
        expected_epoch: Option<String>,
        #[arg(long)]
        wait: bool,
    },
    Show {
        #[arg(value_parser=clap::value_parser!(i64).range(1..))]
        id: i64,
        #[arg(long)]
        expected_epoch: Option<String>,
    },
    Controls {
        #[arg(value_parser=clap::value_parser!(i64).range(1..))]
        id: i64,
        #[arg(long)]
        expected_epoch: Option<String>,
    },
    Cancel {
        #[arg(value_parser=clap::value_parser!(i64).range(1..))]
        id: i64,
        #[arg(long)]
        expected_epoch: String,
        #[arg(long)]
        expected_version: u64,
    },
    Retry {
        #[arg(value_parser=clap::value_parser!(i64).range(1..))]
        id: i64,
        #[arg(long)]
        expected_epoch: String,
        #[arg(long)]
        expected_version: u64,
        #[arg(long, value_parser=tasks::request_id)]
        request_id: String,
    },
}
fn value(value: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|_| (5, "cannot encode diagnostic response".into()))
}
pub(super) async fn run(client: &HpsClient, command: &DiagnosticCommand) -> Result<Value> {
    match command {
        DiagnosticCommand::Alerts { after, limit } => {
            value(client.alerts(*after, *limit).await.map_err(client_error)?)
        }
        DiagnosticCommand::Start {
            window_secs,
            request_id,
            expected_epoch,
            wait,
        } => {
            let epoch = match expected_epoch {
                Some(epoch) => epoch.clone(),
                None => client.status().await.map_err(client_error)?.instance_id,
            };
            let task = client
                .submit_diagnostic(&DiagnosticInput {
                    expected_epoch: epoch.clone(),
                    request_id: request_id
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    window_secs: *window_secs,
                })
                .await
                .map_err(client_error)?;
            if *wait {
                let completed = tasks::wait(client, task.id, Some(&epoch), 60).await?;
                if completed.state != "completed" {
                    return value(completed);
                }
                value(
                    client
                        .diagnostic(task.id, Some(&epoch))
                        .await
                        .map_err(client_error)?,
                )
            } else {
                value(task)
            }
        }
        DiagnosticCommand::Show { id, expected_epoch } => value(
            client
                .diagnostic(*id, expected_epoch.as_deref())
                .await
                .map_err(client_error)?,
        ),
        DiagnosticCommand::Controls { id, expected_epoch } => value(
            client
                .task_controls(*id, expected_epoch.as_deref())
                .await
                .map_err(client_error)?,
        ),
        DiagnosticCommand::Cancel {
            id,
            expected_epoch,
            expected_version,
        } => value(
            client
                .cancel_task(
                    *id,
                    &TaskControlInput {
                        expected_epoch: expected_epoch.clone(),
                        expected_version: *expected_version,
                    },
                )
                .await
                .map_err(client_error)?,
        ),
        DiagnosticCommand::Retry {
            id,
            expected_epoch,
            expected_version,
            request_id,
        } => value(
            client
                .retry_task(
                    *id,
                    &TaskRetryInput {
                        expected_epoch: expected_epoch.clone(),
                        expected_version: *expected_version,
                        request_id: request_id.clone(),
                    },
                )
                .await
                .map_err(client_error)?,
        ),
    }
}
