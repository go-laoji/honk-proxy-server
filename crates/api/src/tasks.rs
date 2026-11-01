use super::*;
use axum::{
    extract::Query,
    http::HeaderMap,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use hps_domain::{EventQuery, EventsOverview, TaskInfo, TaskQuery, TaskSubmission, TasksOverview};
use std::{convert::Infallible, time::Duration};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/tasks", get(overview).post(submit))
        .route("/api/v1/tasks/{id}", get(detail))
        .route("/api/v1/events", get(events))
        .route("/api/v1/events/stream", get(stream))
}

#[cfg(test)]
mod tests;
fn application(state: &AppState) -> Result<&Arc<Supervisor>, (StatusCode, Json<ApiError>)> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))
}
async fn overview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TaskQuery>,
) -> ApiResult<TasksOverview> {
    application(&state)?
        .tasks(query)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn submit(
    State(state): State<Arc<AppState>>,
    Json(input): Json<TaskSubmission>,
) -> Result<(StatusCode, Json<TaskInfo>), (StatusCode, Json<ApiError>)> {
    application(&state)?
        .submit_task(input)
        .await
        .map(|t| (StatusCode::ACCEPTED, Json(t)))
        .map_err(application_error)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EpochQuery {
    expected_epoch: Option<String>,
}
async fn detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(query): Query<EpochQuery>,
) -> ApiResult<TaskInfo> {
    application(&state)?
        .task(id, query.expected_epoch.as_deref())
        .await
        .map(Json)
        .map_err(application_error)
}
async fn events(
    State(state): State<Arc<AppState>>,
    Query(query): Query<EventQuery>,
) -> ApiResult<EventsOverview> {
    application(&state)?
        .events(query)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn stream(
    State(state): State<Arc<AppState>>,
    Query(mut query): Query<EventQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiError>)> {
    let application = application(&state)?.clone();
    if let Some(header) = headers.get("last-event-id") {
        let cursor = header
            .to_str()
            .map_err(|_| application_error(ApplicationError::TaskInput))?;
        // Browser EventSource reconnects keep the original URL but advance this header.
        query.cursor = Some(cursor.into());
    }
    let permit = application
        .reserve_event_stream()
        .map_err(application_error)?;
    let initial = application
        .events(query.clone())
        .await
        .map_err(application_error)?;
    let initial_is_snapshot = query.cursor.is_none();
    let cancel = application.cancellation();
    let stream = async_stream::stream! {
        let _permit=permit;
        let mut next=initial;
        let mut first=true;
        let mut last_head=None;
        loop {
            let kind=if next.reset_reason.is_some(){"reset"}else if first && initial_is_snapshot{"snapshot"}else if next.records.is_empty(){"checkpoint"}else{"events"};
            let immediately=next.has_more;
            if first || last_head!=Some(next.head_sequence) || !next.records.is_empty() || next.reset_reason.is_some() {
                let event=match Event::default().event(kind).id(&next.cursor).json_data(&next) {Ok(e)=>e,Err(_)=>break};
                yield Ok::<Event,Infallible>(event);
            }
            first=false;last_head=Some(next.head_sequence);query.cursor=Some(next.cursor);
            if immediately {
                if cancel.is_cancelled(){break;}
                tokio::task::yield_now().await;
            }else {
                tokio::select! {_=cancel.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(1))=>{}}
            }
            let result=tokio::select! {_=cancel.cancelled()=>break,result=application.events(query.clone())=>result};
            match result {
                Ok(value)=>next=value,
                Err(_)=>{
                    yield Ok(Event::default().event("unavailable").data("{\"code\":\"event_stream_unavailable\"}"));
                    break;
                }
            }
        }
    };
    Ok((
        [("cache-control", "no-store"), ("x-accel-buffering", "no")],
        Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))),
    ))
}
