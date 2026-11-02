use super::*;
use axum::extract::{DefaultBodyLimit, Query};
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/diagnostics", post(submit))
        .route("/api/v1/diagnostics/{id}", get(export))
        .route("/api/v1/tasks/{id}/controls", get(controls))
        .route("/api/v1/tasks/{id}/cancel", post(cancel))
        .route("/api/v1/tasks/{id}/retry", post(retry))
        .layer(DefaultBodyLimit::max(4096))
}
fn application(state: &AppState) -> Result<&Arc<Supervisor>, (StatusCode, Json<ApiError>)> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))
}
async fn submit(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_domain::DiagnosticInput>,
) -> Result<(StatusCode, Json<hps_domain::TaskInfo>), (StatusCode, Json<ApiError>)> {
    application(&state)?
        .submit_diagnostic(input, build_version())
        .await
        .map(|t| (StatusCode::ACCEPTED, Json(t)))
        .map_err(application_error)
}
async fn export(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(query): Query<super::tasks::EpochQuery>,
) -> ApiResult<hps_domain::DiagnosticBundle> {
    let service = application(&state)?;
    query.check(service)?;
    service
        .diagnostic(id, query.expected_epoch.as_deref())
        .await
        .map(Json)
        .map_err(application_error)
}
async fn controls(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(query): Query<super::tasks::EpochQuery>,
) -> ApiResult<hps_domain::TaskControls> {
    let service = application(&state)?;
    query.check(service)?;
    service
        .task_controls(id, query.expected_epoch.as_deref())
        .await
        .map(Json)
        .map_err(application_error)
}
async fn cancel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(input): Json<hps_domain::TaskControlInput>,
) -> ApiResult<hps_domain::TaskInfo> {
    application(&state)?
        .cancel_task(id, input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn retry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(input): Json<hps_domain::TaskRetryInput>,
) -> Result<(StatusCode, Json<hps_domain::TaskInfo>), (StatusCode, Json<ApiError>)> {
    application(&state)?
        .retry_task(id, input)
        .await
        .map(|t| (StatusCode::ACCEPTED, Json(t)))
        .map_err(application_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn real_http_diagnostic_task_export_is_whitelisted_and_recoverable_without_auth() {
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        config.instance_name = "secret-instance-label".into();
        config.proxy.upstreams.push(
            hps_config::parse_upstream_address(
                "http://private-user:diagnostic-private-password@127.0.0.1:9",
                hps_config::ProxyProtocol::Http,
            )
            .unwrap(),
        );
        config.proxy.upstreams[0].enabled = false;
        let service = Supervisor::boot(
            hps_storage::Store::open(None, config).unwrap(),
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
        let input = hps_domain::DiagnosticInput {
            expected_epoch: service.event_epoch().into(),
            request_id: "http-diagnostic".into(),
            window_secs: 300,
        };
        let task = client.submit_diagnostic(&input).await.unwrap();
        assert_eq!(client.submit_diagnostic(&input).await.unwrap().id, task.id);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if client.task(task.id, None).await.unwrap().terminal {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let exported = client
            .diagnostic(task.id, Some(service.event_epoch()))
            .await
            .unwrap();
        let text = serde_json::to_string(&exported).unwrap();
        for private in [
            "secret-instance-label",
            "private-user",
            "diagnostic-private-password",
            "127.0.0.1",
            "/Users/",
        ] {
            assert!(!text.contains(private), "{private}");
        }
        assert_eq!(exported.format_version, 1);
        assert_eq!(exported.version.web_build_id, build_version().web_build_id);
        assert!(!exported.consistent_database_backup);
        let controls = client
            .task_controls(task.id, Some(service.event_epoch()))
            .await
            .unwrap();
        assert!(!controls.can_cancel && !controls.can_retry);
        assert!(matches!(
            client
                .cancel_task(
                    task.id,
                    &hps_domain::TaskControlInput {
                        expected_epoch: service.event_epoch().into(),
                        expected_version: controls.version
                    }
                )
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        assert!(matches!(
            client
                .diagnostic(task.id, Some(&uuid::Uuid::new_v4().to_string()))
                .await,
            Err(hps_client::ClientError::Api { status: 409, .. })
        ));
        service.shutdown().await.unwrap();
        server.abort();
    }
}
