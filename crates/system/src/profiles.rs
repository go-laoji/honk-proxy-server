use crate::{Result, files, valid_id};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Local,
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub endpoint: String,
    #[serde(default)]
    pub ca_certificate: Option<PathBuf>,
}
impl Profile {
    pub fn validate(&mut self) -> Result<()> {
        if !valid_id(&self.id) || self.name.trim().is_empty() || self.name.len() > 160 {
            return Err("profile requires a safe id and a name of at most 160 bytes".into());
        }
        let certificate = self
            .ca_certificate
            .as_ref()
            .map(|path| -> Result<Vec<u8>> {
                if !path.is_absolute() {
                    return Err("CA certificate path must be absolute".into());
                }
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                if bytes.len() > 1_048_576 {
                    return Err("CA certificate exceeds 1 MiB".into());
                }
                Ok(bytes)
            })
            .transpose()?;
        let client = hps_client::HpsClient::with_ca(
            &self.endpoint,
            Duration::from_secs(10),
            certificate.as_deref(),
        )
        .map_err(|e| e.to_string())?;
        if self.kind == Kind::Local && !is_loopback(client.endpoint()) {
            return Err("local profiles require a literal loopback IP or localhost".into());
        }
        self.endpoint = client.endpoint().as_str().to_owned();
        Ok(())
    }
}
pub fn is_loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(host)) => host == "localhost",
        None => false,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Profiles {
    pub format_version: u32,
    pub revision: u64,
    pub selected: Option<String>,
    pub profiles: Vec<Profile>,
}
impl Profiles {
    pub fn active(&self) -> Option<&Profile> {
        self.selected
            .as_ref()
            .and_then(|id| self.profiles.iter().find(|p| p.id == *id))
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    Unselect,
    Upsert { profile: Profile },
    Remove { id: String },
    Select { id: String },
}

pub struct Store {
    path: PathBuf,
}
impl Store {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.to_owned(),
        }
    }
    fn read(&self) -> Result<Profiles> {
        let state: Profiles = files::read(&self.path.join("profiles.json"))?.unwrap_or(Profiles {
            format_version: 1,
            ..Profiles::default()
        });
        if state.format_version != 1
            || state.profiles.len() > 32
            || state
                .selected
                .as_ref()
                .is_some_and(|id| !state.profiles.iter().any(|p| p.id == *id))
        {
            return Err("unsupported or invalid profile document; preserved unchanged".into());
        }
        let mut seen = std::collections::HashSet::new();
        for p in &state.profiles {
            if !valid_id(&p.id) || !seen.insert(&p.id) {
                return Err("invalid or duplicate profile id".into());
            }
        }
        Ok(state)
    }
    pub fn load(&self) -> Result<Profiles> {
        let _lock = files::lock(&self.path)?;
        self.read()
    }
    pub fn change(&self, expected_revision: u64, change: Change) -> Result<Profiles> {
        let _lock = files::lock(&self.path)?;
        let mut state = self.read()?;
        if state.revision != expected_revision {
            return Err("profile revision conflict; reload current profiles".into());
        }
        match change {
            Change::Unselect => state.selected = None,
            Change::Upsert { mut profile } => {
                profile.validate()?;
                if let Some(existing) = state.profiles.iter_mut().find(|p| p.id == profile.id) {
                    *existing = profile;
                } else {
                    if state.profiles.len() == 32 {
                        return Err("at most 32 profiles are allowed".into());
                    }
                    state.profiles.push(profile);
                }
            }
            Change::Remove { id } => {
                let pos = state
                    .profiles
                    .iter()
                    .position(|p| p.id == id)
                    .ok_or("profile not found")?;
                state.profiles.remove(pos);
                if state.selected.as_ref() == Some(&id) {
                    state.selected = None;
                }
            }
            Change::Select { id } => {
                let mut profile = state
                    .profiles
                    .iter()
                    .find(|p| p.id == id)
                    .cloned()
                    .ok_or("profile not found")?;
                profile.validate()?;
                state.selected = Some(id);
            }
        }
        state.revision = state
            .revision
            .checked_add(1)
            .ok_or("profile revision exhausted")?;
        files::json(&self.path.join("profiles.json"), &state)?;
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(id: &str) -> Profile {
        Profile {
            id: id.into(),
            name: id.into(),
            kind: Kind::Local,
            endpoint: "http://127.0.0.1:19990".into(),
            ca_certificate: None,
        }
    }
    #[test]
    fn cross_process_revision_conflicts_and_corruption_preserve_existing_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let initial = store
            .change(
                0,
                Change::Upsert {
                    profile: p("local"),
                },
            )
            .unwrap();
        let other = Store::new(dir.path());
        other
            .change(initial.revision, Change::Select { id: "local".into() })
            .unwrap();
        assert!(
            store
                .change(initial.revision, Change::Remove { id: "local".into() })
                .unwrap_err()
                .contains("conflict")
        );
        assert_eq!(
            store.load().unwrap().active().unwrap().endpoint,
            "http://127.0.0.1:19990/"
        );
        let path = dir.path().join("profiles.json");
        std::fs::write(&path, "broken").unwrap();
        assert!(
            store
                .change(0, Change::Upsert { profile: p("new") })
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
    }
    #[test]
    fn local_is_explicit_and_not_guessed_from_a_forwarded_endpoint() {
        let mut remote = p("remote");
        remote.kind = Kind::Remote;
        remote.validate().unwrap();
        assert_eq!(remote.kind, Kind::Remote);
        let mut local = p("local");
        local.endpoint = "http://192.0.2.1".into();
        assert!(local.validate().is_err());
        local.endpoint = "http://user:pass@127.0.0.1".into();
        assert!(local.validate().is_err());
        assert!(!valid_id("../another-service"));
    }
}
