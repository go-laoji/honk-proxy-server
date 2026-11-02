use super::*;
use axum::extract::{DefaultBodyLimit, Query};
use hps_domain::{
    ConfigActivationResult, NodeActionInput, NodeActionResult, NodeImportInput, NodeImportPreview,
    NodeQuery, NodesOverview,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/nodes", get(overview))
        .route("/api/v1/nodes/action", post(action))
        .route("/api/v1/nodes/import/preview", post(preview))
        .route("/api/v1/nodes/import/apply", post(import))
        .route("/api/v1/nodes/{id}", get(detail))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
}
fn application(state: &AppState) -> Result<&Arc<Supervisor>, (StatusCode, Json<ApiError>)> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))
}
async fn overview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<NodeQuery>,
) -> ApiResult<NodesOverview> {
    application(&state)?
        .nodes(query)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<NodesOverview> {
    application(&state)?
        .node(id)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn action(
    State(state): State<Arc<AppState>>,
    Json(input): Json<NodeActionInput>,
) -> ApiResult<NodeActionResult> {
    application(&state)?
        .node_action(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn preview(
    State(state): State<Arc<AppState>>,
    Json(input): Json<NodeImportInput>,
) -> ApiResult<NodeImportPreview> {
    application(&state)?
        .preview_nodes(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn import(
    State(state): State<Arc<AppState>>,
    Json(input): Json<NodeImportInput>,
) -> ApiResult<ConfigActivationResult> {
    application(&state)?
        .import_nodes(input)
        .await
        .map(Json)
        .map_err(application_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn real_http_import_preview_explicit_partial_apply_query_controls_and_cas() {
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        let application = Supervisor::boot(
            hps_storage::Store::open(None, config).unwrap(),
            hps_application::StartupOverrides::default(),
        )
        .await
        .unwrap();
        application.confirm_startup().await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(AppState::with_application(application.clone()));
        let server = tokio::spawn(async move {
            axum::serve(listener, crate::router(state)).await.unwrap();
        });
        let client = hps_client::HpsClient::new(
            &format!("http://{address}"),
            std::time::Duration::from_secs(3),
        )
        .unwrap();
        let mut input = NodeImportInput {
            document:
                "http://supplier:top%2Bsecret@127.0.0.1:9\nvmess://unsupported\nsocks5://[::1]:1080"
                    .into(),
            format: "lines".into(),
            default_protocol: "http".into(),
            expected_revision: 1,
            preview_checksum: None,
            selected_rows: vec![],
        };
        let preview = client.preview_nodes(&input).await.unwrap();
        assert_eq!(preview.valid_count, 2);
        assert_eq!(preview.invalid_count, 1);
        assert!(!preview.applicable);
        let json = serde_json::to_string(&preview).unwrap();
        assert!(!json.contains("top%2Bsecret"));
        assert!(!json.contains("supplier"));
        assert!(!format!("{input:?}").contains("top%2Bsecret"));
        input.preview_checksum = Some(preview.checksum);
        assert!(client.import_nodes(&input).await.is_err());
        assert_eq!(
            client
                .nodes(&NodeQuery::default())
                .await
                .unwrap()
                .summary
                .inventory,
            0
        );
        input.selected_rows = vec![1, 3];
        let preview = client.preview_nodes(&input).await.unwrap();
        assert!(preview.applicable);
        input.preview_checksum = Some(preview.checksum);
        let activation = client.import_nodes(&input).await.unwrap();
        assert_eq!(activation.running_revision, 2);
        let list = client
            .nodes(&NodeQuery {
                protocol: Some("http".into()),
                limit: Some(1),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(list.filtered_total, 1);
        assert_eq!(list.summary.inventory, 2);
        let node = &list.nodes[0];
        assert!(node.has_credentials);
        assert!(node.eligible);
        assert_eq!(
            application.current().config.proxy.upstreams[0]
                .credentials
                .as_ref()
                .unwrap()
                .password,
            "top+secret"
        );
        let input = NodeActionInput {
            id: node.id.clone(),
            action: "drain".into(),
            expected_revision: 2,
            expected_control_version: 0,
            lease_id: None,
            batch_id: None,
            provider_id: None,
        };
        client.node_action(&input).await.unwrap();
        assert!(matches!(
            client.node_action(&input).await,
            Err(hps_client::ClientError::Api { status: 409, .. })
        ));
        let detail = client.node(&node.id).await.unwrap();
        assert!(!detail.nodes[0].eligible);
        assert_eq!(detail.nodes[0].operation.version, 1);
        assert!(
            detail.nodes[0]
                .admission_reasons
                .contains(&"draining".into())
        );
        assert!(
            !serde_json::to_string(&detail)
                .unwrap()
                .contains("top+secret")
        );
        assert!(
            client
                .nodes(&NodeQuery {
                    limit: Some(201),
                    ..Default::default()
                })
                .await
                .is_err()
        );
        server.abort();
        application.shutdown().await.unwrap();
    }
}
