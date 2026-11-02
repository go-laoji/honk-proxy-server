use std::path::PathBuf;
#[derive(clap::Subcommand)]
pub enum BackupCommand {
    /// Download a consistent snapshot. Contains upstream credentials. Refuses overwrite.
    Create { path: PathBuf },
    /// Validate a full SQLite snapshot locally without starting a daemon.
    Inspect {
        path: PathBuf,
        #[arg(long)]
        sha256: Option<String>,
    },
    /// Stop the daemon first. Atomically replace its DB, preserving the old DB. Restart required.
    Restore {
        path: PathBuf,
        #[arg(long)]
        data_dir: PathBuf,
        #[arg(long)]
        sha256: String,
        #[arg(long, required = true)]
        offline: bool,
    },
}
pub fn local(
    command: &BackupCommand,
    remote: bool,
) -> Option<Result<serde_json::Value, (u8, String)>> {
    let result = match command {
        BackupCommand::Create { .. } => return None,
        _ if remote => {
            return Some(Err((
                2,
                "inspect/restore run on this computer; omit --endpoint and --profile".into(),
            )));
        }
        BackupCommand::Inspect { path, sha256 } => {
            hps_storage::backup::inspect(path, sha256.as_deref()).and_then(|v| {
                serde_json::to_value(v).map_err(|_| hps_storage::backup::BackupError::Invalid)
            })
        }
        BackupCommand::Restore {
            path,
            data_dir,
            sha256,
            ..
        } => hps_storage::backup::restore(data_dir, path, sha256).and_then(|v| {
            serde_json::to_value(v).map_err(|_| hps_storage::backup::BackupError::Invalid)
        }),
    };
    Some(result.map_err(|e| (5, e.to_string())))
}
