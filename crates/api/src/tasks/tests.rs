use super::*;
use hps_application::StartupOverrides;
use hps_domain::{ConfigInput, TaskOperation, parse_event_cursor};
use hps_storage::Store;
use tokio::net::TcpListener;

struct Fixture {
    service: Arc<Supervisor>,
    endpoint: String,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn new_fixture() -> Fixture {
    let store = Store::open(None, DaemonConfig::default()).unwrap();
    let service = Supervisor::boot(store, StartupOverrides::default())
        .await
        .unwrap();
    let app = super::super::router(Arc::new(AppState::with_application(service.clone())));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Fixture {
        service,
        endpoint,
        server,
    }
}
fn submission() -> TaskSubmission {
    TaskSubmission {request_id:"http-prepare-one".into(),operation:TaskOperation::ConfigurationPrepare(ConfigInput {
        toml:"[[proxy.upstreams]]\nid='private-test'\nprotocol='http'\nhost='127.0.0.1'\nport=9\n[proxy.upstreams.credentials]\nusername='fixture-user'\npassword='fixture-credential'\n".into(),
        expected_revision:1,expected_draft_version:None,note:"private-test-note".into(),use_draft_credentials:false,
    })}
}
async fn frame(response: &mut reqwest::Response, buffer: &mut String) -> (String, EventsOverview) {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let Some(end) = buffer.find("\n\n") {
                let block = buffer[..end].to_owned();
                buffer.drain(..end + 2);
                let kind = block
                    .lines()
                    .find_map(|s| s.strip_prefix("event: "))
                    .unwrap_or("message")
                    .to_owned();
                let data = block
                    .lines()
                    .filter_map(|s| s.strip_prefix("data: "))
                    .collect::<Vec<_>>()
                    .join("\n");
                if data.is_empty() {
                    continue;
                }
                let result: EventsOverview = serde_json::from_str(&data).unwrap();
                let id = block.lines().find_map(|s| s.strip_prefix("id: ")).unwrap();
                assert_eq!(id, result.cursor);
                return (kind, result);
            }
            let chunk = response
                .chunk()
                .await
                .unwrap()
                .expect("event stream ended before frame");
            buffer.push_str(std::str::from_utf8(&chunk).unwrap());
            assert!(buffer.len() < 512 * 1024);
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn real_http_tasks_sse_reconnect_and_capacity_release_preserve_actual_facts() {
    let fixture = new_fixture().await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let url = |path: &str| format!("{}{path}", fixture.endpoint);
    let mut stream = client
        .get(url("/api/v1/events/stream"))
        .send()
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    assert_eq!(stream.headers()["content-type"], "text/event-stream");
    assert_eq!(stream.headers()["cache-control"], "no-store");
    let mut buffer = String::new();
    let (kind, initial) = frame(&mut stream, &mut buffer).await;
    assert_eq!(kind, "snapshot");
    assert!(initial.records.is_empty());
    let status: ServiceStatus = client
        .get(url("/api/v1/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(initial.epoch, status.instance_id);
    let input = submission();
    let response = client
        .post(url("/api/v1/tasks"))
        .json(&input)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let accepted: TaskInfo = response.json().await.unwrap();
    let duplicate: TaskInfo = client
        .post(url("/api/v1/tasks"))
        .json(&input)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(accepted.id, duplicate.id);
    let complete = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let task: TaskInfo = client
                .get(url(&format!(
                    "/api/v1/tasks/{}?expected_epoch={}",
                    accepted.id, initial.epoch
                )))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if task.terminal {
                return task;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(complete.state, "completed");
    assert_eq!(complete.result_ref.as_deref(), Some("config:2"));
    let (_, observed) = frame(&mut stream, &mut buffer).await;
    assert!(
        observed
            .records
            .iter()
            .any(|e| e.task_id == Some(accepted.id) && e.code == "task_completed")
    );
    let text =
        serde_json::to_string(&observed).unwrap() + &serde_json::to_string(&complete).unwrap();
    assert!(!text.contains("fixture-credential"));
    assert!(!text.contains("private-test-note"));
    let audit: EventsOverview = client
        .get(url(&format!(
            "/api/v1/events?module=management&request_id={request_id}"
        )))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(audit.records.len(), 2);
    assert_eq!(audit.records[0].outcome.as_deref(), Some("started"));
    assert_eq!(audit.records[1].outcome.as_deref(), Some("accepted"));
    assert!(audit.records.iter().all(|e| {
        e.peer.as_ref().is_some_and(|p| p.starts_with("127.0.0.1:"))
            && e.action.as_deref() == Some("task_submit")
    }));
    let list: TasksOverview = client
        .get(url(
            "/api/v1/tasks?kind=configuration_prepare&request_id=http-prepare-one&limit=1",
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.records.len(), 1);
    assert_eq!(list.records[0].id, accepted.id);
    assert!(list.records[0].result.is_none());
    let mut changed = input;
    let TaskOperation::ConfigurationPrepare(ref mut config) = changed.operation else {
        unreachable!()
    };
    config.note = "changed".into();
    assert_eq!(
        client
            .post(url("/api/v1/tasks"))
            .json(&changed)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let stale = client
        .get(url(&format!(
            "/api/v1/tasks/{}?expected_epoch={}",
            accepted.id,
            uuid::Uuid::new_v4()
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale.json::<ApiError>().await.unwrap().code,
        "task_instance_changed"
    );
    // EventSource reconnects retain their original URL cursor and advance Last-Event-ID.
    let checkpoint: EventsOverview = client
        .get(url("/api/v1/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut resumed = client
        .get(url(&format!(
            "/api/v1/events/stream?cursor={}",
            initial.cursor
        )))
        .header("last-event-id", &checkpoint.cursor)
        .send()
        .await
        .unwrap();
    let (_, fresh) = frame(&mut resumed, &mut String::new()).await;
    assert!(fresh.records.is_empty());
    assert_eq!(fresh.cursor, checkpoint.cursor);
    drop(resumed);
    let other = new_fixture().await;
    let mut switched = client
        .get(format!("{}/api/v1/events/stream", other.endpoint))
        .header("last-event-id", &observed.cursor)
        .send()
        .await
        .unwrap();
    let (kind, new) = frame(&mut switched, &mut String::new()).await;
    assert_eq!(kind, "reset");
    assert_eq!(new.reset_reason.as_deref(), Some("instance_changed"));
    assert_ne!(new.epoch, observed.epoch);
    assert!(new.records.is_empty());
    drop(switched);
    other.service.shutdown().await.unwrap();
    assert_eq!(
        parse_event_cursor(&fresh.cursor).unwrap().0,
        fixture.service.event_epoch()
    );
    let mut subscriptions = vec![];
    // The first stream is still open. Read initial frames so every response owns its permit.
    tokio::time::sleep(Duration::from_millis(30)).await;
    for _ in 0..15 {
        let mut response = client
            .get(url("/api/v1/events/stream"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        frame(&mut response, &mut String::new()).await;
        subscriptions.push(response);
    }
    let full = client
        .get(url("/api/v1/events/stream"))
        .send()
        .await
        .unwrap();
    assert_eq!(full.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        full.json::<ApiError>().await.unwrap().code,
        "task_event_capacity"
    );
    drop(subscriptions);
    drop(stream);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let mut permits = vec![];
            while let Ok(p) = fixture.service.reserve_event_stream() {
                permits.push(p);
            }
            if permits.len() == 16 {
                break;
            }
            drop(permits);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn management_audit_uses_real_peer_decoded_resources_and_owned_request_correlation() {
    let fixture = new_fixture().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let config = ConfigInput {
        toml: String::new(),
        expected_revision: 1,
        expected_draft_version: None,
        note: "body-private-marker".into(),
        use_draft_credentials: false,
    };
    let response = client
        .post(format!(
            "{}/api/v1/config/validate?token=query-private-marker",
            fixture.endpoint
        ))
        .header("authorization", "Bearer header-private-marker")
        .header("x-forwarded-for", "203.0.113.25")
        .json(&config)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        response
            .json::<hps_domain::ConfigValidation>()
            .await
            .unwrap()
            .valid
    );
    let tasks: TasksOverview = client
        .get(format!("{}/api/v1/tasks?request_id={id}", fixture.endpoint))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tasks.records.len(), 1);
    assert_eq!(tasks.records[0].kind, "configuration_validate");
    let events: EventsOverview = client
        .get(format!(
            "{}/api/v1/events?request_id={id}",
            fixture.endpoint
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        events
            .records
            .iter()
            .any(|e| e.task_id == Some(tasks.records[0].id) && e.code == "task_completed")
    );
    let management = events
        .records
        .iter()
        .filter(|e| e.module == "management")
        .collect::<Vec<_>>();
    assert_eq!(management.len(), 2);
    assert!(
        management
            .iter()
            .all(|e| e.peer.as_ref().unwrap().starts_with("127.0.0.1:"))
    );
    let encoded = client
        .post(format!(
            "{}/api/v1/inbounds/%62rowser/start?token=query-private-marker",
            fixture.endpoint
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(encoded.status(), StatusCode::NOT_FOUND);
    let id = encoded.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(encoded.json::<ApiError>().await.unwrap().request_id, id);
    let events: EventsOverview = client
        .get(format!(
            "{}/api/v1/events?request_id={id}",
            fixture.endpoint
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(events.records.len(), 2);
    assert!(
        events
            .records
            .iter()
            .all(|e| e.resource_id.as_deref() == Some("browser")
                && e.action.as_deref() == Some("inbound_start"))
    );
    assert_eq!(events.records[1].outcome.as_deref(), Some("rejected"));
    let all: EventsOverview = client
        .get(format!("{}/api/v1/events", fixture.endpoint))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = serde_json::to_string(&all).unwrap() + &serde_json::to_string(&tasks).unwrap();
    for secret in [
        "body-private-marker",
        "query-private-marker",
        "header-private-marker",
        "203.0.113.25",
    ] {
        assert!(!text.contains(secret));
    }
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn shared_rust_client_stream_outlives_normal_request_timeout_and_uses_complete_cursors() {
    let fixture = new_fixture().await;
    let client = hps_client::HpsClient::new(&fixture.endpoint, Duration::from_millis(100)).unwrap();
    let first = client.events(&EventQuery::default()).await.unwrap();
    let mut subscription = client
        .subscribe_events(&EventQuery {
            cursor: Some(first.cursor),
            module: Some("configuration".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(matches!(
        subscription.next().await.unwrap(),
        Some(hps_domain::EventFrame::Checkpoint(_))
    ));
    let task = client.submit_task(&submission()).await.unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(4), subscription.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let hps_domain::EventFrame::Events(page) = frame else {
        panic!("expected incremental task events");
    };
    assert!(
        page.records
            .iter()
            .any(|e| e.task_id == Some(task.id) && e.code == "task_completed")
    );
    assert_eq!(subscription.cursor(), Some(page.cursor.as_str()));
    assert_eq!(
        client.task(task.id, Some(&page.epoch)).await.unwrap().state,
        "completed"
    );
    assert_eq!(
        client
            .tasks(&TaskQuery {
                request_id: Some("http-prepare-one".into()),
                ..Default::default()
            })
            .await
            .unwrap()
            .records
            .len(),
        1
    );
    assert!(matches!(
        client.task(0, None).await,
        Err(hps_client::ClientError::InvalidTaskId)
    ));
    assert!(matches!(
        client
            .events(&EventQuery {
                cursor: Some("bad".into()),
                ..Default::default()
            })
            .await,
        Err(hps_client::ClientError::InvalidEventCursor)
    ));
    drop(subscription);
    fixture.service.shutdown().await.unwrap();
}
