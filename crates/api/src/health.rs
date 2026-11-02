use super::*;
use axum::extract::{DefaultBodyLimit, Query};
use hps_config::{HealthConfig, HealthSave};
use hps_domain::{ConfigActivationResult, HealthOverview, HealthProbeInput, HealthTask};
use serde::Deserialize;
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/health", get(overview))
        .route("/api/v1/health/probe", post(probe))
        .route("/api/v1/health/save", post(save))
        .route("/api/v1/health/tasks/{id}", get(task))
        .layer(DefaultBodyLimit::max(128 * 1024))
}
fn application(state: &AppState) -> Result<&Arc<Supervisor>, (StatusCode, Json<ApiError>)> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    node: Option<String>,
    after: Option<String>,
    limit: Option<u32>,
}
async fn overview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
) -> ApiResult<HealthOverview<HealthConfig>> {
    application(&state)?
        .health_overview_for(query.after, query.limit.unwrap_or(50), query.node)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn probe(
    State(state): State<Arc<AppState>>,
    Json(input): Json<HealthProbeInput>,
) -> Result<(StatusCode, Json<HealthTask>), (StatusCode, Json<ApiError>)> {
    application(&state)?
        .probe_health(input)
        .await
        .map(|task| (StatusCode::ACCEPTED, Json(task)))
        .map_err(application_error)
}
async fn task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(query): Query<super::tasks::EpochQuery>,
) -> ApiResult<HealthTask> {
    let service = application(&state)?;
    query.check(service)?;
    service
        .health_task(id)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn save(
    State(state): State<Arc<AppState>>,
    Json(input): Json<HealthSave>,
) -> ApiResult<ConfigActivationResult> {
    application(&state)?
        .edit_health(input)
        .await
        .map(Json)
        .map_err(application_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn real_http_health_policy_uses_revision_conflicts_and_bounded_inputs() {
        let service = Supervisor::boot(
            hps_storage::Store::open(None, DaemonConfig::default()).unwrap(),
            hps_application::StartupOverrides::default(),
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
            std::time::Duration::from_secs(3),
        )
        .unwrap();
        let overview = client.health(None, 50).await.unwrap();
        assert!(overview.configuration.enabled);
        assert_eq!(overview.total_nodes, 0);
        let mut config = overview.configuration;
        config.enabled = false;
        let saved = client
            .save_health(&HealthSave {
                configuration: config.clone(),
                expected_revision: 1,
            })
            .await
            .unwrap();
        assert_eq!(saved.running_revision, 2);
        assert!(matches!(
            client
                .save_health(&HealthSave {
                    configuration: config.clone(),
                    expected_revision: 1
                })
                .await,
            Err(hps_client::ClientError::Api { status: 409, .. })
        ));
        assert!(!client.health(None, 50).await.unwrap().configuration.enabled);
        assert!(matches!(
            client
                .probe_health(&HealthProbeInput {
                    request_id: "disabled".into(),
                    node_ids: vec![]
                })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        config.enabled = true;
        config.max_parallel = 1000;
        assert!(matches!(
            client
                .save_health(&HealthSave {
                    configuration: config,
                    expected_revision: 2
                })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        assert_eq!(client.config_workspace().await.unwrap().desired_revision, 2);
        service.shutdown().await.unwrap();
        server.abort();
    }
}
