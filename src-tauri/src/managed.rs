use super::*;
use hps_application::{StartupOverrides, Supervisor};
use hps_config::DaemonConfig;
use hps_daemon::{ManagementServer, ServerError};
use std::{net::SocketAddr, path::PathBuf};
use tokio::{
    sync::{Mutex as AsyncMutex, oneshot},
    task::JoinHandle,
};

pub(super) struct ManagedService {
    directory: PathBuf,
    running: AsyncMutex<Option<Running>>,
}

struct Running {
    endpoint: String,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), ServerError>>,
}

impl Running {
    async fn start(directory: PathBuf) -> Result<Self, String> {
        let listen = SocketAddr::from(([127, 0, 0, 1], 0));
        let mut config = DaemonConfig::from_toml(include_str!("../desktop-service.toml"))
            .map_err(|error| error.to_string())?;
        config.management_listen = listen;
        let store =
            tokio::task::spawn_blocking(move || hps_storage::Store::open(Some(&directory), config))
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
        let application = Supervisor::boot(
            store,
            StartupOverrides {
                listen: Some(listen),
                management_plaintext: true,
                ..Default::default()
            },
        )
        .await
        .map_err(|error| error.to_string())?;
        let server = ManagementServer::bind(application)
            .await
            .map_err(|error| error.to_string())?;
        let endpoint = format!("http://{}/", server.address());
        let (shutdown, receiver) = oneshot::channel();
        let task = tokio::spawn(server.serve(async move {
            let _ = receiver.await;
        }));
        Ok(Self {
            endpoint,
            shutdown,
            task,
        })
    }

    async fn stop(self) -> Result<(), String> {
        let _ = self.shutdown.send(());
        self.task
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }
}

impl ManagedService {
    pub(super) fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            running: AsyncMutex::new(None),
        }
    }

    pub(super) async fn shutdown(&self) -> Result<(), String> {
        stop(&mut *self.running.lock().await).await
    }

    async fn start(&self, running: &mut Option<Running>) -> Result<bool, String> {
        if running
            .as_ref()
            .is_some_and(|service| !service.task.is_finished())
        {
            return Ok(false);
        }
        let _ = stop(running).await;
        *running = Some(Running::start(self.directory.clone()).await?);
        Ok(true)
    }
}

async fn stop(running: &mut Option<Running>) -> Result<(), String> {
    if let Some(running) = running.take() {
        running.stop().await?;
    }
    Ok(())
}

fn checked_target<'a>(
    connection: &'a Connection,
    subscriptions: &events::NativeEvents,
    window: &tauri::WebviewWindow,
    context: events::SubscriptionContext,
) -> Result<std::sync::MutexGuard<'a, Target>, String> {
    let current = connection
        .0
        .lock()
        .map_err(|_| "connection state unavailable")?;
    subscriptions
        .validate_context(window.label(), context)
        .map_err(|error| error.message)?;
    Ok(current)
}

#[derive(serde::Serialize)]
pub(super) struct Status {
    endpoint: Option<String>,
    selected: bool,
}

#[tauri::command]
pub(super) async fn managed_service_status(
    managed: tauri::State<'_, ManagedService>,
    connection: tauri::State<'_, Connection>,
    subscriptions: tauri::State<'_, events::NativeEvents>,
    window: tauri::WebviewWindow,
    context: events::SubscriptionContext,
) -> Result<Status, String> {
    let running = managed.running.lock().await;
    let current = checked_target(&connection, &subscriptions, &window, context)?;
    Ok(Status {
        endpoint: running
            .as_ref()
            .filter(|service| !service.task.is_finished())
            .map(|service| service.endpoint.clone()),
        selected: current.managed,
    })
}

#[tauri::command]
pub(super) async fn managed_service_start(
    managed: tauri::State<'_, ManagedService>,
    connection: tauri::State<'_, Connection>,
    subscriptions: tauri::State<'_, events::NativeEvents>,
    window: tauri::WebviewWindow,
    context: events::SubscriptionContext,
) -> Result<(), String> {
    let mut running = managed.running.lock().await;
    drop(checked_target(
        &connection,
        &subscriptions,
        &window,
        context,
    )?);
    let created = managed.start(&mut running).await?;
    let result = (|| {
        let mut current = checked_target(&connection, &subscriptions, &window, context)?;
        let endpoint = &running.as_ref().expect("managed service started").endpoint;
        if !current.managed || current.endpoint != *endpoint {
            subscriptions
                .change_target()
                .map_err(|error| error.message)?;
            *current = Target {
                managed: true,
                ..Target::address(endpoint.clone())
            };
        }
        Ok(())
    })();
    if result.is_err() && created {
        let _ = stop(&mut running).await;
    }
    result
}

#[tauri::command]
pub(super) async fn managed_service_stop(
    app: tauri::AppHandle,
    managed: tauri::State<'_, ManagedService>,
    connection: tauri::State<'_, Connection>,
    subscriptions: tauri::State<'_, events::NativeEvents>,
    window: tauri::WebviewWindow,
    context: events::SubscriptionContext,
) -> Result<(), String> {
    let mut running = managed.running.lock().await;
    let selected = checked_target(&connection, &subscriptions, &window, context)?.managed;
    if selected {
        let fallback = tokio::task::spawn_blocking(move || {
            saved_target(&app).unwrap_or_else(|_| default_target())
        })
        .await
        .map_err(|error| error.to_string())?;
        let mut current = checked_target(&connection, &subscriptions, &window, context)?;
        subscriptions
            .change_target()
            .map_err(|error| error.message)?;
        *current = fallback;
    }
    stop(&mut running).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use hps_domain::{ConfigDraft, ConfigInput, ConfigWorkspace, ProxyStatus, ServiceStatus};

    #[tokio::test]
    async fn desktop_starts_once_stops_and_keeps_its_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path().join("desktop-service");
        let managed = ManagedService::new(data_dir.clone());
        let external = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = {
            let mut running = managed.running.lock().await;
            assert!(managed.start(&mut running).await.unwrap());
            assert!(!managed.start(&mut running).await.unwrap());
            running.as_ref().unwrap().endpoint.clone()
        };
        assert_ne!(
            endpoint,
            format!("http://{}/", external.local_addr().unwrap())
        );
        assert!(Running::start(data_dir.clone()).await.is_err());
        let client = HpsClient::new(&endpoint, Duration::from_secs(2)).unwrap();
        let before: ServiceStatus = client.get_json("api/v1/status").await.unwrap();
        assert!(before.management_ready);
        assert!(!before.proxy_ready);
        let inbounds: ProxyStatus = client.get_json("api/v1/inbounds").await.unwrap();
        assert_eq!(inbounds.inbounds.len(), 2);
        for inbound in inbounds.inbounds {
            assert!(inbound.running);
            let address: SocketAddr = inbound.listen.unwrap().parse().unwrap();
            assert!(address.ip().is_loopback());
            assert_ne!(address.port(), 0);
        }
        let workspace: ConfigWorkspace = client.get_json("api/v1/config/workspace").await.unwrap();
        let input = ConfigInput {
            toml: "instance_name = \"Desktop draft persists\"".into(),
            expected_revision: workspace.desired_revision,
            expected_draft_version: Some(workspace.draft.version),
            note: String::new(),
            use_draft_credentials: false,
        };
        let draft: ConfigDraft = client
            .post_body("api/v1/config/draft", &input)
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), managed.shutdown())
            .await
            .unwrap()
            .unwrap();
        assert!(
            client
                .get_json::<ServiceStatus>("api/v1/status")
                .await
                .is_err()
        );
        assert!(data_dir.join("hps.sqlite3").is_file());
        let _ = tokio::net::TcpStream::connect(external.local_addr().unwrap())
            .await
            .unwrap();

        let endpoint = {
            let mut running = managed.running.lock().await;
            assert!(managed.start(&mut running).await.unwrap());
            running.as_ref().unwrap().endpoint.clone()
        };
        let restarted = HpsClient::new(&endpoint, Duration::from_secs(2)).unwrap();
        let after: ServiceStatus = restarted.get_json("api/v1/status").await.unwrap();
        assert_ne!(after.instance_id, before.instance_id);
        let workspace: ConfigWorkspace =
            restarted.get_json("api/v1/config/workspace").await.unwrap();
        assert_eq!(workspace.draft.version, draft.version);
        assert!(workspace.draft.toml.contains("Desktop draft persists"));
        managed.shutdown().await.unwrap();
    }
}
