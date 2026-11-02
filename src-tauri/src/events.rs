use super::*;
use hps_domain::{EventFrame, EventQuery};
use std::{collections::HashMap, sync::Arc};
use tauri::ipc::Channel;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SubscriptionContext {
    target_generation: u64,
    document_generation: u64,
}
#[derive(serde::Serialize)]
pub(super) struct NativeMessage {
    id: String,
    context: SubscriptionContext,
    serial: u64,
    sequence: u64,
    payload: Payload,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartRequest {
    context: SubscriptionContext,
    serial: u64,
    id: String,
    query: EventQuery,
}
#[derive(serde::Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
enum Payload {
    Frame(EventFrame),
    Error(NativeApiError),
}
struct Slot {
    id: String,
    serial: u64,
    cancel: oneshot::Sender<()>,
    ack: mpsc::Sender<u64>,
    pending: u64,
    task: tokio::task::JoinHandle<()>,
}
struct WindowStreams {
    document_generation: u64,
    latest_serial: u64,
    slot: Option<Slot>,
}
impl Default for WindowStreams {
    fn default() -> Self {
        Self {
            document_generation: 1,
            latest_serial: 0,
            slot: None,
        }
    }
}
struct Inner {
    target_generation: u64,
    windows: HashMap<String, WindowStreams>,
}
pub(super) struct NativeEvents {
    inner: Arc<Mutex<Inner>>,
    slots: Arc<Semaphore>,
    ack_timeout: Duration,
}
impl Default for NativeEvents {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                target_generation: 1,
                windows: HashMap::new(),
            })),
            slots: Arc::new(Semaphore::new(4)),
            ack_timeout: Duration::from_secs(15),
        }
    }
}
fn error(code: &str, message: &str) -> NativeApiError {
    NativeApiError {
        code: code.into(),
        message: message.into(),
        status: 0,
        request_id: None,
    }
}
fn window<'a>(inner: &'a mut Inner, label: &str) -> Result<&'a mut WindowStreams, NativeApiError> {
    if inner.windows.len() >= 16 && !inner.windows.contains_key(label) {
        return Err(error(
            "native_event_capacity",
            "native event window capacity reached",
        ));
    }
    Ok(inner.windows.entry(label.into()).or_default())
}
fn cancel_slot(slot: Option<Slot>) -> Option<tokio::task::JoinHandle<()>> {
    slot.map(|s| {
        let _ = s.cancel.send(());
        s.task
    })
}
fn current(
    inner: &Inner,
    label: &str,
    context: SubscriptionContext,
    id: &str,
    serial: u64,
) -> bool {
    inner.target_generation == context.target_generation
        && inner.windows.get(label).is_some_and(|w| {
            w.document_generation == context.document_generation
                && w.slot
                    .as_ref()
                    .is_some_and(|s| s.id == id && s.serial == serial)
        })
}
impl NativeEvents {
    fn context(&self, label: &str) -> Result<SubscriptionContext, NativeApiError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| error("native_event_state", "event state unavailable"))?;
        let target_generation = inner.target_generation;
        Ok(SubscriptionContext {
            target_generation,
            document_generation: window(&mut inner, label)?.document_generation,
        })
    }
    pub(super) fn ensure_target_generation(&self) -> Result<(), NativeApiError> {
        if self
            .inner
            .lock()
            .map_err(|_| error("native_event_state", "event state unavailable"))?
            .target_generation
            == u64::MAX
        {
            return Err(error(
                "native_event_generation",
                "event generation exhausted",
            ));
        }
        Ok(())
    }
    pub(super) fn change_target(&self) -> Result<(), NativeApiError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| error("native_event_state", "event state unavailable"))?;
        inner.target_generation = inner
            .target_generation
            .checked_add(1)
            .ok_or_else(|| error("native_event_generation", "event generation exhausted"))?;
        for window in inner.windows.values_mut() {
            cancel_slot(window.slot.take());
        }
        Ok(())
    }
    pub(super) fn navigate(&self, label: &str) -> Result<(), NativeApiError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| error("native_event_state", "event state unavailable"))?;
        let window = window(&mut inner, label)?;
        window.document_generation = window
            .document_generation
            .checked_add(1)
            .ok_or_else(|| error("native_event_generation", "document generation exhausted"))?;
        window.latest_serial = 0;
        cancel_slot(window.slot.take());
        Ok(())
    }
    fn start(
        &self,
        label: String,
        request: StartRequest,
        channel: Channel<NativeMessage>,
        client: HpsClient,
    ) -> Result<(), NativeApiError> {
        let StartRequest {
            context,
            serial,
            id,
            query,
        } = request;
        if !hps_domain::valid_connection_epoch(&id)
            || serial == 0
            || serial > 9_007_199_254_740_991
            || !valid_query(&query)
        {
            return Err(error("native_event_input", "invalid event subscription"));
        }
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            error(
                "native_event_capacity",
                "native event worker capacity reached",
            )
        })?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| error("native_event_state", "event state unavailable"))?;
        if inner.target_generation != context.target_generation {
            return Err(error(
                "native_event_superseded",
                "target changed before subscription",
            ));
        }
        let window = window(&mut inner, &label)?;
        if window.document_generation != context.document_generation
            || serial <= window.latest_serial
        {
            return Err(error(
                "native_event_superseded",
                "document or subscription was superseded",
            ));
        }
        cancel_slot(window.slot.take());
        window.latest_serial = serial;
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let (ack_tx, ack_rx) = mpsc::channel(1);
        let state = self.inner.clone();
        let key = label.clone();
        let identity = id.clone();
        let timeout = self.ack_timeout;
        let task = tokio::spawn(
            Worker {
                state,
                label: key,
                context,
                serial,
                id: identity,
                query,
                channel,
                client,
                cancel: cancel_rx,
                ack: ack_rx,
                _permit: permit,
                timeout,
            }
            .run(),
        );
        window.slot = Some(Slot {
            id,
            serial,
            cancel: cancel_tx,
            ack: ack_tx,
            pending: 0,
            task,
        });
        Ok(())
    }
    fn cancel(
        &self,
        label: &str,
        context: SubscriptionContext,
        serial: u64,
        id: &str,
    ) -> Result<Option<tokio::task::JoinHandle<()>>, NativeApiError> {
        if serial == 0 || serial > 9_007_199_254_740_991 || !hps_domain::valid_connection_epoch(id)
        {
            return Err(error("native_event_input", "invalid cancellation identity"));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| error("native_event_state", "event state unavailable"))?;
        if inner.target_generation != context.target_generation {
            return Ok(None);
        }
        let window = window(&mut inner, label)?;
        if window.document_generation != context.document_generation
            || serial < window.latest_serial
        {
            return Ok(None);
        }
        if serial == window.latest_serial && window.slot.as_ref().is_some_and(|s| s.id != id) {
            return Ok(None);
        }
        window.latest_serial = serial;
        Ok(cancel_slot(window.slot.take()))
    }
    fn acknowledge(
        &self,
        label: &str,
        context: SubscriptionContext,
        serial: u64,
        id: &str,
        sequence: u64,
    ) -> Result<bool, NativeApiError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| error("native_event_state", "event state unavailable"))?;
        if !current(&inner, label, context, id, serial) {
            return Ok(false);
        }
        let slot = inner
            .windows
            .get_mut(label)
            .and_then(|w| w.slot.as_mut())
            .expect("current slot");
        if sequence == 0 || sequence != slot.pending {
            return Ok(false);
        }
        match slot.ack.try_send(sequence) {
            Ok(()) => {
                slot.pending = 0;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
}

struct Worker {
    state: Arc<Mutex<Inner>>,
    label: String,
    context: SubscriptionContext,
    serial: u64,
    id: String,
    query: EventQuery,
    channel: Channel<NativeMessage>,
    client: HpsClient,
    cancel: oneshot::Receiver<()>,
    ack: mpsc::Receiver<u64>,
    _permit: OwnedSemaphorePermit,
    timeout: Duration,
}
impl Worker {
    async fn run(self) {
        let Self {
            state,
            label,
            context,
            serial,
            id,
            query,
            channel,
            client,
            mut cancel,
            mut ack,
            _permit,
            timeout,
        } = self;
        let mut sequence = 0u64;
        let result:Result<(),NativeApiError>=async {
        let mut stream=tokio::select! {_=&mut cancel=>return Ok(()),result=client.subscribe_events(&query)=>result.map_err(NativeApiError::from)?};
        loop {
            let frame=tokio::select! {_=&mut cancel=>return Ok(()),result=stream.next()=>result.map_err(NativeApiError::from)?};
            let Some(frame)=frame else{return Err(error("native_event_ended","event transport ended; reconnect from the last received cursor"));};
            sequence=sequence.checked_add(1).ok_or_else(||error("native_event_generation","frame sequence exhausted"))?;
            {
                let mut inner=state.lock().map_err(|_|error("native_event_state","event state unavailable"))?;
                if !current(&inner,&label,context,&id,serial){return Ok(());}
                inner.windows.get_mut(&label).and_then(|w|w.slot.as_mut()).expect("current slot").pending=sequence;
                channel.send(NativeMessage {id:id.clone(),context,serial,sequence,payload:Payload::Frame(frame)}).map_err(|_|error("native_event_channel","frontend channel closed"))?;
            }
            let received=tokio::select! {_=&mut cancel=>return Ok(()),result=tokio::time::timeout(timeout,ack.recv())=>result};
            match received {Ok(Some(value)) if value==sequence=>{},Ok(None)=>return Ok(()),_=>return Err(error("native_event_ack_timeout","frontend did not acknowledge the bounded event frame"))}
        }
    }.await;
        if let Ok(mut inner) = state.lock()
            && current(&inner, &label, context, &id, serial)
        {
            if let Err(error) = result {
                let _ = channel.send(NativeMessage {
                    id: id.clone(),
                    context,
                    serial,
                    sequence: sequence.saturating_add(1),
                    payload: Payload::Error(error),
                });
            }
            inner
                .windows
                .get_mut(&label)
                .expect("current window")
                .slot
                .take();
        }
    }
}

#[tauri::command]
pub(super) fn events_context(
    state: tauri::State<'_, NativeEvents>,
    window: tauri::WebviewWindow,
) -> Result<SubscriptionContext, NativeApiError> {
    state.context(window.label())
}
#[tauri::command]
pub(super) async fn events_subscribe(
    state: tauri::State<'_, NativeEvents>,
    connection: tauri::State<'_, Connection>,
    window: tauri::WebviewWindow,
    request: StartRequest,
    channel: Channel<NativeMessage>,
) -> Result<(), NativeApiError> {
    let endpoint = connection
        .0
        .lock()
        .map_err(|_| error("native_event_state", "connection state unavailable"))?
        .clone();
    let client = endpoint
        .client(Duration::from_secs(10))
        .map_err(NativeApiError::from)?;
    state.start(window.label().into(), request, channel, client)
}
#[tauri::command]
pub(super) async fn events_cancel(
    state: tauri::State<'_, NativeEvents>,
    window: tauri::WebviewWindow,
    context: SubscriptionContext,
    serial: u64,
    id: String,
) -> Result<(), NativeApiError> {
    let task = state.cancel(window.label(), context, serial, &id)?;
    if let Some(task) = task {
        let _ = task.await;
    }
    Ok(())
}
#[tauri::command]
pub(super) fn events_ack(
    state: tauri::State<'_, NativeEvents>,
    window: tauri::WebviewWindow,
    context: SubscriptionContext,
    serial: u64,
    id: String,
    sequence: u64,
) -> Result<bool, NativeApiError> {
    state.acknowledge(window.label(), context, serial, &id, sequence)
}

fn token(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn event_field(key: &str, value: &str) -> bool {
    match key {
        "cursor" => hps_domain::parse_event_cursor(value).is_some(),
        "level" => ["info", "warn", "error"].contains(&value),
        "module" => hps_domain::EVENT_MODULES.contains(&value),
        "task_id" => value.parse::<i64>().is_ok_and(|n| n > 0),
        "request_id" => token(value, 64),
        "node_id" => hps_config::valid_node_id(value),
        "provider_id" => super::valid_resource_id(value),
        "after_unix_ms" => value.parse::<u64>().is_ok_and(|n| n <= i64::MAX as u64),
        "limit" => value.parse::<u32>().is_ok_and(|n| (1..=200).contains(&n)),
        _ => false,
    }
}
fn task_field(key: &str, value: &str) -> bool {
    match key {
        "source_ref" => hps_domain::valid_task_source_ref(value),
        "expected_epoch" => hps_domain::valid_connection_epoch(value),
        "before" => value.parse::<i64>().is_ok_and(|n| n > 0),
        "kind" => hps_domain::TASK_KINDS.contains(&value),
        "state" => hps_domain::TASK_STATES.contains(&value),
        "resource_kind" => token(value, 32),
        "resource_id" => token(value, 128),
        "request_id" => token(value, 64),
        "created_after_unix_ms" => value.parse::<u64>().is_ok_and(|n| n <= i64::MAX as u64),
        "limit" => value.parse::<u32>().is_ok_and(|n| (1..=200).contains(&n)),
        _ => false,
    }
}
fn valid_query(query: &EventQuery) -> bool {
    let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(query) else {
        return false;
    };
    fields
        .into_iter()
        .filter(|(_, v)| !v.is_null())
        .all(|(k, v)| {
            event_field(
                &k,
                &match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                },
            )
        })
}
pub(super) fn supported_get(path: &str) -> bool {
    let (base, query) = path
        .split_once('?')
        .map_or((path, None), |(b, q)| (b, Some(q)));
    let detail = base.strip_prefix("api/v1/tasks/");
    let task = base == "api/v1/tasks";
    let event = base == "api/v1/events";
    if !task && !event && detail.is_none() {
        return false;
    }
    if detail.is_some_and(|s| !s.parse::<i64>().is_ok_and(|n| n > 0 && n.to_string() == s)) {
        return false;
    }
    query.is_none_or(|q| {
        let mut seen = std::collections::HashSet::new();
        !q.is_empty()
            && url::form_urlencoded::parse(q.as_bytes()).all(|(k, v)| {
                seen.insert(k.to_string())
                    && if detail.is_some() {
                        k == "expected_epoch" && hps_domain::valid_connection_epoch(&v)
                    } else if task {
                        task_field(&k, &v)
                    } else {
                        event_field(&k, &v)
                    }
            })
    })
}

#[cfg(test)]
mod tests;
