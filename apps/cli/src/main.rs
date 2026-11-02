use clap::{Parser, Subcommand, ValueEnum};
use hps_client::{ClientError, HpsClient};
use hps_config::DaemonConfig;
use std::{io::Write, path::PathBuf, process::ExitCode, time::Duration};

#[derive(Parser)]
#[command(
    name = "hps",
    version,
    about = "Manage a local or remote honk proxy daemon without a desktop"
)]
struct Args {
    #[arg(
        long,
        global = true,
        default_value = "http://127.0.0.1:17900",
        env = "HPS_ENDPOINT"
    )]
    endpoint: String,
    #[arg(long = "ca-cert", global = true, env = "HPS_CA_CERT")]
    ca_certificate: Option<PathBuf>,
    #[arg(long, global = true, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=300))]
    timeout: u64,
    #[arg(long, global = true, value_enum, default_value_t = Output::Text)]
    output: Output,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Output {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Command {
    Status,
    Version,
    Capabilities,
    Inbound {
        #[command(subcommand)]
        command: InboundCommand,
    },
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    Linux {
        #[command(subcommand)]
        command: LinuxCommand,
    },
    Init {
        #[arg(long, default_value = "hps.toml")]
        config: PathBuf,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Ui {
        #[arg(long)]
        open: bool,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    Show,
    Check {
        path: PathBuf,
    },
    Workspace,
    Validate {
        path: PathBuf,
        #[arg(long)]
        expected_revision: Option<i64>,
        #[arg(long)]
        draft_version: Option<i64>,
        #[arg(long, requires = "draft_version")]
        use_draft_credentials: bool,
    },
    SaveDraft {
        path: PathBuf,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        draft_version: i64,
        #[arg(long)]
        use_draft_credentials: bool,
    },
    Import {
        path: PathBuf,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long, default_value = "CLI import")]
        note: String,
        #[arg(long)]
        draft_version: Option<i64>,
        #[arg(long, requires = "draft_version")]
        use_draft_credentials: bool,
    },
    Activate {
        revision: i64,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        defer_until_restart: bool,
    },
    Rollback {
        revision: i64,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        defer_until_restart: bool,
    },
    Export {
        #[arg(long)]
        revision: Option<i64>,
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum LinuxCommand {
    Preflight,
}
#[derive(Subcommand)]
enum InboundCommand {
    List,
    Start { id: String },
    Stop { id: String },
}

#[derive(Subcommand)]
enum ProviderCommand {
    List,
    Show {
        id: String,
    },
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
    Extract {
        id: String,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long)]
        wait: bool,
    },
    Task {
        id: i64,
    },
    Leases {
        id: String,
        #[arg(long)]
        before: Option<i64>,
    },
    Diagnose {
        id: String,
        #[arg(value_parser=["quota","whitelist"])]
        kind: String,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args).await {
        Ok(value) => {
            match args.output {
                Output::Json => println!("{}", value),
                Output::Text
                    if matches!(
                        args.command,
                        Command::Config {
                            command: ConfigCommand::Export { path: None, .. }
                        }
                    ) =>
                {
                    print!("{}", value["toml"].as_str().unwrap_or(""))
                }
                Output::Text => println!(
                    "{}",
                    serde_json::to_string_pretty(&value).expect("JSON value")
                ),
            }
            if value
                .get("state")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|s| ["failed", "outcome_unknown", "discarded"].contains(&s))
                || value.get("valid").and_then(serde_json::Value::as_bool) == Some(false)
            {
                ExitCode::from(5)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err((code, message)) => {
            eprintln!("{message}");
            ExitCode::from(code)
        }
    }
}

fn client_error(error: ClientError) -> (u8, String) {
    let code = match &error {
        ClientError::InvalidEndpoint | ClientError::InvalidCertificate => 2,
        ClientError::Transport(e) if e.is_timeout() => 6,
        ClientError::Transport(_) => 3,
        ClientError::Api { status: 409, .. } => 7,
        ClientError::Api { .. } => 5,
    };
    (code, error.to_string())
}

async fn run(args: &Args) -> Result<serde_json::Value, (u8, String)> {
    // Local configuration commands do not require a reachable daemon.
    match &args.command {
        Command::Init { config } => {
            let body =
                toml::to_string_pretty(&DaemonConfig::default()).map_err(|e| (2, e.to_string()))?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(config)
                .map_err(|e| (2, e.to_string()))?;
            file.write_all(body.as_bytes())
                .map_err(|e| (2, e.to_string()))?;
            return Ok(serde_json::json!({"created": config}));
        }
        Command::Config {
            command: ConfigCommand::Check { path },
        } => {
            DaemonConfig::load(path).map_err(|e| (2, e.to_string()))?;
            return Ok(serde_json::json!({"valid": true, "path": path}));
        }
        _ => {}
    }
    let certificate = if let Some(path) = &args.ca_certificate {
        let metadata = std::fs::metadata(path).map_err(|error| (2, error.to_string()))?;
        if !metadata.is_file() || metadata.len() > 1024 * 1024 {
            return Err((
                2,
                "CA bundle must be a regular file no larger than 1 MiB".into(),
            ));
        }
        Some(std::fs::read(path).map_err(|error| (2, error.to_string()))?)
    } else {
        None
    };
    let client = HpsClient::with_ca(
        &args.endpoint,
        Duration::from_secs(args.timeout),
        certificate.as_deref(),
    )
    .map_err(client_error)?;
    let value = match &args.command {
        Command::Status => serde_json::to_value(client.status().await.map_err(client_error)?),
        Command::Version => serde_json::to_value(client.version().await.map_err(client_error)?),
        Command::Capabilities => {
            serde_json::to_value(client.capabilities().await.map_err(client_error)?)
        }
        Command::Inbound { command } => match command {
            InboundCommand::List => {
                serde_json::to_value(client.inbounds().await.map_err(client_error)?)
            }
            InboundCommand::Start { id } => {
                serde_json::to_value(client.start_inbound(id).await.map_err(client_error)?)
            }
            InboundCommand::Stop { id } => {
                serde_json::to_value(client.stop_inbound(id).await.map_err(client_error)?)
            }
        },
        Command::Provider { command } => match command {
            ProviderCommand::List => {
                serde_json::to_value(client.providers().await.map_err(client_error)?)
            }
            ProviderCommand::Show { id } => {
                serde_json::to_value(client.provider(id).await.map_err(client_error)?)
            }
            ProviderCommand::Save {
                path,
                expected_revision,
            } => {
                let metadata = std::fs::metadata(path).map_err(|e| (2, e.to_string()))?;
                if !metadata.is_file() || metadata.len() > 512 * 1024 {
                    return Err((
                        2,
                        "provider TOML must be a regular file no larger than 512 KiB".into(),
                    ));
                }
                let body = std::fs::read_to_string(path).map_err(|e| (2, e.to_string()))?;
                let configuration: hps_config::ProviderConfig = toml::from_str(&body)
                    .map_err(|_| (2, "invalid provider TOML; check syntax and fields".into()))?;
                serde_json::to_value(
                    client
                        .save_provider(configuration, *expected_revision)
                        .await
                        .map_err(client_error)?,
                )
            }
            ProviderCommand::Delete {
                id,
                expected_revision,
            } => serde_json::to_value(
                client
                    .delete_provider(id, *expected_revision)
                    .await
                    .map_err(client_error)?,
            ),
            ProviderCommand::Extract {
                id,
                request_id,
                wait,
            } => {
                let key = request_id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                eprintln!("extraction request_id={key}; reuse this ID if acknowledgment is lost");
                let mut task = client
                    .extract_provider(id, key)
                    .await
                    .map_err(client_error)?;
                if *wait {
                    let until = tokio::time::Instant::now() + Duration::from_secs(args.timeout);
                    while ["accepted", "requesting"].contains(&task.state.as_str()) {
                        if tokio::time::Instant::now() >= until {
                            eprintln!(
                                "extraction task {} continues in daemon; inspect it with provider task",
                                task.id
                            );
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        task = client.provider_task(task.id).await.map_err(client_error)?;
                    }
                }
                serde_json::to_value(task)
            }
            ProviderCommand::Task { id } => {
                serde_json::to_value(client.provider_task(*id).await.map_err(client_error)?)
            }
            ProviderCommand::Leases { id, before } => serde_json::to_value(
                client
                    .provider_leases(id, *before)
                    .await
                    .map_err(client_error)?,
            ),
            ProviderCommand::Diagnose { id, kind } => serde_json::to_value(
                client
                    .provider_diagnostic(id, kind)
                    .await
                    .map_err(client_error)?,
            ),
        },
        Command::Linux {
            command: LinuxCommand::Preflight,
        } => serde_json::to_value(client.linux_preflight().await.map_err(client_error)?),
        Command::Config { command } => match command {
            ConfigCommand::Show => {
                serde_json::to_value(client.config().await.map_err(client_error)?)
            }
            ConfigCommand::Workspace => {
                serde_json::to_value(client.config_workspace().await.map_err(client_error)?)
            }
            ConfigCommand::Validate {
                path,
                expected_revision,
                draft_version,
                use_draft_credentials,
            } => {
                let expected = match expected_revision {
                    Some(expected) => *expected,
                    None => {
                        client
                            .config_workspace()
                            .await
                            .map_err(client_error)?
                            .desired_revision
                    }
                };
                serde_json::to_value(
                    client
                        .validate_config(&read_config_input(
                            path,
                            expected,
                            *draft_version,
                            String::new(),
                            *use_draft_credentials,
                        )?)
                        .await
                        .map_err(client_error)?,
                )
            }
            ConfigCommand::SaveDraft {
                path,
                expected_revision,
                draft_version,
                use_draft_credentials,
            } => serde_json::to_value(
                client
                    .save_draft(&read_config_input(
                        path,
                        *expected_revision,
                        Some(*draft_version),
                        String::new(),
                        *use_draft_credentials,
                    )?)
                    .await
                    .map_err(client_error)?,
            ),
            ConfigCommand::Import {
                path,
                expected_revision,
                note,
                draft_version,
                use_draft_credentials,
            } => serde_json::to_value(
                client
                    .create_revision(&read_config_input(
                        path,
                        *expected_revision,
                        *draft_version,
                        note.clone(),
                        *use_draft_credentials,
                    )?)
                    .await
                    .map_err(client_error)?,
            ),
            ConfigCommand::Activate {
                revision,
                expected_revision,
                defer_until_restart,
            } => serde_json::to_value(
                client
                    .activate_config(&hps_domain::ConfigActivate {
                        revision: *revision,
                        expected_revision: *expected_revision,
                        defer_until_restart: *defer_until_restart,
                    })
                    .await
                    .map_err(client_error)?,
            ),
            ConfigCommand::Rollback {
                revision,
                expected_revision,
                defer_until_restart,
            } => serde_json::to_value(
                client
                    .rollback_config(&hps_domain::ConfigActivate {
                        revision: *revision,
                        expected_revision: *expected_revision,
                        defer_until_restart: *defer_until_restart,
                    })
                    .await
                    .map_err(client_error)?,
            ),
            ConfigCommand::Export { revision, path } => {
                let export = client
                    .export_config(*revision)
                    .await
                    .map_err(client_error)?;
                if let Some(path) = path {
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)
                        .map_err(|error| (2, error.to_string()))?;
                    file.write_all(export.toml.as_bytes())
                        .map_err(|error| (2, error.to_string()))?;
                    Ok(serde_json::json!({"created":path,"revision":export.revision}))
                } else {
                    serde_json::to_value(export)
                }
            }
            ConfigCommand::Check { .. } => unreachable!(),
        },
        Command::Ui { open } => {
            let url = client.console_url();
            if *open {
                open_browser(url.as_str()).map_err(|e| (3, e.to_string()))?;
            }
            Ok(serde_json::json!({"url": url.as_str()}))
        }
        Command::Init { .. } => unreachable!(),
    };
    value.map_err(|e| (5, e.to_string()))
}

fn read_config_input(
    path: &std::path::Path,
    expected_revision: i64,
    expected_draft_version: Option<i64>,
    note: String,
    use_draft_credentials: bool,
) -> Result<hps_domain::ConfigInput, (u8, String)> {
    let body = std::fs::read_to_string(path).map_err(|error| (2, error.to_string()))?;
    if body.len() > 2 * 1024 * 1024 {
        return Err((2, "configuration file exceeds 2 MiB".into()));
    }
    Ok(hps_domain::ConfigInput {
        toml: body,
        expected_revision,
        expected_draft_version,
        note,
        use_draft_credentials,
    })
}

fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = std::process::Command::new("xdg-open");
    if command.arg(url).status()?.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("browser could not be opened"))
    }
}
