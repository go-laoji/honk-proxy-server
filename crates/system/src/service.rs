//! Own, persistent OS-managed daemon services. The UI process never owns the daemon child.
use crate::{
    Result, files, process,
    profiles::{Kind, Profile},
    valid_id,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    User,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Install {
    pub id: String,
    pub binary: PathBuf,
    pub config: PathBuf,
    pub endpoint: String,
    pub scope: Scope,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format_version: u32,
    pub id: String,
    pub endpoint: String,
    pub scope: Scope,
    pub binary: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub unit: PathBuf,
    pub unit_text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub installed: bool,
    pub manager: String,
    pub state: String,
    pub pid: Option<u32>,
    pub detail: String,
    pub manifest: Option<Manifest>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Start,
    Stop,
    Restart,
    Uninstall,
}

fn label(id: &str) -> String {
    format!("io.bitdata.hps.{id}")
}
fn uid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}
fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME unavailable".into())
}
fn unit_path(id: &str, scope: Scope) -> Result<PathBuf> {
    if !valid_id(id) {
        return Err("invalid service id".into());
    }
    #[cfg(target_os = "macos")]
    {
        let root = match scope {
            Scope::User => home()?.join("Library/LaunchAgents"),
            Scope::System => PathBuf::from("/Library/LaunchDaemons"),
        };
        return Ok(root.join(format!("{}.plist", label(id))));
    }
    #[cfg(target_os = "linux")]
    {
        let root = match scope {
            Scope::System => PathBuf::from("/etc/systemd/system"),
            Scope::User => std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or(home()?.join(".config"))
                .join("systemd/user"),
        };
        return Ok(root.join(format!("{}.service", label(id))));
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = scope;
        Err("persistent services currently require macOS launchd or Linux systemd".into())
    }
}
fn text(path: &Path) -> Result<&str> {
    path.to_str().ok_or("service paths must be UTF-8".into())
}
#[cfg(any(target_os = "macos", test))]
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
#[cfg(any(not(target_os = "macos"), test))]
fn systemd_arg(value: &str) -> Result<String> {
    if value.contains(['\n', '\r', '\0']) {
        return Err("service paths cannot contain newlines or NUL".into());
    }
    Ok(format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    ))
}
fn render(m: &Manifest) -> Result<String> {
    let args = [
        text(&m.binary)?,
        "--config",
        text(&m.config)?,
        "--data-dir",
        text(&m.data)?,
    ];
    #[cfg(target_os = "macos")]
    {
        let arguments = args
            .iter()
            .map(|s| format!("<string>{}</string>", xml(s)))
            .collect::<String>();
        let log = m
            .data
            .parent()
            .ok_or("missing service parent")?
            .join("daemon.log");
        return Ok(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array>{arguments}</array><key>RunAtLoad</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict><key>ThrottleInterval</key><integer>5</integer><key>ProcessType</key><string>Background</string><key>StandardOutPath</key><string>{}</string><key>StandardErrorPath</key><string>{}</string></dict></plist>\n",
            label(&m.id),
            xml(text(&log)?),
            xml(text(&log)?)
        ));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let arguments = args
            .iter()
            .map(|s| systemd_arg(s))
            .collect::<Result<Vec<_>>>()?
            .join(" ");
        Ok(format!(
            "# Owned by honk-proxy-server; do not replace another unit.\n[Unit]\nDescription=HPS daemon {}\nAfter=network.target\nStartLimitIntervalSec=60\nStartLimitBurst=5\n[Service]\nType=exec\nExecStart={arguments}\nRestart=on-failure\nRestartSec=5\nTimeoutStopSec=30\nUMask=0077\n[Install]\nWantedBy={}\n",
            m.id,
            if m.scope == Scope::System {
                "multi-user.target"
            } else {
                "default.target"
            }
        ))
    }
}
fn check_scope(scope: Scope) -> Result<()> {
    if scope == Scope::System && uid() != 0 {
        return Err("system service installation/control requires an administrator CLI; keep Desktop unprivileged".into());
    }
    if scope == Scope::User && uid() == 0 {
        return Err("root must explicitly select system scope; do not run Desktop as root".into());
    }
    Ok(())
}

pub struct Manager {
    root: PathBuf,
}
impl Manager {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
        }
    }
    fn directory(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err("invalid service id".into());
        }
        Ok(self.root.join("services").join(id))
    }
    fn manifest(&self, id: &str) -> Result<Option<Manifest>> {
        let Some(m): Option<Manifest> = files::read(&self.directory(id)?.join("manifest.json"))?
        else {
            return Ok(None);
        };
        let directory = self.directory(id)?;
        if m.format_version != 1
            || m.id != id
            || m.binary != directory.join("hpsd")
            || m.config != directory.join("hps.toml")
            || m.data != directory.join("data")
            || m.unit != unit_path(id, m.scope)?
            || render(&m)? != m.unit_text
        {
            return Err("service manifest identity mismatch; no OS objects modified".into());
        }
        let actual = fs::read_to_string(&m.unit)
            .map_err(|e| format!("cannot verify owned service unit: {e}"))?;
        if actual != m.unit_text {
            return Err("service unit ownership conflict; no OS objects modified".into());
        }
        Ok(Some(m))
    }
    pub fn install(&self, input: Install) -> Result<Status> {
        check_scope(input.scope)?;
        if !self.root.is_absolute() {
            return Err("service state directory must be absolute".into());
        }
        let mut profile = Profile {
            id: input.id.clone(),
            name: input.id.clone(),
            kind: Kind::Local,
            endpoint: input.endpoint,
            ca_certificate: None,
        };
        profile.validate()?;
        let endpoint = url::Url::parse(&profile.endpoint).map_err(|e| e.to_string())?;
        if endpoint.path() != "/" {
            return Err("local daemon requires a root endpoint".into());
        }
        let config = hps_config::DaemonConfig::load(&input.config).map_err(|e| e.to_string())?;
        if !config.management_listen.ip().is_loopback()
            || endpoint.port_or_known_default() != Some(config.management_listen.port())
            || (config.management_tls.is_some() != (endpoint.scheme() == "https"))
        {
            return Err("service endpoint does not match loopback management configuration".into());
        }
        if input.scope == Scope::User
            && (config.linux_capture.is_some() || config.dns.capture.is_some())
        {
            return Err("eBPF capture requires an administrator-installed system service".into());
        }
        let _lock = files::lock(&self.root)?;
        let directory = self.directory(&input.id)?;
        let unit = unit_path(&input.id, input.scope)?;
        if unit.symlink_metadata().is_ok() || directory.join("manifest.json").exists() {
            return Err(
                "service already exists; inspect or uninstall the owned service first".into(),
            );
        }
        if directory.exists() {
            return Err("service staging directory already exists; inspect interrupted installation before retrying".into());
        }
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let binary = directory.join("hpsd");
        let result: Result<()> = (|| {
            let source = input.binary.canonicalize().map_err(|e| e.to_string())?;
            if !source.is_file() {
                return Err("daemon binary must be a regular file".into());
            }
            fs::copy(source, &binary).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&binary, fs::Permissions::from_mode(0o700))
                    .map_err(|e| e.to_string())?;
            }
            let version = process::checked(text(&binary)?, &["--version"])?;
            if !version.trim().starts_with("hpsd ") {
                return Err("selected binary is not hpsd".into());
            }
            let mut m = Manifest {
                format_version: 1,
                id: input.id,
                endpoint: profile.endpoint,
                scope: input.scope,
                binary,
                config: directory.join("hps.toml"),
                data: directory.join("data"),
                unit,
                unit_text: String::new(),
            };
            files::write(
                &m.config,
                toml::to_string_pretty(&config)
                    .map_err(|e| e.to_string())?
                    .as_bytes(),
            )?;
            m.unit_text = render(&m)?;
            fs::create_dir_all(m.unit.parent().ok_or("missing service directory")?)
                .map_err(|e| e.to_string())?;
            // Manifest is durable before installing the unit; interrupted installs remain inspectable.
            files::json(&directory.join("manifest.json"), &m)?;
            use std::io::Write;
            let mut f = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&m.unit)
                .map_err(|e| e.to_string())?;
            f.write_all(m.unit_text.as_bytes())
                .and_then(|_| f.sync_all())
                .map_err(|e| e.to_string())?;
            #[cfg(target_os = "linux")]
            self.systemctl(&m, &["daemon-reload"])?;
            Ok(())
        })();
        // Preserve staged evidence on failure instead of guessing which objects were installed.
        result?;
        self.status_locked(&profile.id)
    }
    #[cfg(target_os = "linux")]
    fn systemctl(&self, m: &Manifest, args: &[&str]) -> Result<String> {
        let mut arguments = Vec::new();
        if m.scope == Scope::User {
            arguments.push("--user");
        }
        arguments.extend_from_slice(args);
        process::checked("systemctl", &arguments)
    }
    #[cfg(target_os = "macos")]
    fn domain(&self, m: &Manifest) -> String {
        if m.scope == Scope::System {
            "system".into()
        } else {
            format!("gui/{}", uid())
        }
    }
    pub fn status(&self, id: &str) -> Result<Status> {
        let _lock = files::lock(&self.root)?;
        self.status_locked(id)
    }
    fn status_locked(&self, id: &str) -> Result<Status> {
        let Some(m) = self.manifest(id)? else {
            return Ok(Status {
                installed: false,
                manager: std::env::consts::OS.into(),
                state: "not_installed".into(),
                pid: None,
                detail: String::new(),
                manifest: None,
            });
        };
        #[cfg(target_os = "macos")]
        {
            let result = process::run(
                "/bin/launchctl",
                &["print", &format!("{}/{}", self.domain(&m), label(id))],
            )?;
            let pid = result.text.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("pid = ")
                    .and_then(|p| p.parse().ok())
            });
            let state = if pid.is_some() {
                "running"
            } else if result.success {
                "loaded"
            } else {
                "stopped"
            };
            return Ok(Status {
                installed: true,
                manager: "launchd".into(),
                state: state.into(),
                pid,
                detail: if result.success {
                    "OS job status; API readiness must be checked separately".into()
                } else {
                    result.text
                },
                manifest: Some(m),
            });
        }
        #[cfg(target_os = "linux")]
        {
            let output = self.systemctl(
                &m,
                &[
                    "show",
                    &format!("{}.service", label(id)),
                    "--property=ActiveState,SubState,MainPID,LoadState",
                ],
            )?;
            let pid = output
                .lines()
                .find_map(|l| {
                    l.strip_prefix("MainPID=")
                        .and_then(|s| s.parse::<u32>().ok())
                })
                .filter(|p| *p > 0);
            let state = output
                .lines()
                .find_map(|l| l.strip_prefix("ActiveState="))
                .unwrap_or("unknown")
                .to_owned();
            return Ok(Status {
                installed: true,
                manager: "systemd".into(),
                state,
                pid,
                detail: output,
                manifest: Some(m),
            });
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = m;
            Err("unsupported service manager".into())
        }
    }
    pub fn control(&self, id: &str, expected_endpoint: &str, action: Action) -> Result<Status> {
        let _lock = files::lock(&self.root)?;
        let m = self.manifest(id)?.ok_or("owned service is not installed")?;
        check_scope(m.scope)?;
        if m.endpoint != expected_endpoint {
            return Err("local service target changed; reload before controlling it".into());
        }
        #[cfg(target_os = "macos")]
        {
            let domain = self.domain(&m);
            let target = format!("{domain}/{}", label(id));
            let loaded = process::run("/bin/launchctl", &["print", &target])?.success;
            match action {
                Action::Start => {
                    if !loaded {
                        process::checked(
                            "/bin/launchctl",
                            &["bootstrap", &domain, text(&m.unit)?],
                        )?;
                    } else {
                        process::checked("/bin/launchctl", &["kickstart", &target])?;
                    }
                }
                Action::Restart => {
                    if loaded {
                        process::checked("/bin/launchctl", &["kickstart", "-k", &target])?;
                    } else {
                        process::checked(
                            "/bin/launchctl",
                            &["bootstrap", &domain, text(&m.unit)?],
                        )?;
                    }
                }
                Action::Stop | Action::Uninstall => {
                    if loaded {
                        process::checked("/bin/launchctl", &["bootout", &target])?;
                    }
                }
            }
        }
        #[cfg(target_os = "linux")]
        {
            let unit = format!("{}.service", label(id));
            match action {
                Action::Start => {
                    self.systemctl(&m, &["enable", "--now", &unit])?;
                }
                Action::Restart => {
                    self.systemctl(&m, &["restart", &unit])?;
                }
                Action::Stop => {
                    self.systemctl(&m, &["stop", &unit])?;
                }
                Action::Uninstall => {
                    self.systemctl(&m, &["disable", "--now", &unit])?;
                }
            }
        }
        if matches!(action, Action::Uninstall) {
            self.manifest(id)?
                .ok_or("service disappeared during uninstall")?;
            fs::remove_file(&m.unit).map_err(|e| e.to_string())?;
            #[cfg(target_os = "linux")]
            self.systemctl(&m, &["daemon-reload"])?;
            fs::rename(
                self.directory(id)?.join("manifest.json"),
                self.directory(id)?.join("uninstalled.json"),
            )
            .map_err(|e| e.to_string())?;
            return Ok(Status {
                installed: false,
                manager: std::env::consts::OS.into(),
                state: "uninstalled_data_preserved".into(),
                pid: None,
                detail: format!(
                    "configuration and data retained in {}",
                    self.directory(id)?.display()
                ),
                manifest: None,
            });
        }
        self.status_locked(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn systemd_arguments_do_not_expand_paths_into_directives_or_variables() {
        assert_eq!(
            systemd_arg("/tmp/a %h $HOME \\\".bin").unwrap(),
            "\"/tmp/a %%h $$HOME \\\\\\\".bin\""
        );
        assert!(systemd_arg("/tmp/path\nExecStart=bad").is_err());
        assert_eq!(xml("<&\"'"), "&lt;&amp;&quot;&apos;");
    }
    #[test]
    fn forged_manifest_cannot_control_another_unit() {
        let root = tempfile::tempdir().unwrap();
        let manager = Manager::new(root.path());
        assert!(manager.status("../foreign").is_err());
        let dir = root.path().join("services/test");
        fs::create_dir_all(&dir).unwrap();
        let m = Manifest {
            format_version: 1,
            id: "test".into(),
            endpoint: "http://127.0.0.1:19990/".into(),
            scope: Scope::User,
            binary: dir.join("hpsd"),
            config: dir.join("hps.toml"),
            data: dir.join("data"),
            unit: root.path().join("foreign.service"),
            unit_text: "foreign".into(),
        };
        files::json(&dir.join("manifest.json"), &m).unwrap();
        fs::write(&m.unit, "foreign").unwrap();
        assert!(
            manager
                .control("test", &m.endpoint, Action::Stop)
                .unwrap_err()
                .contains("identity mismatch")
        );
        assert_eq!(fs::read_to_string(m.unit).unwrap(), "foreign");
    }
}
