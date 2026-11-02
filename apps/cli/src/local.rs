use clap::Subcommand;
use hps_system::profiles::{Change, Kind, Profile, Store};
use std::path::PathBuf;
#[derive(Subcommand)]
pub enum ServiceCommand {
    Install {
        #[arg(long)]
        binary: PathBuf,
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        address: String,
        #[arg(long)]
        system: bool,
    },
    Status,
    Start {
        #[arg(long)]
        expected_endpoint: String,
    },
    Stop {
        #[arg(long)]
        expected_endpoint: String,
    },
    Restart {
        #[arg(long)]
        expected_endpoint: String,
    },
    Uninstall {
        #[arg(long)]
        expected_endpoint: String,
    },
}
pub fn service(id: &str, command: &ServiceCommand) -> Result<serde_json::Value, (u8, String)> {
    use hps_system::service::{Action, Install, Manager, Scope};
    let manager = Manager::new(&hps_system::state_dir().map_err(|e| (2, e))?);
    let state = match command {
        ServiceCommand::Status => manager.status(id),
        ServiceCommand::Install {
            binary,
            config,
            address,
            system,
        } => manager.install(Install {
            id: id.into(),
            binary: binary.clone(),
            config: config.clone(),
            endpoint: address.clone(),
            scope: if *system { Scope::System } else { Scope::User },
        }),
        ServiceCommand::Start { expected_endpoint } => {
            manager.control(id, expected_endpoint, Action::Start)
        }
        ServiceCommand::Stop { expected_endpoint } => {
            manager.control(id, expected_endpoint, Action::Stop)
        }
        ServiceCommand::Restart { expected_endpoint } => {
            manager.control(id, expected_endpoint, Action::Restart)
        }
        ServiceCommand::Uninstall { expected_endpoint } => {
            manager.control(id, expected_endpoint, Action::Uninstall)
        }
    }
    .map_err(|e| (5, e))?;
    serde_json::to_value(state).map_err(|e| (2, e.to_string()))
}

#[derive(Subcommand)]
pub enum ProfileCommand {
    List,
    Save {
        id: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        address: String,
        /// Explicit local OS actions are allowed only for a local profile.
        #[arg(long)]
        local: bool,
        #[arg(long)]
        ca_certificate: Option<PathBuf>,
        #[arg(long)]
        expected_revision: u64,
    },
    Select {
        id: String,
        #[arg(long)]
        expected_revision: u64,
    },
    Remove {
        id: String,
        #[arg(long)]
        expected_revision: u64,
    },
}
pub fn profiles(command: &ProfileCommand) -> Result<serde_json::Value, (u8, String)> {
    let store = Store::new(&hps_system::state_dir().map_err(|e| (2, e))?);
    let result = match command {
        ProfileCommand::List => store.load(),
        ProfileCommand::Save {
            id,
            name,
            address,
            local,
            ca_certificate,
            expected_revision,
        } => store.change(
            *expected_revision,
            Change::Upsert {
                profile: Profile {
                    id: id.clone(),
                    name: name.clone(),
                    endpoint: address.clone(),
                    kind: if *local { Kind::Local } else { Kind::Remote },
                    ca_certificate: ca_certificate.clone(),
                },
            },
        ),
        ProfileCommand::Select {
            id,
            expected_revision,
        } => store.change(*expected_revision, Change::Select { id: id.clone() }),
        ProfileCommand::Remove {
            id,
            expected_revision,
        } => store.change(*expected_revision, Change::Remove { id: id.clone() }),
    }
    .map_err(|e| {
        (
            if e.contains("revision conflict") {
                7
            } else {
                2
            },
            e,
        )
    })?;
    serde_json::to_value(result).map_err(|e| (2, e.to_string()))
}
