//! Recoverable, compare-before-restore ownership of local OS proxy preferences.
use crate::{Result, files};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[cfg(target_os = "macos")]
mod macos;

#[derive(Clone, Debug, Serialize)]
pub struct NetworkService {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub supported: bool,
    pub state: String,
    pub network_service: Option<String>,
    pub endpoint: Option<String>,
    pub detail: String,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format_version: u32,
    service: String,
    endpoint: String,
    original: Option<plist::Value>,
    applied: plist::Value,
}
#[cfg(any(target_os = "macos", test))]
pub(crate) trait Backend {
    fn services(&self) -> Result<Vec<NetworkService>>;
    fn read(&self, service: &str) -> Result<Option<plist::Value>>;
    fn write(&mut self, service: &str, value: Option<&plist::Value>) -> Result<()>;
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Apply {
    pub network_service: String,
    pub http: Option<u16>,
    pub socks5: Option<u16>,
}
#[cfg(any(target_os = "macos", test))]
fn applied(original: Option<&plist::Value>, input: &Apply) -> Result<plist::Value> {
    if input.http.is_none() && input.socks5.is_none()
        || input.http == Some(0)
        || input.socks5 == Some(0)
    {
        return Err("select at least one running local HTTP or SOCKS5 port".into());
    }
    let mut dictionary = match original {
        None => plist::Dictionary::new(),
        Some(plist::Value::Dictionary(d)) => d.clone(),
        _ => return Err("unexpected OS proxy dictionary".into()),
    };
    for (prefix, port) in [
        ("HTTP", input.http),
        ("HTTPS", input.http),
        ("SOCKS", input.socks5),
    ] {
        dictionary.insert(
            format!("{prefix}Enable"),
            plist::Value::Integer(u64::from(port.is_some()).into()),
        );
        if let Some(port) = port {
            dictionary.insert(
                format!("{prefix}Proxy"),
                plist::Value::String("127.0.0.1".into()),
            );
            dictionary.insert(
                format!("{prefix}Port"),
                plist::Value::Integer(u64::from(port).into()),
            );
            dictionary.insert(format!("{prefix}User"), plist::Value::String(String::new()));
        }
    }
    dictionary.insert(
        "ProxyAutoConfigEnable".into(),
        plist::Value::Integer(0u64.into()),
    );
    dictionary.insert(
        "ProxyAutoDiscoveryEnable".into(),
        plist::Value::Integer(0u64.into()),
    );
    Ok(plist::Value::Dictionary(dictionary))
}
#[cfg(any(target_os = "macos", test))]
fn load(root: &Path) -> Result<Option<Journal>> {
    let journal: Option<Journal> = files::read(&root.join("system-proxy.json"))?;
    if journal.as_ref().is_some_and(|j| j.format_version != 1) {
        return Err("unsupported proxy backup; preserved unchanged".into());
    }
    Ok(journal)
}
#[cfg(any(target_os = "macos", test))]
fn status_with(root: &Path, backend: &impl Backend) -> Result<Status> {
    let Some(j) = load(root)? else {
        return Ok(Status {
            supported: true,
            state: "unmanaged".into(),
            network_service: None,
            endpoint: None,
            detail: "No local proxy changes owned by this application".into(),
        });
    };
    let actual = backend.read(&j.service)?;
    let state = if actual.as_ref() == Some(&j.applied) {
        "owned"
    } else if actual == j.original {
        "original_restored"
    } else {
        "external_change"
    };
    Ok(Status{supported:true,state:state.into(),network_service:Some(j.service),endpoint:Some(j.endpoint),detail:"Original settings are retained until an explicit successful restore; window exit does not change them".into()})
}
#[cfg(any(target_os = "macos", test))]
fn apply_with(
    root: &Path,
    backend: &mut impl Backend,
    endpoint: &str,
    input: &Apply,
) -> Result<Status> {
    if load(root)?.is_some() {
        return Err(
            "restore the existing proxy backup before applying another configuration".into(),
        );
    }
    if !backend
        .services()?
        .iter()
        .any(|s| s.id == input.network_service)
    {
        return Err("network service no longer exists".into());
    }
    let original = backend.read(&input.network_service)?;
    let applied = applied(original.as_ref(), input)?;
    let j = Journal {
        format_version: 1,
        service: input.network_service.clone(),
        endpoint: endpoint.into(),
        original,
        applied,
    };
    files::json(&root.join("system-proxy.json"), &j)?;
    // Backup precedes the OS commit. On error retain it so a partially applied operation is recoverable.
    backend.write(&j.service, Some(&j.applied))?;
    status_with(root, backend)
}
#[cfg(any(target_os = "macos", test))]
fn restore_with(root: &Path, backend: &mut impl Backend) -> Result<Status> {
    if let Some(j) = load(root)? {
        let actual = backend.read(&j.service)?;
        if actual.as_ref() == Some(&j.applied) {
            backend.write(&j.service, j.original.as_ref())?;
        } else if actual != j.original {
            return Err("OS proxy ownership conflict: settings changed after HPS applied them; original backup retained".into());
        }
        if backend.read(&j.service)? != j.original {
            return Err("OS proxy restoration could not be confirmed; backup retained".into());
        }
        std::fs::remove_file(root.join("system-proxy.json")).map_err(|e| e.to_string())?;
    }
    status_with(root, backend)
}

pub fn services() -> Result<Vec<NetworkService>> {
    #[cfg(target_os = "macos")]
    {
        macos::Session::open(false)?.services()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("system proxy integration currently requires macOS SystemConfiguration; explicit browser proxy settings remain available".into())
    }
}
pub fn status(root: &Path) -> Result<Status> {
    let _lock = files::lock(root)?;
    #[cfg(target_os = "macos")]
    {
        status_with(root, &macos::Session::open(false)?)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(Status {
            supported: false,
            state: "unsupported".into(),
            network_service: None,
            endpoint: None,
            detail: "No supported OS proxy preference backend on this platform".into(),
        })
    }
}
pub fn apply(root: &Path, endpoint: &str, input: &Apply) -> Result<Status> {
    let url = url::Url::parse(endpoint).map_err(|e| e.to_string())?;
    if !crate::profiles::is_loopback(&url) {
        return Err("system proxy changes require an explicit local profile".into());
    }
    let _lock = files::lock(root)?;
    #[cfg(target_os = "macos")]
    {
        apply_with(root, &mut macos::Session::open(true)?, endpoint, input)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = input;
        Err("system proxy backend unsupported on this platform".into())
    }
}
pub fn restore(root: &Path) -> Result<Status> {
    let _lock = files::lock(root)?;
    #[cfg(target_os = "macos")]
    {
        restore_with(root, &mut macos::Session::open(true)?)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("system proxy backend unsupported on this platform".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        value: Option<plist::Value>,
        fail_after_commit: bool,
    }
    impl Backend for Fixture {
        fn services(&self) -> Result<Vec<NetworkService>> {
            Ok(vec![NetworkService {
                id: "test".into(),
                name: "test".into(),
            }])
        }
        fn read(&self, _: &str) -> Result<Option<plist::Value>> {
            Ok(self.value.clone())
        }
        fn write(&mut self, _: &str, value: Option<&plist::Value>) -> Result<()> {
            self.value = value.cloned();
            if self.fail_after_commit {
                self.fail_after_commit = false;
                Err("OS apply failed after commit".into())
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn preserves_pac_bypass_auth_and_absent_dictionaries_without_overwriting_external_changes() {
        let dir = tempfile::tempdir().unwrap();
        let original=plist::Value::from_reader_xml(std::io::Cursor::new(b"<plist version=\"1.0\"><dict><key>ProxyAutoConfigURLString</key><string>https://pac.invalid/a</string><key>HTTPUser</key><string>original-user</string><key>ExceptionsList</key><array><string>*.local</string></array></dict></plist>")).unwrap();
        let mut fixture = Fixture {
            value: Some(original.clone()),
            fail_after_commit: true,
        };
        let input = Apply {
            network_service: "test".into(),
            http: Some(18983),
            socks5: None,
        };
        assert!(apply_with(dir.path(), &mut fixture, "http://127.0.0.1/", &input).is_err());
        assert_eq!(status_with(dir.path(), &fixture).unwrap().state, "owned");
        let applied = fixture.value.clone();
        fixture.value = Some(plist::Value::String("external".into()));
        assert!(
            restore_with(dir.path(), &mut fixture)
                .unwrap_err()
                .contains("ownership conflict")
        );
        assert!(dir.path().join("system-proxy.json").exists());
        fixture.value = applied;
        restore_with(dir.path(), &mut fixture).unwrap();
        assert_eq!(fixture.value, Some(original));
        fixture.value = None;
        apply_with(dir.path(), &mut fixture, "http://127.0.0.1/", &input).unwrap();
        restore_with(dir.path(), &mut fixture).unwrap();
        assert!(fixture.value.is_none());
    }
}
