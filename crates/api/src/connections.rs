use super::*;
use axum::extract::{DefaultBodyLimit, Query};
use hps_domain::{
    ConnectionDisconnectConfirm, ConnectionDisconnectInput, ConnectionDisconnectPreview,
    ConnectionDisconnectPreviewInput, ConnectionDisconnectResult, ConnectionInfo, ConnectionQuery,
    ConnectionSummary, ConnectionsOverview,
};
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/connections", get(overview))
        .route("/api/v1/connections/summary", get(summary))
        .route("/api/v1/connections/{id}", get(detail))
        .route("/api/v1/connections/disconnect", post(disconnect))
        .route("/api/v1/connections/disconnect/preview", post(preview))
        .route("/api/v1/connections/disconnect/confirm", post(confirm))
        .layer(DefaultBodyLimit::max(128 * 1024))
}
fn application(state: &AppState) -> Result<&Arc<Supervisor>, (StatusCode, Json<ApiError>)> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))
}
async fn overview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ConnectionQuery>,
) -> ApiResult<ConnectionsOverview> {
    application(&state)?
        .connections(query)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn summary(State(state): State<Arc<AppState>>) -> ApiResult<ConnectionSummary> {
    application(&state)?
        .connection_summary()
        .await
        .map(Json)
        .map_err(application_error)
}
async fn detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<ConnectionInfo> {
    application(&state)?
        .connection(id)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn disconnect(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ConnectionDisconnectInput>,
) -> ApiResult<ConnectionDisconnectResult> {
    application(&state)?
        .disconnect_connection(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn preview(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ConnectionDisconnectPreviewInput>,
) -> ApiResult<ConnectionDisconnectPreview> {
    application(&state)?
        .preview_disconnect_connections(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn confirm(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ConnectionDisconnectConfirm>,
) -> ApiResult<ConnectionDisconnectResult> {
    application(&state)?
        .confirm_disconnect_connections(input)
        .await
        .map(Json)
        .map_err(application_error)
}

#[cfg(test)]
mod tests;
