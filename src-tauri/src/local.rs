use super::*;
use hps_system::{
    profiles::{Change, Profiles, Store},
    proxy,
    service::{Action, Install, Manager, Scope},
};
use tauri::Manager as _;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExpectedTarget {
    endpoint: String,
    profile_id: String,
}
impl ExpectedTarget {
    fn check(&self, current: &Target) -> Result<(), String> {
        if !current.local
            || current.endpoint != self.endpoint
            || current.profile_id.as_deref() != Some(&self.profile_id)
        {
            return Err(
                "OS actions require the same explicitly selected local profile; reload the target"
                    .into(),
            );
        }
        Ok(())
    }
}
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| "local operation could not finish".to_owned())?
}
#[tauri::command]
pub(super) async fn profiles_get() -> Result<Profiles, String> {
    blocking(|| Store::new(&hps_system::state_dir()?).load()).await
}
#[tauri::command]
pub(super) async fn profiles_change(
    app: tauri::AppHandle,
    expected_revision: u64,
    change: Change,
) -> Result<Profiles, String> {
    blocking(move || {
        let state = app.state::<Connection>();
        let mut current = state.0.lock().map_err(|_| "connection state unavailable")?;
        let subscriptions = app.state::<events::NativeEvents>();
        subscriptions
            .ensure_target_generation()
            .map_err(|e| e.message)?;
        let store = Store::new(&hps_system::state_dir()?);
        let profiles = store.change(expected_revision, change)?;
        // Changing the active profile cancels every stream before another document can observe the target.
        if let Some(profile) = profiles.active() {
            let target = Target::profile(profile)?;
            subscriptions.change_target().map_err(|e| e.message)?;
            *current = target;
        } else if current.profile_id.is_some() {
            subscriptions.change_target().map_err(|e| e.message)?;
            current.local = false;
            current.profile_id = None;
        }
        Ok(profiles)
    })
    .await
}
#[derive(serde::Serialize)]
pub(super) struct Context {
    endpoint: String,
    profile_id: Option<String>,
    local: bool,
    platform: &'static str,
    service: Option<hps_system::service::Status>,
    service_error: Option<String>,
    proxy: Option<proxy::Status>,
    network_services: Vec<proxy::NetworkService>,
}
#[tauri::command]
pub(super) async fn local_context(app: tauri::AppHandle) -> Result<Context, String> {
    blocking(move || {
        let target = app
            .state::<Connection>()
            .0
            .lock()
            .map_err(|_| "connection state unavailable")?
            .clone();
        let root = hps_system::state_dir()?;
        let (service, service_error, proxy, network_services) = if target.local {
            let status = Manager::new(&root).status(
                target
                    .profile_id
                    .as_deref()
                    .ok_or("local profile missing")?,
            );
            match status {
                Ok(status) => (
                    Some(status),
                    None,
                    Some(proxy::status(&root)?),
                    proxy::services().unwrap_or_default(),
                ),
                Err(error) => (
                    None,
                    Some(error),
                    Some(proxy::status(&root)?),
                    proxy::services().unwrap_or_default(),
                ),
            }
        } else {
            (None, None, None, vec![])
        };
        Ok(Context {
            endpoint: target.endpoint,
            profile_id: target.profile_id,
            local: target.local,
            platform: std::env::consts::OS,
            service,
            service_error,
            proxy,
            network_services,
        })
    })
    .await
}
#[tauri::command]
pub(super) async fn local_service_install(
    app: tauri::AppHandle,
    expected: ExpectedTarget,
    binary: String,
    config: String,
) -> Result<hps_system::service::Status, String> {
    blocking(move || {
        let state = app.state::<Connection>();
        let current = state.0.lock().map_err(|_| "connection state unavailable")?;
        expected.check(&current)?;
        Manager::new(&hps_system::state_dir()?).install(Install {
            id: expected.profile_id,
            binary: binary.into(),
            config: config.into(),
            endpoint: expected.endpoint,
            scope: Scope::User,
        })
    })
    .await
}
#[tauri::command]
pub(super) async fn local_service_control(
    app: tauri::AppHandle,
    expected: ExpectedTarget,
    action: Action,
) -> Result<hps_system::service::Status, String> {
    blocking(move || {
        let state = app.state::<Connection>();
        let current = state.0.lock().map_err(|_| "connection state unavailable")?;
        expected.check(&current)?;
        Manager::new(&hps_system::state_dir()?).control(
            &expected.profile_id,
            &expected.endpoint,
            action,
        )
    })
    .await
}
#[tauri::command]
pub(super) async fn local_proxy_apply(
    app: tauri::AppHandle,
    expected: ExpectedTarget,
    input: proxy::Apply,
) -> Result<proxy::Status, String> {
    // Check the running daemon's actual listeners before asking the OS to update proxy preferences.
    let target = {
        let state = app.state::<Connection>();
        let current = state.0.lock().map_err(|_| "connection state unavailable")?;
        expected.check(&current)?;
        current.clone()
    };
    let inbounds = target
        .client(Duration::from_secs(10))
        .map_err(|e| e.to_string())?
        .inbounds()
        .await
        .map_err(|e| e.to_string())?;
    for (protocol, port) in [("http", input.http), ("socks5", input.socks5)] {
        if let Some(port) = port {
            if !inbounds.inbounds.iter().any(|i| {
                i.protocol == protocol
                    && i.running
                    && i.listen
                        .as_ref()
                        .and_then(|s| s.parse::<std::net::SocketAddr>().ok())
                        .is_some_and(|a| {
                            a.port() == port
                                && a.is_ipv4()
                                && (a.ip().is_loopback() || a.ip().is_unspecified())
                        })
            }) {
                return Err("selected local proxy port is not a running compatible inbound".into());
            }
        }
    }
    blocking(move || {
        let state = app.state::<Connection>();
        let current = state.0.lock().map_err(|_| "connection state unavailable")?;
        expected.check(&current)?;
        proxy::apply(&hps_system::state_dir()?, &expected.endpoint, &input)
    })
    .await
}
#[tauri::command]
pub(super) async fn local_proxy_restore(
    app: tauri::AppHandle,
    expected: ExpectedTarget,
) -> Result<proxy::Status, String> {
    blocking(move || {
        let state = app.state::<Connection>();
        let current = state.0.lock().map_err(|_| "connection state unavailable")?;
        expected.check(&current)?;
        proxy::restore(&hps_system::state_dir()?)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forwarded_remote_and_stale_targets_cannot_mutate_local_os_objects() {
        let expected = ExpectedTarget {
            endpoint: "http://127.0.0.1:17983/".into(),
            profile_id: "validation-t16".into(),
        };
        let mut target = Target::address(expected.endpoint.clone());
        target.profile_id = Some(expected.profile_id.clone());
        assert!(expected.check(&target).is_err());
        target.local = true;
        expected.check(&target).unwrap();
        target.profile_id = Some("another".into());
        assert!(expected.check(&target).is_err());
    }
}
