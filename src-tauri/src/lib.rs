use hps_client::HpsClient;
use std::{sync::Mutex, time::Duration};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
mod events;
mod local;

#[derive(Clone, PartialEq, Eq)]
struct Target {
    endpoint: String,
    ca_certificate: Option<Vec<u8>>,
    profile_id: Option<String>,
    local: bool,
}
impl Target {
    fn client(&self, timeout: Duration) -> Result<HpsClient, hps_client::ClientError> {
        HpsClient::with_ca(&self.endpoint, timeout, self.ca_certificate.as_deref())
    }
    fn address(endpoint: String) -> Self {
        Self {
            endpoint,
            ca_certificate: None,
            profile_id: None,
            local: false,
        }
    }
    fn profile(profile: &hps_system::profiles::Profile) -> Result<Self, String> {
        let mut profile = profile.clone();
        profile.validate()?;
        let ca_certificate = profile
            .ca_certificate
            .as_ref()
            .map(std::fs::read)
            .transpose()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            endpoint: profile.endpoint,
            ca_certificate,
            profile_id: Some(profile.id),
            local: profile.kind == hps_system::profiles::Kind::Local,
        })
    }
}
struct Connection(Mutex<Target>);
#[derive(Debug, serde::Serialize)]
struct NativeApiError {
    message: String,
    code: String,
    status: u16,
    request_id: Option<String>,
}
impl From<&str> for NativeApiError {
    fn from(message: &str) -> Self {
        Self {
            message: message.into(),
            code: "native_transport_error".into(),
            status: 0,
            request_id: None,
        }
    }
}
impl From<hps_client::ClientError> for NativeApiError {
    fn from(error: hps_client::ClientError) -> Self {
        match error {
            hps_client::ClientError::Api { status, error } => Self {
                message: error.message,
                code: error.code,
                status,
                request_id: Some(error.request_id),
            },
            other => Self {
                message: other.to_string(),
                code: "transport_error".into(),
                status: 0,
                request_id: None,
            },
        }
    }
}

#[tauri::command]
fn get_endpoint(state: tauri::State<'_, Connection>) -> Result<String, String> {
    state
        .0
        .lock()
        .map(|s| s.endpoint.clone())
        .map_err(|_| "connection state unavailable".into())
}

#[tauri::command]
fn set_endpoint(
    app: tauri::AppHandle,
    state: tauri::State<'_, Connection>,
    endpoint: String,
    subscriptions: tauri::State<'_, events::NativeEvents>,
    window: tauri::WebviewWindow,
    context: events::SubscriptionContext,
) -> Result<(), String> {
    let client = HpsClient::new(&endpoint, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    let endpoint = client.endpoint().as_str().to_owned();
    let mut current = state.0.lock().map_err(|_| "connection state unavailable")?;
    subscriptions
        .validate_context(window.label(), context)
        .map_err(|e| e.message)?;
    subscriptions
        .ensure_target_generation()
        .map_err(|e| e.message)?;
    let directory = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let temporary = directory.join("connection.json.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec(&endpoint).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(temporary, directory.join("connection.json")).map_err(|e| e.to_string())?;
    let store = hps_system::profiles::Store::new(&hps_system::state_dir()?);
    let profiles = store.load()?;
    if profiles.selected.is_some() {
        store.change(profiles.revision, hps_system::profiles::Change::Unselect)?;
    }
    subscriptions.change_target().map_err(|e| e.message)?;
    *current = Target::address(endpoint);
    Ok(())
}

fn supported_api(path: &str) -> bool {
    if path.len() > 4096 {
        return false;
    }
    matches!(
        path,
        "api/v1/status"
            | "api/v1/version"
            | "api/v1/capabilities"
            | "api/v1/config"
            | "api/v1/linux/preflight"
            | "api/v1/linux/capture"
            | "api/v1/inbounds"
            | "api/v1/config/workspace"
            | "api/v1/config/export"
            | "api/v1/providers"
            | "api/v1/subscriptions"
            | "api/v1/groups"
    ) || supported_health_get(path)
        || supported_observability_get(path)
        || events::supported_get(path)
        || supported_connections_get(path)
        || supported_nodes_get(path)
        || supported_rules_get(path)
        || supported_dns_get(path)
        || supported_provider_get(path)
        || supported_epoch_detail(path, "api/v1/config/revisions/")
}
fn supported_observability_get(path: &str) -> bool {
    if path == "api/v1/observability" {
        return true;
    }
    let Some(query) = path.strip_prefix("api/v1/observability?") else {
        return false;
    };
    let pairs = url::form_urlencoded::parse(query.as_bytes()).collect::<Vec<_>>();
    pairs.len() == 1
        && pairs[0].0 == "window_secs"
        && pairs[0]
            .1
            .parse::<u32>()
            .is_ok_and(|n| (60..=3600).contains(&n))
}
fn supported_epoch_detail(path: &str, prefix: &str) -> bool {
    let (base, query) = path
        .split_once('?')
        .map_or((path, None), |(base, q)| (base, Some(q)));
    base.strip_prefix(prefix)
        .is_some_and(|id| id.parse::<i64>().is_ok_and(|id| id > 0))
        && query.is_none_or(|q| {
            let pairs = url::form_urlencoded::parse(q.as_bytes()).collect::<Vec<_>>();
            pairs.len() == 1
                && pairs[0].0 == "expected_epoch"
                && hps_domain::valid_connection_epoch(&pairs[0].1)
        })
}
fn supported_connections_get(path: &str) -> bool {
    let (base, query) = path
        .split_once('?')
        .map_or((path, None), |(base, q)| (base, Some(q)));
    if base == "api/v1/connections/summary" {
        return query.is_none();
    }
    if let Some(id) = base.strip_prefix("api/v1/connections/") {
        let encoded = format!("id={id}");
        let pairs = url::form_urlencoded::parse(encoded.as_bytes()).collect::<Vec<_>>();
        return query.is_none()
            && pairs.len() == 1
            && pairs[0].0 == "id"
            && hps_domain::valid_connection_id(&pairs[0].1);
    }
    if base != "api/v1/connections" {
        return false;
    }
    query.is_none_or(|q| {
        let mut keys = std::collections::HashSet::new();
        !q.is_empty()
            && url::form_urlencoded::parse(q.as_bytes()).all(|(key, value)| {
                keys.insert(key.to_string())
                    && match key.as_ref() {
                        "snapshot" | "expected_epoch" => hps_domain::valid_connection_epoch(&value),
                        "id" => hps_domain::valid_connection_id(&value),
                        "source" => !value.is_empty() && value.len() <= 64,
                        "source_port" | "destination_port" => {
                            value.parse::<u16>().is_ok_and(|p| p > 0)
                        }
                        "destination" => {
                            !value.is_empty()
                                && value.len() <= 256
                                && value.is_ascii()
                                && !value.chars().any(char::is_control)
                        }
                        "inbound" => value == "@unknown" || valid_resource_id(&value),
                        "node" => value == "@direct" || hps_config::valid_node_id(&value),
                        "provider" => value == "@static" || hps_config::valid_source_id(&value),
                        "group" => value == "@none" || valid_resource_id(&value),
                        "rule" => value == "@default" || valid_resource_id(&value),
                        "protocol" => {
                            ["http", "https", "socks5", "transparent"].contains(&value.as_ref())
                        }
                        "transport" => ["tcp", "udp"].contains(&value.as_ref()),
                        "origin" => {
                            ["explicit", "linux_local", "gateway"].contains(&value.as_ref())
                        }
                        "state" => [
                            "active",
                            "all",
                            "ended",
                            "open",
                            "dialing",
                            "failed",
                            "rejected",
                            "disconnected",
                            "closed",
                            "disconnecting",
                        ]
                        .contains(&value.as_ref()),
                        "error" => {
                            !value.is_empty()
                                && value.len() <= 64
                                && value.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                        }
                        "created_after_unix_ms" => value.parse::<u64>().is_ok(),
                        "sort" => ["newest", "oldest", "bytes_up", "bytes_down", "elapsed"]
                            .contains(&value.as_ref()),
                        "offset" => value.parse::<u32>().is_ok_and(|o| o <= 10512),
                        "limit" => value.parse::<u32>().is_ok_and(|n| (1..=200).contains(&n)),
                        _ => false,
                    }
            })
    })
}
fn supported_nodes_get(path: &str) -> bool {
    let (base, query) = path
        .split_once('?')
        .map_or((path, None), |(base, q)| (base, Some(q)));
    if let Some(id) = base.strip_prefix("api/v1/nodes/") {
        return query.is_none() && hps_config::valid_node_id(id);
    }
    if base != "api/v1/nodes" {
        return false;
    }
    query.is_none_or(|q| {
        let mut keys = std::collections::HashSet::new();
        !q.is_empty()
            && url::form_urlencoded::parse(q.as_bytes()).all(|(key, value)| {
                keys.insert(key.to_string())
                    && match key.as_ref() {
                        "id" => hps_config::valid_node_id(&value),
                        "search" => value.len() <= 256,
                        "provider" if value == "@static" => true,
                        "provider" => hps_config::valid_source_id(&value),
                        "group" => hps_config::valid_node_id(&value) && !value.contains(':'),
                        "protocol" => ["http", "https", "socks5"].contains(&value.as_ref()),
                        "capability" => ["tcp", "udp", "ipv4", "ipv6"].contains(&value.as_ref()),
                        "state" => [
                            "eligible",
                            "blocked",
                            "expired",
                            "near_expiry",
                            "disabled",
                            "draining",
                            "deleted",
                            "healthy",
                            "unhealthy",
                            "unknown",
                            "retired",
                        ]
                        .contains(&value.as_ref()),
                        "sort" => ["id", "expiry_asc", "expiry_desc"].contains(&value.as_ref()),
                        "remaining_max_secs" => value.parse::<u64>().is_ok(),
                        "offset" => value.parse::<u32>().is_ok_and(|v| v <= 300000),
                        "limit" => value.parse::<u32>().is_ok_and(|v| (1..=200).contains(&v)),
                        _ => false,
                    }
            })
    })
}
fn supported_dns_get(path: &str) -> bool {
    let (base, query) = path
        .split_once('?')
        .map_or((path, None), |(base, q)| (base, Some(q)));
    if base == "api/v1/dns/context" {
        let Some(query) = query else { return false };
        let pairs = url::form_urlencoded::parse(query.as_bytes()).collect::<Vec<_>>();
        return pairs.len() == 1
            && pairs[0].0 == "ip"
            && pairs[0].1.parse::<std::net::IpAddr>().is_ok();
    }
    if base != "api/v1/dns" {
        return false;
    }
    query.is_none_or(|query| {
        let mut keys = std::collections::HashSet::new();
        !query.is_empty()
            && url::form_urlencoded::parse(query.as_bytes()).all(|(key, value)| {
                keys.insert(key.to_string())
                    && match key.as_ref() {
                        "limit" => value.parse::<u32>().is_ok_and(|v| (1..=200).contains(&v)),
                        "after" => {
                            value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
                        }
                        _ => false,
                    }
            })
    })
}
fn supported_rules_get(path: &str) -> bool {
    if path == "api/v1/rules" || path == "api/v1/rules/export" {
        return true;
    }
    let Some(query) = path
        .strip_prefix("api/v1/rules?")
        .or_else(|| path.strip_prefix("api/v1/rules/export?"))
    else {
        return false;
    };
    let pairs = url::form_urlencoded::parse(query.as_bytes()).collect::<Vec<_>>();
    pairs.len() == 1 && pairs[0].0 == "revision" && pairs[0].1.parse::<i64>().is_ok_and(|id| id > 0)
}

fn supported_health_get(path: &str) -> bool {
    let (base, query) = path
        .split_once('?')
        .map_or((path, None), |(base, q)| (base, Some(q)));
    if let Some(id) = base.strip_prefix("api/v1/health/tasks/") {
        return !id.is_empty() && supported_epoch_detail(path, "api/v1/health/tasks/");
    }
    if base == "api/v1/health/history" {
        return query.is_none_or(|query| {
            let mut keys = std::collections::HashSet::new();
            !query.is_empty()
                && url::form_urlencoded::parse(query.as_bytes()).all(|(key, value)| {
                    keys.insert(key.to_string())
                        && match key.as_ref() {
                            "node" => hps_config::valid_node_id(&value),
                            "since_unix" => value.parse::<i64>().is_ok_and(|n| n >= 0),
                            "before" => value.parse::<i64>().is_ok_and(|n| n > 0),
                            "limit" => value.parse::<u32>().is_ok_and(|n| (1..=200).contains(&n)),
                            _ => false,
                        }
                })
        });
    }
    if base != "api/v1/health" {
        return false;
    }
    query.is_none_or(|query| {
        let mut keys = std::collections::HashSet::new();
        !query.is_empty()
            && url::form_urlencoded::parse(query.as_bytes()).all(|(key, value)| {
                keys.insert(key.to_string())
                    && match key.as_ref() {
                        "limit" => value.parse::<u32>().is_ok_and(|n| (1..=200).contains(&n)),
                        "node" => hps_config::valid_node_id(&value),
                        "after" => {
                            !value.is_empty()
                                && value.len() <= 128
                                && value
                                    .bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"_:-".contains(&b))
                        }
                        _ => false,
                    }
            })
    })
}

fn valid_resource_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn supported_provider_get(path: &str) -> bool {
    let (base, query) = path
        .split_once('?')
        .map_or((path, None), |(base, query)| (base, Some(query)));
    let parts: Vec<_> = base.split('/').collect();
    match parts.as_slice() {
        ["api", "v1", "provider-tasks", id] => {
            query.is_none() && id.parse::<i64>().is_ok_and(|n| n > 0)
        }
        ["api", "v1", "providers", id] => query.is_none() && valid_resource_id(id),
        ["api", "v1", "providers", id, "controller"] => query.is_none() && valid_resource_id(id),
        ["api", "v1", "providers", id, "leases"] => {
            valid_resource_id(id)
                && query.is_none_or(|q| {
                    !q.is_empty()
                        && q.split('&').all(|pair| {
                            pair.split_once('=').is_some_and(|(k, v)| {
                                matches!(k, "before" | "limit")
                                    && v.parse::<i64>()
                                        .is_ok_and(|n| n > 0 && (k != "limit" || n <= 200))
                            })
                        })
                })
        }
        _ => false,
    }
}

fn supported_action(path: &str) -> bool {
    if path.len() > 4096 {
        return false;
    }
    if matches!(
        path,
        "api/v1/health/probe"
            | "api/v1/linux/capture/control"
            | "api/v1/linux/capture/save"
            | "api/v1/tasks"
            | "api/v1/rules/save"
            | "api/v1/rules/import/preview"
            | "api/v1/rules/import/apply"
            | "api/v1/rules/upsert"
            | "api/v1/rules/delete"
            | "api/v1/rules/toggle"
            | "api/v1/rules/reorder"
            | "api/v1/rules/apply"
            | "api/v1/rules/simulate"
            | "api/v1/nodes/action"
            | "api/v1/nodes/import/preview"
            | "api/v1/nodes/import/apply"
            | "api/v1/connections/disconnect"
            | "api/v1/connections/disconnect/preview"
            | "api/v1/connections/disconnect/confirm"
            | "api/v1/groups/simulate"
            | "api/v1/groups/save"
            | "api/v1/groups/upsert"
            | "api/v1/groups/delete"
            | "api/v1/groups/pin"
            | "api/v1/dns/resolve"
            | "api/v1/dns/cache/clear"
            | "api/v1/dns/save"
            | "api/v1/health/save"
            | "api/v1/config/validate"
            | "api/v1/config/draft"
            | "api/v1/config/revisions"
            | "api/v1/config/activate"
            | "api/v1/config/rollback"
    ) {
        return true;
    }
    let parts: Vec<_> = path.split('/').collect();
    if matches!(parts.as_slice(),["api","v1","providers",id,"controller","pause"|"resume"|"evaluate"] if valid_resource_id(id))
        || matches!(parts.as_slice(),["api","v1","providers",id,"save"|"delete"|"extract"] if valid_resource_id(id))
        || matches!(parts.as_slice(),["api","v1","subscriptions",id,"save"|"delete"|"refresh"] if valid_resource_id(id))
        || matches!(parts.as_slice(),["api","v1","providers",id,"diagnose","quota"|"whitelist"] if valid_resource_id(id))
    {
        return true;
    }
    matches!(parts.as_slice(),["api","v1","inbounds",id,"start"|"stop"] if !id.is_empty() && id.len()<=64 && id.bytes().all(|c|c.is_ascii_alphanumeric() || c==b'_' || c==b'-'))
}

#[tauri::command]
async fn api_post(
    state: tauri::State<'_, Connection>,
    subscriptions: tauri::State<'_, events::NativeEvents>,
    window: tauri::WebviewWindow,
    context: events::SubscriptionContext,
    path: String,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, NativeApiError> {
    if !supported_action(&path) {
        return Err("unsupported API operation".into());
    }
    let endpoint = {
        let target = state.0.lock().map_err(|_| "connection state unavailable")?;
        subscriptions.validate_context(window.label(), context)?;
        target.clone()
    };
    let timeout = if path == "api/v1/dns/resolve" { 65 } else { 10 };
    let client = endpoint
        .client(Duration::from_secs(timeout))
        .map_err(NativeApiError::from)?;
    let result = match body {
        Some(body) => client.post_body(&path, &body).await,
        None => client.post_json(&path).await,
    }
    .map_err(NativeApiError::from);
    subscriptions.validate_context(window.label(), context)?;
    result
}

#[tauri::command]
async fn save_config_export(
    app: tauri::AppHandle,
    state: tauri::State<'_, Connection>,
    subscriptions: tauri::State<'_, events::NativeEvents>,
    window: tauri::WebviewWindow,
    context: events::SubscriptionContext,
    revision: i64,
) -> Result<bool, String> {
    let endpoint = {
        let target = state.0.lock().map_err(|_| "connection state unavailable")?;
        subscriptions
            .validate_context(window.label(), context)
            .map_err(|e| e.message)?;
        target.clone()
    };
    let export = endpoint
        .client(Duration::from_secs(10))
        .map_err(|error| error.to_string())?
        .export_config(Some(revision))
        .await
        .map_err(|error| error.to_string())?;
    subscriptions
        .validate_context(window.label(), context)
        .map_err(|e| e.message)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!("hps-config-r{revision}.toml"))
        .add_filter("TOML", &["toml"])
        .save_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(path) = receiver.await.map_err(|_| "save dialog did not complete")? else {
        return Ok(false);
    };
    subscriptions
        .validate_context(window.label(), context)
        .map_err(|e| e.message)?;
    let path = path
        .into_path()
        .map_err(|_| "selected file path is unavailable")?;
    tokio::task::spawn_blocking(move || std::fs::write(path, export.toml))
        .await
        .map_err(|_| "file save could not finish")?
        .map_err(|error| error.to_string())?;
    Ok(true)
}

#[tauri::command]
async fn save_rules_export(
    app: tauri::AppHandle,
    state: tauri::State<'_, Connection>,
    subscriptions: tauri::State<'_, events::NativeEvents>,
    window: tauri::WebviewWindow,
    context: events::SubscriptionContext,
    revision: i64,
) -> Result<bool, String> {
    let target = {
        let target = state.0.lock().map_err(|_| "connection state unavailable")?;
        subscriptions
            .validate_context(window.label(), context)
            .map_err(|e| e.message)?;
        target.clone()
    };
    let exported = target
        .client(Duration::from_secs(10))
        .map_err(|e| e.to_string())?
        .export_rules(Some(revision))
        .await
        .map_err(|e| e.to_string())?;
    subscriptions
        .validate_context(window.label(), context)
        .map_err(|e| e.message)?;
    let document =
        serde_json::to_vec_pretty(&exported).map_err(|_| "rule export serialization failed")?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!("hps-rules-r{revision}.json"))
        .add_filter("JSON", &["json"])
        .save_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(path) = receiver.await.map_err(|_| "save dialog did not complete")? else {
        return Ok(false);
    };
    subscriptions
        .validate_context(window.label(), context)
        .map_err(|e| e.message)?;
    let path = path
        .into_path()
        .map_err(|_| "selected file path is unavailable")?;
    tokio::task::spawn_blocking(move || std::fs::write(path, document))
        .await
        .map_err(|_| "file save could not finish")?
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
async fn api_get(
    state: tauri::State<'_, Connection>,
    subscriptions: tauri::State<'_, events::NativeEvents>,
    window: tauri::WebviewWindow,
    context: events::SubscriptionContext,
    path: String,
) -> Result<serde_json::Value, NativeApiError> {
    if !supported_api(&path) {
        return Err("unsupported API path".into());
    }
    let endpoint = {
        let target = state.0.lock().map_err(|_| "connection state unavailable")?;
        subscriptions.validate_context(window.label(), context)?;
        target.clone()
    };
    let result = endpoint
        .client(Duration::from_secs(10))
        .map_err(NativeApiError::from)?
        .get_json(&path)
        .await
        .map_err(NativeApiError::from);
    subscriptions.validate_context(window.label(), context)?;
    result
}

#[tauri::command]
fn get_web_build_id() -> &'static str {
    hps_web_assets::BUILD_ID
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            if hps_system::service::desktop_is_root() {
                return Err("Desktop must run as an ordinary user; use the administrator CLI for system services".into());
            }
            let saved = app
                .path()
                .app_config_dir()
                .ok()
                .and_then(|p| std::fs::read(p.join("connection.json")).ok())
                .and_then(|bytes| serde_json::from_slice::<String>(&bytes).ok())
                .filter(|endpoint| HpsClient::new(endpoint, Duration::from_secs(10)).is_ok());
            let store=hps_system::profiles::Store::new(&hps_system::state_dir()?);
            let profiles=store.load()?;
            let target=match profiles.active() {
                Some(profile) => Target::profile(profile)?,
                None => Target::address(saved.unwrap_or_else(|| "http://127.0.0.1:17900/".into())),
            };
            app.manage(Connection(Mutex::new(target)));
            app.manage(events::NativeEvents::default());
            Ok(())
        })
        .on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Started
                && let Some(state) = webview.app_handle().try_state::<events::NativeEvents>()
            {
                let _ = state.navigate(webview.label());
            }
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed)
                && let Some(state) = window.app_handle().try_state::<events::NativeEvents>()
            {
                let _ = state.navigate(window.label());
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_endpoint,
            set_endpoint,
            api_get,
            api_post,
            save_config_export,
            save_rules_export,
            get_web_build_id,
            local::profiles_get,
            local::profiles_change,
            local::local_context,
            local::local_service_install,
            local::local_service_control,
            local::local_proxy_apply,
            local::local_proxy_restore,
            events::events_context,
            events::events_subscribe,
            events::events_cancel,
            events::events_ack,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_paths_and_structured_errors_keep_snapshot_recovery_consistent() {
        let epoch = "b30e2676-6a33-4cc6-814d-a8ca70a7d151";
        let id = format!("{epoch}:00000000000000000001");
        assert!(supported_api(
            "api/v1/connections?node=%40direct&state=active&offset=0&limit=200"
        ));
        assert!(supported_api(&format!("api/v1/connections/{id}")));
        assert!(supported_api("api/v1/connections/summary"));
        assert!(!supported_api(
            "api/v1/connections/summary?url=https://external"
        ));
        assert!(!supported_api("api/v1/connections?limit=50&limit=200"));
        assert!(!supported_api("api/v1/connections?url=http://external"));
        assert!(!supported_api("api/v1/connections?offset=10513"));
        assert!(supported_action("api/v1/connections/disconnect/confirm"));
        let failure=NativeApiError::from(hps_client::ClientError::Api {status:409,error:serde_json::from_value(serde_json::json!({"code":"connection_snapshot_expired","message":"Review a new snapshot","request_id":"test-request","retryable":false})).unwrap()});
        assert_eq!(failure.status, 409);
        assert_eq!(failure.code, "connection_snapshot_expired");
        assert_eq!(failure.request_id.as_deref(), Some("test-request"));
    }
    #[test]
    fn transport_accepts_implemented_api_only() {
        assert!(supported_api("api/v1/linux/capture"));
        assert!(supported_action("api/v1/linux/capture/control"));
        assert!(supported_action("api/v1/linux/capture/save"));
        assert!(!supported_action("api/v1/linux/capture/unknown"));
        assert!(supported_api("api/v1/status"));
        assert!(supported_api("api/v1/observability?window_secs=300"));
        assert!(!supported_api("api/v1/observability?window_secs=3601"));
        assert!(!supported_api(
            "api/v1/observability?window_secs=60&window_secs=120"
        ));
        let connection = "12345678-1234-4321-8234-123456789abc%3A00000000000000000001";
        assert!(supported_api(&format!("api/v1/connections/{connection}")));
        assert!(!supported_api(&format!(
            "api/v1/connections/{connection}&id=other"
        )));
        assert!(!supported_api(
            "api/v1/connections/12345678-1234-4321-8234-123456789abc%2F00000000000000000001"
        ));
        assert!(supported_api("api/v1/groups"));
        assert!(supported_api("api/v1/rules"));
        assert!(supported_api("api/v1/rules/export?revision=42"));
        assert!(!supported_api(
            "api/v1/rules/export?revision=42&revision=43"
        ));
        assert!(supported_action("api/v1/rules/import/preview"));
        assert!(supported_action("api/v1/rules/import/apply"));
        assert!(supported_api(
            "api/v1/nodes?provider=vendor&state=draining&limit=50&offset=0"
        ));
        assert!(supported_api("api/v1/nodes/lease:42"));
        assert!(!supported_api("api/v1/nodes/lease:042"));
        assert!(!supported_api("api/v1/nodes?offset=0&offset=1"));
        assert!(!supported_api("api/v1/nodes?url=http://external"));
        assert!(!supported_api("api/v1/nodes?limit=201"));
        assert!(supported_action("api/v1/nodes/import/apply"));
        assert!(supported_api("api/v1/rules?revision=42"));
        assert!(!supported_api("api/v1/rules?revision=1&revision=2"));
        assert!(!supported_api("api/v1/rules?revision=0"));
        assert!(!supported_api("api/v1/rules?url=http://external"));
        assert!(supported_action("api/v1/rules/simulate"));
        assert!(supported_action("api/v1/rules/apply"));
        assert!(supported_api("api/v1/subscriptions"));
        assert!(supported_action("api/v1/subscriptions/fixture/save"));
        assert!(supported_action("api/v1/subscriptions/fixture/refresh"));
        assert!(supported_action("api/v1/subscriptions/fixture/delete"));
        assert!(!supported_action("api/v1/subscriptions/fixture/unknown"));
        assert!(!supported_action("api/v1/subscriptions/../refresh"));
        assert!(supported_api(
            "api/v1/nodes?provider=subscription%3Afixture&limit=50"
        ));
        assert!(supported_api(
            "api/v1/nodes/sub:fixture:0123456789abcdef01234567"
        ));
        assert!(supported_action("api/v1/groups/simulate"));
        assert!(supported_action("api/v1/groups/pin"));
        assert!(!supported_action("api/v1/groups/unknown"));
        assert!(supported_api("api/v1/health?after=lease%3A12&limit=50"));
        assert!(supported_api("api/v1/health/tasks/12"));
        assert!(supported_api(
            "api/v1/health/history?node=lease%3A12&limit=100&since_unix=1&before=42"
        ));
        assert!(!supported_api("api/v1/health/history?limit=201"));
        assert!(!supported_api("api/v1/health/history?before=0"));
        assert!(!supported_api("api/v1/health/history?node=one&node=two"));
        for prefix in ["api/v1/health/tasks/", "api/v1/config/revisions/"] {
            let epoch = "12345678-1234-4321-8234-123456789abc";
            assert!(supported_api(&format!("{prefix}12?expected_epoch={epoch}")));
            assert!(!supported_api(&format!("{prefix}12?expected_epoch=bad")));
            assert!(!supported_api(&format!(
                "{prefix}12?expected_epoch={epoch}&expected_epoch={epoch}"
            )));
            assert!(!supported_api(&format!("{prefix}12?url=http://external")));
        }
        assert!(supported_action("api/v1/health/probe"));
        assert!(supported_action("api/v1/health/save"));
        assert!(supported_api("api/v1/dns?limit=50"));
        assert!(supported_api("api/v1/dns/context?ip=2001%3Adb8%3A%3A1"));
        assert!(supported_action("api/v1/dns/resolve"));
        assert!(supported_action("api/v1/dns/cache/clear"));
        assert!(supported_action("api/v1/dns/save"));
        assert!(!supported_api("api/v1/dns?url=http://external"));
        assert!(!supported_api("api/v1/dns?limit=50&limit=100"));
        assert!(!supported_api("api/v1/dns/context?ip=host.example"));
        assert!(!supported_api("api/v1/health?limit=10000"));
        assert!(!supported_api("api/v1/health?url=http://external"));
        assert!(!supported_api("api/v1/health/tasks/1/../2"));
        assert!(supported_api(
            "api/v1/providers/source/leases?before=200&limit=50"
        ));
        assert!(supported_api("api/v1/provider-tasks/1"));
        assert!(supported_api("api/v1/providers/source/controller"));
        assert!(supported_action(
            "api/v1/providers/source/controller/resume"
        ));
        assert!(!supported_action(
            "api/v1/providers/source/controller/reset-budget"
        ));
        assert!(!supported_api(
            "api/v1/providers/source/leases?before=1&url=http://external"
        ));
        assert!(!supported_api("api/v1/providers/../config"));
        assert!(supported_action("api/v1/providers/source/extract"));
        assert!(supported_action(
            "api/v1/providers/source/diagnose/whitelist"
        ));
        assert!(!supported_action(
            "api/v1/providers/source/diagnose/purchase"
        ));
        assert!(!supported_api("https://example.com/api/v1/status"));
        assert!(!supported_api("api/v1/../../private"));
        assert!(supported_action("api/v1/inbounds/browser-http/start"));
        assert!(supported_action("api/v1/inbounds/browser-http/stop"));
        assert!(supported_action("api/v1/config/activate"));
        assert!(supported_api("api/v1/config/revisions/12"));
        assert!(!supported_api("api/v1/config/revisions/1/../../private"));
        assert!(!supported_action("api/v1/inbounds/../start"));
        assert!(!supported_action("api/v1/inbounds/browser/start?other=1"));
    }
}
