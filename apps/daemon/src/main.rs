use clap::Parser;
use hps_config::DaemonConfig;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

#[derive(Parser)]
#[command(name = "hpsd", version, about = "Headless honk proxy service")]
struct Args {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    listen: Option<SocketAddr>,
    #[arg(long)]
    instance_name: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut config = match args.config {
        Some(path) => DaemonConfig::load(&path)?,
        None => DaemonConfig::default(),
    };
    if let Some(listen) = args.listen {
        config.management_listen = listen;
    }
    if let Some(name) = args.instance_name {
        config.instance_name = name;
    }
    config.validate()?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_new(&config.log_filter)?)
        .json()
        .with_writer(std::io::stderr)
        .init();
    let listener = tokio::net::TcpListener::bind(config.management_listen).await?;
    let address = listener.local_addr()?;
    tracing::info!(%address, "management API listening; proxy runtime is not initialized");
    axum::serve(
        listener,
        hps_api::router(Arc::new(hps_api::AppState::new(config))),
    )
    .with_graceful_shutdown(shutdown())
    .await?;
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = terminate.recv() => {},
            },
            Err(error) => {
                tracing::error!(%error, "cannot register SIGTERM handler");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
