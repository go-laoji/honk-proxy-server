use super::*;
fn path(id: i64, action: &str, epoch: Option<&str>) -> Result<String, ClientError> {
    if id <= 0 || epoch.is_some_and(|e| !hps_domain::valid_connection_epoch(e)) {
        return Err(ClientError::InvalidTaskId);
    }
    Ok(format!(
        "api/v1/{action}/{id}{}",
        epoch.map_or(String::new(), |e| format!("?expected_epoch={e}"))
    ))
}
impl HpsClient {
    pub async fn alerts(
        &self,
        after: Option<u64>,
        limit: u32,
    ) -> Result<hps_domain::AlertFeed, ClientError> {
        self.get_json(&format!(
            "api/v1/observability/alerts?limit={limit}{}",
            after.map_or(String::new(), |n| format!("&after={n}"))
        ))
        .await
    }
    pub async fn save_alerts(
        &self,
        input: &hps_domain::AlertSave,
    ) -> Result<hps_domain::ConfigRevision, ClientError> {
        self.post_body("api/v1/observability/alerts/save", input)
            .await
    }
    pub async fn submit_diagnostic(
        &self,
        input: &hps_domain::DiagnosticInput,
    ) -> Result<hps_domain::TaskInfo, ClientError> {
        self.post_body("api/v1/diagnostics", input).await
    }
    pub async fn diagnostic(
        &self,
        id: i64,
        epoch: Option<&str>,
    ) -> Result<hps_domain::DiagnosticBundle, ClientError> {
        self.get_json(&path(id, "diagnostics", epoch)?).await
    }
    pub async fn task_controls(
        &self,
        id: i64,
        epoch: Option<&str>,
    ) -> Result<hps_domain::TaskControls, ClientError> {
        let mut base = path(id, "tasks", None)?;
        base.push_str("/controls");
        if let Some(epoch) = epoch {
            if !hps_domain::valid_connection_epoch(epoch) {
                return Err(ClientError::InvalidTaskId);
            }
            base.push_str(&format!("?expected_epoch={epoch}"));
        }
        self.get_json(&base).await
    }
    pub async fn cancel_task(
        &self,
        id: i64,
        input: &hps_domain::TaskControlInput,
    ) -> Result<hps_domain::TaskInfo, ClientError> {
        self.post_body(&format!("{}/cancel", path(id, "tasks", None)?), input)
            .await
    }
    pub async fn retry_task(
        &self,
        id: i64,
        input: &hps_domain::TaskRetryInput,
    ) -> Result<hps_domain::TaskInfo, ClientError> {
        self.post_body(&format!("{}/retry", path(id, "tasks", None)?), input)
            .await
    }
}
