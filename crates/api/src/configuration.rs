use super::{ApiResult, AppState, application_error};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use hps_application::{ApplicationError, Supervisor};
use hps_domain::{
    ConfigActivate, ConfigActivationResult, ConfigDraft, ConfigExport, ConfigInput, ConfigRevision,
    ConfigValidation, ConfigWorkspace,
};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/config/workspace", get(workspace))
        .route("/api/v1/config/validate", post(validate))
        .route("/api/v1/config/draft", post(draft))
        .route("/api/v1/config/revisions", post(create_revision))
        .route("/api/v1/config/revisions/{revision}", get(revision))
        .route("/api/v1/config/activate", post(activate))
        .route("/api/v1/config/rollback", post(rollback))
        .route("/api/v1/config/export", get(export))
}
fn application(
    state: &AppState,
) -> Result<&Arc<Supervisor>, (axum::http::StatusCode, Json<hps_domain::ApiError>)> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))
}
async fn workspace(State(state): State<Arc<AppState>>) -> ApiResult<ConfigWorkspace> {
    application(&state)?
        .workspace()
        .await
        .map(Json)
        .map_err(application_error)
}
async fn validate(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ConfigInput>,
) -> ApiResult<ConfigValidation> {
    Ok(Json(application(&state)?.validate(input).await))
}
async fn draft(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ConfigInput>,
) -> ApiResult<ConfigDraft> {
    application(&state)?
        .save_draft(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn create_revision(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ConfigInput>,
) -> ApiResult<ConfigRevision> {
    application(&state)?
        .create_revision(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn activate(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ConfigActivate>,
) -> ApiResult<ConfigActivationResult> {
    application(&state)?
        .activate(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn rollback(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ConfigActivate>,
) -> ApiResult<ConfigActivationResult> {
    application(&state)?
        .rollback(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn revision(
    State(state): State<Arc<AppState>>,
    Path(revision): Path<i64>,
) -> ApiResult<ConfigExport> {
    let toml = application(&state)?
        .export_revision(revision)
        .await
        .map_err(application_error)?;
    Ok(Json(ConfigExport {
        format_version: 1,
        revision,
        toml,
    }))
}
#[derive(serde::Deserialize)]
struct ExportQuery {
    revision: Option<i64>,
}
async fn export(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ExportQuery>,
) -> ApiResult<ConfigExport> {
    let service = application(&state)?;
    let (revision, toml) = if let Some(revision) = query.revision {
        (
            revision,
            service
                .export_revision(revision)
                .await
                .map_err(application_error)?,
        )
    } else {
        let workspace = service.workspace().await.map_err(application_error)?;
        (workspace.desired_revision, workspace.desired_toml)
    };
    Ok(Json(ConfigExport {
        format_version: 1,
        revision,
        toml,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn real_http_config_write_conflict_activation_export_and_known_good_rollback() {
        let store = hps_storage::Store::open(None, hps_config::DaemonConfig::default()).unwrap();
        let service = Supervisor::boot(store, hps_application::StartupOverrides::default())
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
        let workspace = client.config_workspace().await.unwrap();
        assert_eq!(workspace.desired_revision, 1);
        let mut input = ConfigInput {
            toml: workspace
                .desired_toml
                .replace("honk-proxy-server", "changed-via-api"),
            expected_revision: 1,
            expected_draft_version: Some(1),
            note: "API edit".into(),
            use_draft_credentials: false,
        };
        assert!(client.validate_config(&input).await.unwrap().valid);
        client.save_draft(&input).await.unwrap();
        assert!(matches!(
            client.save_draft(&input).await,
            Err(hps_client::ClientError::Api { status: 409, .. })
        ));
        input.expected_draft_version = Some(2);
        let revision = client.create_revision(&input).await.unwrap();
        assert!(revision.applied_at.is_none());
        assert!(matches!(
            client
                .rollback_config(&ConfigActivate {
                    revision: revision.id,
                    expected_revision: 1,
                    defer_until_restart: false
                })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        let activated = client
            .activate_config(&ConfigActivate {
                revision: revision.id,
                expected_revision: 1,
                defer_until_restart: false,
            })
            .await
            .unwrap();
        assert_eq!(activated.state, "applied");
        assert_eq!(
            client.status().await.unwrap().instance_name,
            "changed-via-api"
        );
        assert_eq!(
            client.config().await.unwrap().instance_name,
            "changed-via-api"
        );
        let export = client.export_config(Some(revision.id)).await.unwrap();
        assert_eq!(export.format_version, 1);
        assert!(export.toml.contains("changed-via-api"));
        client
            .rollback_config(&ConfigActivate {
                revision: 1,
                expected_revision: revision.id,
                defer_until_restart: false,
            })
            .await
            .unwrap();
        assert_eq!(
            client.status().await.unwrap().instance_name,
            "honk-proxy-server"
        );
        service.shutdown().await.unwrap();
        server.abort();
    }
}
