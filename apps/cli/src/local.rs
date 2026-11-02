use clap::Subcommand;
use hps_system::profiles::{Change, Kind, Profile, Store};
use std::path::PathBuf;

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
