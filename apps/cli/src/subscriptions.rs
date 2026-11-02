use super::*;
#[derive(Subcommand)]
pub enum SubscriptionCommand {
    List,
    Save {
        path: PathBuf,
        #[arg(long)]
        expected_revision: i64,
    },
    Delete {
        id: String,
        #[arg(long)]
        expected_revision: i64,
    },
    /// GET the source, or atomically import a local URI/Clash document with --document.
    Refresh {
        id: String,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        document: Option<PathBuf>,
    },
}
pub async fn run(
    client: &HpsClient,
    command: &SubscriptionCommand,
) -> Result<serde_json::Value, (u8, String)> {
    let value = match command {
        SubscriptionCommand::List => {
            serde_json::to_value(client.subscriptions().await.map_err(client_error)?)
        }
        SubscriptionCommand::Save {
            path,
            expected_revision,
        } => {
            let raw = read_policy_file(path)?;
            let configuration = toml::from_str::<hps_config::SubscriptionConfig>(&raw)
                .map_err(|_| (2, "invalid subscription definition TOML".into()))?;
            serde_json::to_value(
                client
                    .save_subscription(&hps_config::SubscriptionSave {
                        configuration,
                        expected_revision: *expected_revision,
                    })
                    .await
                    .map_err(client_error)?,
            )
        }
        SubscriptionCommand::Delete {
            id,
            expected_revision,
        } => serde_json::to_value(
            client
                .delete_subscription(id, *expected_revision)
                .await
                .map_err(client_error)?,
        ),
        SubscriptionCommand::Refresh {
            id,
            expected_revision,
            document,
        } => serde_json::to_value(
            client
                .refresh_subscription(
                    id,
                    &hps_config::SubscriptionRefresh {
                        expected_revision: *expected_revision,
                        document: document
                            .as_ref()
                            .map(|path| read_policy_file(path))
                            .transpose()?,
                    },
                )
                .await
                .map_err(client_error)?,
        ),
    };
    value.map_err(|e| (5, e.to_string()))
}
