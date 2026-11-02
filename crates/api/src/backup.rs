use super::*;
use axum::{body::Body, response::Response};
pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/v1/config/backup", post(download))
}
async fn download(
    State(state): State<Arc<AppState>>,
) -> Result<Response, (StatusCode, Json<ApiError>)> {
    let service = state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))?;
    let snapshot = service.database_backup().await.map_err(|error| {
        use hps_storage::backup::BackupError;
        let (status, code) = match error {
            BackupError::Busy => (StatusCode::TOO_MANY_REQUESTS, "backup_capacity"),
            BackupError::Ephemeral => (StatusCode::CONFLICT, "backup_requires_persistent_storage"),
            _ => (StatusCode::SERVICE_UNAVAILABLE, "backup_failed"),
        };
        (
            status,
            Json(ApiError {
                code: code.into(),
                message: error.to_string(),
                request_id: hps_application::current_request_id()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                retryable: false,
            }),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/vnd.sqlite3")
        .header(
            "Content-Disposition",
            "attachment; filename=\"hps-backup.sqlite3\"",
        )
        .header("Cache-Control", "no-store")
        .header(
            "X-HPS-Backup",
            serde_json::to_string(&snapshot.metadata).expect("serializable backup metadata"),
        )
        .body(Body::from_stream(async_stream::stream! {
            // Keep the per-instance snapshot budget through response backpressure/drop.
            // Copy bounded chunks so a final slow frame cannot retain a full 64MiB allocation.
            for chunk in snapshot.bytes.chunks(64*1024) {
                yield Ok::<_,std::convert::Infallible>(axum::body::Bytes::copy_from_slice(chunk));
            }
            drop(snapshot);
        }))
        .expect("valid fixed backup headers"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn unconsumed_response_holds_snapshot_budget_until_body_is_dropped() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        config.dns.enabled = false;
        let service = Supervisor::boot(
            hps_storage::Store::open(Some(directory.path()), config).unwrap(),
            Default::default(),
        )
        .await
        .unwrap();
        service.confirm_startup().await.unwrap();
        let state = Arc::new(AppState::with_application(service.clone()));
        let first = download(State(state.clone())).await.unwrap();
        let (status, error) = download(State(state.clone())).await.unwrap_err();
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error.0.code, "backup_capacity");
        drop(first);
        let third = download(State(state)).await.unwrap();
        assert_eq!(third.status(), StatusCode::OK);
        drop(third);
        service.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn real_http_download_verifies_metadata_and_restore_lock() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        config.dns.enabled = false;
        let service = Supervisor::boot(
            hps_storage::Store::open(Some(directory.path()), config).unwrap(),
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
        let topology = client.linux_diagnostics().await.unwrap();
        assert_eq!(topology.topology.supported, cfg!(target_os = "linux"));
        assert_eq!(topology.epoch, service.event_epoch());
        assert!(topology.capture.program_ids.is_empty());
        let (metadata, bytes) = client.database_backup().await.unwrap();
        assert!(metadata.contains_credentials && metadata.restore_requires_stopped_daemon);
        let export = directory.path().join("download.sqlite3");
        hps_storage::backup::save_new(&export, &bytes).unwrap();
        assert_eq!(
            hps_storage::backup::inspect(&export, Some(&metadata.sha256))
                .unwrap()
                .size_bytes,
            bytes.len() as u64
        );
        assert!(matches!(
            hps_storage::backup::restore(directory.path(), &export, &metadata.sha256),
            Err(hps_storage::backup::BackupError::Running)
        ));
        service.shutdown().await.unwrap();
        server.abort();
        assert!(
            hps_storage::backup::restore(directory.path(), &export, &metadata.sha256)
                .unwrap()
                .restart_required
        );
    }
}
