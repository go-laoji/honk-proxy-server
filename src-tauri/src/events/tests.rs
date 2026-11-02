use super::*;

#[test]
fn request_context_rejects_old_documents_and_targets_even_after_returning_to_same_address() {
    let events = NativeEvents::default();
    let first = events.context("main").unwrap();
    events.validate_context("main", first).unwrap();
    events.change_target().unwrap();
    events.change_target().unwrap();
    assert!(events.validate_context("main", first).is_err());
    let current = events.context("main").unwrap();
    events.navigate("main").unwrap();
    assert!(events.validate_context("main", current).is_err());
    events
        .validate_context("main", events.context("main").unwrap())
        .unwrap();
}
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinSet,
};

const ONE: &str = "11111111-1111-4111-8111-111111111111";
const TWO: &str = "22222222-2222-4222-8222-222222222222";
struct Fixture {
    endpoint: String,
    active: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
async fn fixture() -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let active = Arc::new(AtomicUsize::new(0));
    let count = active.clone();
    let task = tokio::spawn(async move {
        let mut clients = JoinSet::new();
        loop {
            tokio::select! {
                accepted=listener.accept()=>{
                    let Ok((mut socket,_))=accepted else{return;};let count=count.clone();count.fetch_add(1,Ordering::AcqRel);
                    clients.spawn(async move {
                        let _active=Active(count);let mut head=vec![];let mut buffer=[0;1024];
                        while !head.windows(4).any(|w|w==b"\r\n\r\n") {
                            let Ok(n)=socket.read(&mut buffer).await else{return;};if n==0||head.len()+n>8192{return;}head.extend_from_slice(&buffer[..n]);
                        }
                        if socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.is_err(){return;}
                        for sequence in 0..=1 {
                            let page=hps_domain::EventsOverview {epoch:ONE.into(),observed_at_unix_ms:1,records:vec![],cursor:hps_domain::event_cursor(ONE,sequence),oldest_available_sequence:1,head_sequence:sequence,has_more:false,reset_reason:None,retention_limit:10000};
                            let frame=format!("event: {}\nid: {}\ndata: {}\n\n",if sequence==0{"snapshot"}else{"checkpoint"},page.cursor,serde_json::to_string(&page).unwrap());
                            if socket.write_all(frame.as_bytes()).await.is_err(){return;}
                        }
                        let _=socket.read(&mut buffer).await;
                    });
                },
                _=clients.join_next(),if !clients.is_empty()=>{},
            }
        }
    });
    Fixture {
        endpoint,
        active,
        task,
    }
}
fn channel() -> (Channel<NativeMessage>, mpsc::Receiver<serde_json::Value>) {
    let (tx, rx) = mpsc::channel(8);
    (
        Channel::new(move |body| {
            let value = match body {
                tauri::ipc::InvokeResponseBody::Json(text) => serde_json::from_str(&text).unwrap(),
                tauri::ipc::InvokeResponseBody::Raw(bytes) => {
                    serde_json::from_slice(&bytes).unwrap()
                }
            };
            let _ = tx.try_send(value);
            Ok(())
        }),
        rx,
    )
}
async fn message(rx: &mut mpsc::Receiver<serde_json::Value>) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn idle(fixture: &Fixture, events: &NativeEvents) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.active.load(Ordering::Acquire) != 0 || events.slots.available_permits() != 4 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
fn start(
    events: &NativeEvents,
    fixture: &Fixture,
    context: SubscriptionContext,
    serial: u64,
    id: &str,
) -> mpsc::Receiver<serde_json::Value> {
    let (channel, rx) = channel();
    events
        .start(
            "main".into(),
            StartRequest {
                context,
                serial,
                id: id.into(),
                query: EventQuery::default(),
            },
            channel,
            HpsClient::new(&fixture.endpoint, Duration::from_secs(2)).unwrap(),
        )
        .unwrap();
    rx
}

#[tokio::test]
async fn native_frames_require_exact_ack_and_cancel_joins_real_http_ownership() {
    let fixture = fixture().await;
    let events = NativeEvents::default();
    let context = events.context("main").unwrap();
    let mut rx = start(&events, &fixture, context, 1, ONE);
    let first = message(&mut rx).await;
    assert_eq!(first["payload"]["type"], "frame");
    assert_eq!(first["payload"]["value"]["kind"], "snapshot");
    assert_eq!(first["sequence"], 1);
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(rx.try_recv().is_err());
    assert!(!events.acknowledge("main", context, 1, ONE, 2).unwrap());
    assert!(events.acknowledge("main", context, 1, ONE, 1).unwrap());
    assert!(!events.acknowledge("main", context, 1, ONE, 1).unwrap());
    let second = message(&mut rx).await;
    assert_eq!(second["sequence"], 2);
    assert_eq!(second["payload"]["value"]["kind"], "checkpoint");
    assert!(!events.acknowledge("main", context, 1, ONE, 1).unwrap());
    events
        .cancel("main", context, 1, ONE)
        .unwrap()
        .unwrap()
        .await
        .unwrap();
    idle(&fixture, &events).await;
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn native_navigation_target_changes_and_early_cancellation_reject_late_or_old_intents() {
    let fixture = fixture().await;
    let events = NativeEvents::default();
    let old = events.context("main").unwrap();
    assert!(events.cancel("main", old, 1, ONE).unwrap().is_none());
    let (discard, _) = channel();
    assert!(
        events
            .start(
                "main".into(),
                StartRequest {
                    context: old,
                    serial: 1,
                    id: ONE.into(),
                    query: EventQuery::default()
                },
                discard,
                HpsClient::new(&fixture.endpoint, Duration::from_secs(2)).unwrap()
            )
            .is_err()
    );
    let mut rx = start(&events, &fixture, old, 2, TWO);
    message(&mut rx).await;
    assert!(events.cancel("main", old, 1, ONE).unwrap().is_none());
    events.navigate("main").unwrap();
    idle(&fixture, &events).await;
    let new = events.context("main").unwrap();
    assert_ne!(new, old);
    let (discard, _) = channel();
    assert!(
        events
            .start(
                "main".into(),
                StartRequest {
                    context: old,
                    serial: 3,
                    id: ONE.into(),
                    query: EventQuery::default()
                },
                discard,
                HpsClient::new(&fixture.endpoint, Duration::from_secs(2)).unwrap()
            )
            .is_err()
    );
    let mut rx = start(&events, &fixture, new, 1, ONE);
    message(&mut rx).await;
    assert!(!events.acknowledge("main", old, 2, TWO, 1).unwrap());
    assert!(events.cancel("main", old, 2, TWO).unwrap().is_none());
    events.ensure_target_generation().unwrap();
    events.change_target().unwrap();
    idle(&fixture, &events).await;
    assert_ne!(
        events.context("main").unwrap().target_generation,
        new.target_generation
    );
    assert!(!events.acknowledge("main", new, 1, ONE, 1).unwrap());
}

#[tokio::test]
async fn native_unacknowledged_frames_time_out_with_bounded_channel_and_socket_release() {
    let fixture = fixture().await;
    let events = NativeEvents {
        ack_timeout: Duration::from_millis(30),
        ..Default::default()
    };
    let context = events.context("main").unwrap();
    let mut rx = start(&events, &fixture, context, 1, ONE);
    let first = message(&mut rx).await;
    assert_eq!(first["payload"]["type"], "frame");
    let ended = message(&mut rx).await;
    assert_eq!(ended["payload"]["type"], "error");
    assert_eq!(
        ended["payload"]["value"]["code"],
        "native_event_ack_timeout"
    );
    idle(&fixture, &events).await;
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn native_workers_and_window_intent_records_are_bounded_during_replacements() {
    let fixture = fixture().await;
    let events = NativeEvents::default();
    let mut receivers = vec![];
    for n in 0..4 {
        let label = format!("window-{n}");
        let context = events.context(&label).unwrap();
        let (channel, mut rx) = channel();
        events
            .start(
                label,
                StartRequest {
                    context,
                    serial: 1,
                    id: ONE.into(),
                    query: EventQuery::default(),
                },
                channel,
                HpsClient::new(&fixture.endpoint, Duration::from_secs(2)).unwrap(),
            )
            .unwrap();
        message(&mut rx).await;
        receivers.push(rx);
    }
    assert_eq!(events.slots.available_permits(), 0);
    let context = events.context("extra").unwrap();
    let (channel, _) = channel();
    assert!(
        events
            .start(
                "extra".into(),
                StartRequest {
                    context,
                    serial: 1,
                    id: TWO.into(),
                    query: EventQuery::default()
                },
                channel,
                HpsClient::new(&fixture.endpoint, Duration::from_secs(2)).unwrap()
            )
            .is_err()
    );
    for n in 0..4 {
        events.navigate(&format!("window-{n}")).unwrap();
    }
    idle(&fixture, &events).await;
    for n in 5..16 {
        events.context(&format!("window-{n}")).unwrap();
    }
    assert!(events.context("overflow").is_err());
    assert_eq!(events.inner.lock().unwrap().windows.len(), 16);
    assert!(receivers.iter_mut().all(|rx| rx.try_recv().is_err()));
}

#[test]
fn native_task_event_allowlist_has_bounds_and_never_accepts_streams_as_json_or_arbitrary_urls() {
    for path in [
        "api/v1/tasks",
        "api/v1/tasks?limit=1&kind=provider&state=running",
        "api/v1/tasks?source_ref=provider%3A1",
        "api/v1/tasks/12",
        "api/v1/events?module=configuration&level=info&task_id=2",
    ] {
        assert!(super::super::supported_api(path), "{path}");
    }
    for path in [
        "api/v1/events/stream",
        "api/v1/tasks/01",
        "api/v1/tasks?limit=201",
        "api/v1/tasks?kind=unknown",
        "api/v1/events?limit=1&limit=2",
        "api/v1/events?url=http://outside",
        "api/v1/events?cursor=bad",
    ] {
        assert!(!super::super::supported_api(path), "{path}");
    }
    assert!(super::super::supported_action("api/v1/tasks"));
    assert!(!valid_query(&EventQuery {
        request_id: Some("token=secret".into()),
        ..Default::default()
    }));
}
