use super::*;
use axum::{
    extract::{ConnectInfo, FromRequestParts, MatchedPath, RawPathParams, Request},
    middleware::Next,
    response::Response,
};
use hps_application::{RequestContext, with_request_context};
use hps_storage::tasks::ManagementEvent;

fn operation(route: &str) -> Option<(&'static str, &'static str)> {
    Some(match route {
        "/api/v1/inbounds/{id}/start" => ("inbound_start", "inbound"),
        "/api/v1/inbounds/{id}/stop" => ("inbound_stop", "inbound"),
        "/api/v1/config/validate" => ("configuration_validate", "configuration"),
        "/api/v1/config/draft" => ("configuration_draft", "configuration"),
        "/api/v1/config/revisions" => ("configuration_prepare", "configuration"),
        "/api/v1/config/activate" => ("configuration_activate", "configuration"),
        "/api/v1/config/rollback" => ("configuration_rollback", "configuration"),
        "/api/v1/providers/{id}/save" => ("provider_save", "provider"),
        "/api/v1/providers/{id}/delete" => ("provider_delete", "provider"),
        "/api/v1/providers/{id}/extract" => ("provider_extract", "provider"),
        "/api/v1/providers/{id}/diagnose/{kind}" => ("provider_diagnose", "provider"),
        "/api/v1/providers/{id}/controller/pause" => ("provider_pause", "provider"),
        "/api/v1/providers/{id}/controller/resume" => ("provider_resume", "provider"),
        "/api/v1/providers/{id}/controller/evaluate" => ("provider_evaluate", "provider"),
        "/api/v1/nodes/action" => ("node_action", "node"),
        "/api/v1/nodes/import/preview" => ("node_import_preview", "node"),
        "/api/v1/nodes/import/apply" => ("node_import_apply", "node"),
        "/api/v1/groups/simulate" => ("group_simulate", "group"),
        "/api/v1/groups/save" => ("group_save", "group"),
        "/api/v1/groups/upsert" => ("group_upsert", "group"),
        "/api/v1/groups/delete" => ("group_delete", "group"),
        "/api/v1/groups/pin" => ("group_pin", "group"),
        "/api/v1/rules/save" => ("rules_save", "routing"),
        "/api/v1/rules/import/preview" => ("rules_import_preview", "routing"),
        "/api/v1/rules/import/apply" => ("rules_import_candidate", "routing"),
        "/api/v1/rules/upsert" => ("rule_upsert", "routing"),
        "/api/v1/rules/delete" => ("rule_delete", "routing"),
        "/api/v1/rules/toggle" => ("rule_toggle", "routing"),
        "/api/v1/rules/reorder" => ("rules_reorder", "routing"),
        "/api/v1/rules/apply" => ("rules_apply", "routing"),
        "/api/v1/rules/simulate" => ("rules_simulate", "routing"),
        "/api/v1/health/probe" => ("health_probe", "health"),
        "/api/v1/health/save" => ("health_save", "health"),
        "/api/v1/dns/resolve" => ("dns_diagnose", "dns"),
        "/api/v1/dns/cache/clear" => ("dns_cache_clear", "dns"),
        "/api/v1/dns/save" => ("dns_save", "dns"),
        "/api/v1/connections/disconnect" => ("connection_disconnect", "connection"),
        "/api/v1/connections/disconnect/preview" => ("connection_disconnect_preview", "connection"),
        "/api/v1/connections/disconnect/confirm" => ("connection_disconnect_confirm", "connection"),
        "/api/v1/tasks" => ("task_submit", "task"),
        _ => return None,
    })
}
pub async fn management(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() != axum::http::Method::POST {
        return next.run(request).await;
    }
    let Some(application) = state.application.clone() else {
        return next.run(request).await;
    };
    let Some((action, kind)) = request
        .extensions()
        .get::<MatchedPath>()
        .and_then(|r| operation(r.as_str()))
    else {
        return next.run(request).await;
    };
    let peer = request
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|p| p.0);
    let (mut parts, body) = request.into_parts();
    // Axum decodes only the matched path parameters. Request bodies and query strings are not logged.
    let id = RawPathParams::from_request_parts(&mut parts, &state)
        .await
        .ok()
        .and_then(|p| {
            p.iter()
                .find(|(key, _)| *key == "id")
                .map(|(_, value)| value.to_owned())
        })
        .filter(|s| {
            s.len() <= 128
                && !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
        });
    let request = Request::from_parts(parts, body);
    let request_id = uuid::Uuid::new_v4().to_string();
    let event = |outcome| ManagementEvent {
        request_id: request_id.clone(),
        peer,
        action,
        outcome,
        resource_kind: Some(kind.into()),
        resource_id: id.clone(),
        node_id: if kind == "node" { id.clone() } else { None },
        provider_id: if kind == "provider" { id.clone() } else { None },
        configuration_revision: None,
    };
    let _ = application.record_management_event(event("started")).await;
    let context = RequestContext {
        request_id: request_id.clone(),
        peer,
    };
    let mut response = with_request_context(context, next.run(request)).await;
    let status = response.status();
    let outcome = if status == StatusCode::ACCEPTED {
        "accepted"
    } else if status.is_success() {
        "succeeded"
    } else if status.is_client_error() {
        "rejected"
    } else {
        "failed"
    };
    let _ = application.record_management_event(event(outcome)).await;
    response
        .headers_mut()
        .insert("x-request-id", request_id.parse().expect("UUID header"));
    response
}
