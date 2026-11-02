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
    Check { path: PathBuf },
}

#[derive(Subcommand)]
enum LinuxCommand {
    Preflight,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args).await {
        Ok(value) => {
            match args.output {
                Output::Json => println!("{}", value),
                Output::Text => println!(
                    "{}",
                    serde_json::to_string_pretty(&value).expect("JSON value")
                ),
            }
            ExitCode::SUCCESS
        }
        Err((code, message)) => {
            eprintln!("{message}");
            ExitCode::from(code)
        }
    }
}

fn client_error(error: ClientError) -> (u8, String) {
    let code = match &error {
        ClientError::InvalidEndpoint => 2,
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
    let client =
        HpsClient::new(&args.endpoint, Duration::from_secs(args.timeout)).map_err(client_error)?;
    let value = match &args.command {
        Command::Status => serde_json::to_value(client.status().await.map_err(client_error)?),
        Command::Version => serde_json::to_value(client.version().await.map_err(client_error)?),
        Command::Capabilities => {
            serde_json::to_value(client.capabilities().await.map_err(client_error)?)
        }
        Command::Linux {
            command: LinuxCommand::Preflight,
        } => serde_json::to_value(client.linux_preflight().await.map_err(client_error)?),
        Command::Config {
            command: ConfigCommand::Show,
        } => serde_json::to_value(client.config().await.map_err(client_error)?),
        Command::Ui { open } => {
            let url = client.console_url();
            if *open {
                open_browser(url.as_str()).map_err(|e| (3, e.to_string()))?;
            }
            Ok(serde_json::json!({"url": url.as_str()}))
        }
        Command::Init { .. }
        | Command::Config {
            command: ConfigCommand::Check { .. },
        } => unreachable!(),
    };
    value.map_err(|e| (5, e.to_string()))
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
