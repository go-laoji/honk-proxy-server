use super::*;
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/linux/capture", get(overview))
        .route("/api/v1/linux/diagnostics", get(diagnostics))
        .route("/api/v1/linux/capture/control", post(control))
        .route("/api/v1/linux/capture/save", post(save))
}
async fn save(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_config::LinuxCaptureSave>,
) -> ApiResult<hps_domain::ConfigRevision> {
    application(&state)?
        .save_linux_capture(input)
        .await
        .map(Json)
        .map_err(application_error)
}
fn application(state: &AppState) -> Result<&Arc<Supervisor>, (StatusCode, Json<ApiError>)> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))
}
async fn overview(
    State(state): State<Arc<AppState>>,
) -> ApiResult<hps_domain::LinuxCaptureOverview<hps_config::LinuxCaptureConfig>> {
    application(&state)?
        .linux_capture_overview()
        .await
        .map(Json)
        .map_err(application_error)
}
async fn control(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_domain::LinuxCaptureRequest>,
) -> ApiResult<hps_domain::LinuxCaptureStatus> {
    application(&state)?
        .control_linux_capture(input)
        .await
        .map(Json)
        .map_err(application_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hps_domain::{LinuxCaptureAction, LinuxCaptureRequest, TaskQuery};
    #[tokio::test]
    async fn capture_client_reports_actual_unconfigured_state_and_persists_control_failure() {
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        config.dns.enabled = false;
        let service = Supervisor::boot(
            hps_storage::Store::open(None, config).unwrap(),
            Default::default(),
        )
        .await
        .unwrap();
        service.confirm_startup().await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(AppState::with_application(service.clone()));
        let server = tokio::spawn(async move {
            axum::serve(listener, crate::router(state)).await.unwrap();
        });
        let client = hps_client::HpsClient::new(
            &format!("http://{address}"),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        let view = client.linux_capture().await.unwrap();
        assert!(!view.status.configured && !view.status.ready);
        assert!(view.status.program_ids.is_empty());
        let mut input = LinuxCaptureRequest {
            expected_epoch: view.epoch.clone(),
            expected_revision: view.running_revision,
            action: LinuxCaptureAction::Start,
        };
        input.expected_revision += 1;
        assert!(matches!(
            client.control_linux_capture(&input).await,
            Err(hps_client::ClientError::Api { status: 409, .. })
        ));
        input.expected_revision = view.running_revision;
        assert!(
            matches!(client.control_linux_capture(&input).await, Err(hps_client::ClientError::Api {status:422,error}) if error.code == "linux_capture_failed")
        );
        let tasks = client
            .tasks(&TaskQuery {
                kind: Some("linux_capture_control".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(tasks.records.len(), 2);
        assert!(
            tasks
                .records
                .iter()
                .all(|t| t.terminal && t.state == "failed")
        );
        assert_eq!(
            tasks.records[0].error_code.as_deref(),
            Some("linux_capture_failed")
        );
        let candidate = client
            .save_linux_capture(&hps_config::LinuxCaptureSave {
                expected_revision: view.running_revision,
                expected_epoch: view.epoch,
                configuration: None,
            })
            .await
            .unwrap();
        assert_eq!(candidate.note, "Linux capture candidate");
        assert!(!client.linux_capture().await.unwrap().status.ready);
        service.shutdown().await.unwrap();
        server.abort();
    }
}

async fn diagnostics(
    State(state): State<Arc<AppState>>,
) -> ApiResult<hps_domain::LinuxDiagnostics> {
    application(&state)?
        .linux_diagnostics()
        .await
        .map(Json)
        .map_err(application_error)
}
