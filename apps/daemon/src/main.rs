use clap::Parser;
use hps_application::{StartupOverrides, Supervisor};
use hps_config::DaemonConfig;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
mod management;

#[derive(Parser)]
#[command(name = "hpsd", version, about = "Headless honk proxy service")]
struct Args {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    listen: Option<SocketAddr>,
    #[arg(long)]
    instance_name: Option<String>,
    #[arg(long, conflicts_with = "ephemeral")]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    ephemeral: bool,
    #[arg(long)]
    recover_revision: Option<i64>,
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
    if let Some(name) = &args.instance_name {
        config.instance_name = name.clone();
    }
    config.validate()?;
    let directory = if args.ephemeral {
        None
    } else {
        Some(args.data_dir.unwrap_or_else(|| PathBuf::from(".hps")))
    };
    let store =
        tokio::task::spawn_blocking(move || hps_storage::Store::open(directory.as_deref(), config))
            .await??;
    if let Some(revision) = args.recover_revision {
        let target = store.revision(revision).await?;
        if target.revision.applied_at.is_none() {
            return Err("recovery requires a previously applied configuration revision".into());
        }
        let expected = store.workspace().await?.desired.revision.id;
        store.activate(revision, expected, true).await?;
    }
    let application = Supervisor::boot(
        store,
        StartupOverrides {
            listen: args.listen,
            instance_name: args.instance_name,
        },
    )
    .await?;
    let current = application.current();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_new(
            &current.config.log_filter,
        )?)
        .json()
        .with_writer(std::io::stderr)
        .init();
    let listener = match management::ManagementListener::bind(
        current.config.management_listen,
        current.management_tls.clone(),
    )
    .await
    {
        Ok(listener) => listener,
        Err(error) => {
            let _ = application.shutdown().await;
            return Err(error.into());
        }
    };
    let address = listener.address()?;
    application.confirm_startup().await?;
    tracing::info!(%address,proxy_ready=current.engine.ready(),configuration_revision=current.revision,management_tls=current.management_tls.is_some(),"management API listening; persisted runtime initialized");
    let stopping = application.clone();
    let result = axum::serve(
        listener,
        hps_api::router(Arc::new(hps_api::AppState::with_application(
            application.clone(),
        ))),
    )
    .with_graceful_shutdown(async move {
        shutdown().await;
        if let Err(error) = stopping.shutdown().await {
            tracing::error!(%error,"shutdown storage/runtime failed");
        }
    })
    .await;
    application.shutdown().await?;
    result?;
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
