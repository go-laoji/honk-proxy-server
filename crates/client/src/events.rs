use super::*;
use hps_domain::{
    EventFrame, EventQuery, EventsOverview, TaskInfo, TaskQuery, TaskSubmission, TasksOverview,
    parse_event_cursor, valid_connection_epoch,
};

const MAX_FRAME: usize = 512 * 1024;
const MAX_BUFFER: usize = 2 * 1024 * 1024;
fn query_url<T: serde::Serialize>(base: &Url, path: &str, query: &T) -> Result<Url, ClientError> {
    let mut url = base.join(path).map_err(|_| ClientError::InvalidEndpoint)?;
    if let serde_json::Value::Object(fields) =
        serde_json::to_value(query).map_err(|_| ClientError::EventProtocol)?
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in fields {
            if !value.is_null() {
                pairs.append_pair(
                    &key,
                    &match value {
                        serde_json::Value::String(v) => v,
                        other => other.to_string(),
                    },
                );
            }
        }
    }
    Ok(url)
}
impl HpsClient {
    pub async fn tasks(&self, query: &TaskQuery) -> Result<TasksOverview, ClientError> {
        if query
            .expected_epoch
            .as_ref()
            .is_some_and(|e| !valid_connection_epoch(e))
        {
            return Err(ClientError::InvalidTaskId);
        }
        Self::decode(
            self.http
                .get(query_url(&self.endpoint, "api/v1/tasks", query)?)
                .send()
                .await?,
        )
        .await
    }
    pub async fn task(&self, id: i64, epoch: Option<&str>) -> Result<TaskInfo, ClientError> {
        if id <= 0 || epoch.is_some_and(|e| !valid_connection_epoch(e)) {
            return Err(ClientError::InvalidTaskId);
        }
        let mut url = self
            .endpoint
            .join(&format!("api/v1/tasks/{id}"))
            .map_err(|_| ClientError::InvalidEndpoint)?;
        if let Some(e) = epoch {
            url.query_pairs_mut().append_pair("expected_epoch", e);
        }
        Self::decode(self.http.get(url).send().await?).await
    }
    pub async fn submit_task(&self, input: &TaskSubmission) -> Result<TaskInfo, ClientError> {
        self.post_body("api/v1/tasks", input).await
    }
    pub async fn events(&self, query: &EventQuery) -> Result<EventsOverview, ClientError> {
        if query
            .cursor
            .as_ref()
            .is_some_and(|s| parse_event_cursor(s).is_none())
        {
            return Err(ClientError::InvalidEventCursor);
        }
        Self::decode(
            self.http
                .get(query_url(&self.endpoint, "api/v1/events", query)?)
                .send()
                .await?,
        )
        .await
    }
    pub async fn subscribe_events(
        &self,
        query: &EventQuery,
    ) -> Result<EventSubscription, ClientError> {
        if query
            .cursor
            .as_ref()
            .is_some_and(|s| parse_event_cursor(s).is_none())
        {
            return Err(ClientError::InvalidEventCursor);
        }
        let response = self
            .stream_http
            .get(query_url(&self.endpoint, "api/v1/events/stream", query)?)
            .send()
            .await?;
        if !response.status().is_success() {
            return Self::decode::<serde_json::Value>(response)
                .await
                .and(Err(ClientError::EventProtocol));
        }
        if !response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.split(';').next() == Some("text/event-stream"))
        {
            return Err(ClientError::EventProtocol);
        }
        Ok(EventSubscription {
            response: Some(response),
            decoder: Decoder::default(),
            epoch: None,
            cursor: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page() -> EventsOverview {
        let epoch = "11111111-1111-4111-8111-111111111111".to_owned();
        EventsOverview {
            cursor: hps_domain::event_cursor(&epoch, 1),
            epoch,
            observed_at_unix_ms: 1,
            records: vec![],
            oldest_available_sequence: 1,
            head_sequence: 1,
            has_more: false,
            reset_reason: None,
            retention_limit: 10000,
        }
    }
    fn feed(subscription: &mut EventSubscription, kind: &str, page: &EventsOverview) {
        subscription
            .decoder
            .push(
                format!(
                    "event: {kind}\nid: {}\ndata: {}\n\n",
                    page.cursor,
                    serde_json::to_string(page).unwrap()
                )
                .as_bytes(),
            )
            .unwrap();
    }
    fn ready(subscription: &mut EventSubscription) -> Result<Option<EventFrame>, ClientError> {
        use std::{
            future::Future,
            task::{Context, Poll, Waker},
        };
        let mut future = Box::pin(subscription.next());
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("buffered frame must not wait on IO"),
        }
    }
    #[test]
    fn invalid_frames_never_advance_the_last_complete_cursor_or_mix_instances() {
        for mode in ["unknown", "regression", "instance", "reset", "more"] {
            let mut stream = EventSubscription {
                response: None,
                decoder: Decoder::default(),
                epoch: None,
                cursor: None,
            };
            let valid = page();
            feed(&mut stream, "snapshot", &valid);
            assert!(matches!(
                ready(&mut stream).unwrap(),
                Some(EventFrame::Snapshot(_))
            ));
            let mut invalid = valid.clone();
            let mut kind = "events";
            match mode {
                "unknown" => {
                    kind = "unknown";
                    invalid.cursor = hps_domain::event_cursor(&invalid.epoch, 2);
                    invalid.head_sequence = 2;
                }
                "regression" => invalid.cursor = hps_domain::event_cursor(&invalid.epoch, 0),
                "instance" => {
                    invalid.epoch = "22222222-2222-4222-8222-222222222222".into();
                    invalid.cursor = hps_domain::event_cursor(&invalid.epoch, 1);
                }
                "reset" => kind = "reset",
                "more" => invalid.has_more = true,
                _ => unreachable!(),
            }
            feed(&mut stream, kind, &invalid);
            assert!(ready(&mut stream).is_err());
            assert_eq!(stream.cursor(), Some(valid.cursor.as_str()));
            assert!(stream.decoder.buffer.is_empty());
        }
    }
    #[test]
    fn decoder_keeps_fragmented_utf8_crlf_comments_and_multiple_frames() {
        let bytes=": keepalive\r\n\r\nevent: events\r\nid: cursor\r\ndata: {\r\ndata: \"value\":\"中文\"\r\ndata: }\r\n\r\n".as_bytes();
        let mut decoder = Decoder::default();
        let mut frames = vec![];
        for byte in bytes {
            decoder.push(&[*byte]).unwrap();
            while let Some(frame) = decoder.next().unwrap() {
                frames.push(frame);
            }
        }
        assert_eq!(frames.len(), 2);
        assert!(frames[0].data.is_empty());
        assert_eq!(frames[1].kind, "events");
        assert_eq!(frames[1].id.as_deref(), Some("cursor"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&frames[1].data).unwrap()["value"],
            "中文"
        );
        assert!(decoder.buffer.is_empty());
    }
    #[test]
    fn decoder_rejects_oversized_unterminated_frames_bad_utf8_and_nul_ids() {
        let mut decoder = Decoder::default();
        decoder.push(&vec![b'x'; MAX_FRAME + 1]).unwrap();
        assert!(decoder.next().is_err());
        let mut decoder = Decoder::default();
        assert!(decoder.push(&vec![b'x'; MAX_BUFFER + 1]).is_err());
        assert!(decoder.buffer.is_empty());
        let mut decoder = Decoder::default();
        decoder.push(b"data: \xff\n\n").unwrap();
        assert!(decoder.next().is_err());
        let mut decoder = Decoder::default();
        decoder.push(b"id: bad\0value\ndata: {}\n\n").unwrap();
        assert!(decoder.next().is_err());
    }
}

struct RawFrame {
    kind: String,
    id: Option<String>,
    data: String,
}
#[derive(Default)]
struct Decoder {
    buffer: Vec<u8>,
}
impl Decoder {
    fn push(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        if self.buffer.len().saturating_add(bytes.len()) > MAX_BUFFER {
            return Err(ClientError::EventProtocol);
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }
    fn next(&mut self) -> Result<Option<RawFrame>, ClientError> {
        let end = self
            .buffer
            .windows(2)
            .position(|w| w == b"\n\n")
            .map(|n| (n, 2))
            .into_iter()
            .chain(
                self.buffer
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|n| (n, 4)),
            )
            .min_by_key(|(n, _)| *n);
        let Some((end, separator)) = end else {
            if self.buffer.len() > MAX_FRAME {
                return Err(ClientError::EventProtocol);
            }
            return Ok(None);
        };
        if end > MAX_FRAME {
            return Err(ClientError::EventProtocol);
        }
        let text =
            std::str::from_utf8(&self.buffer[..end]).map_err(|_| ClientError::EventProtocol)?;
        let mut kind = "message".to_owned();
        let mut id = None;
        let mut data = String::new();
        for line in text.lines() {
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => kind = value.into(),
                "id" => {
                    if value.contains('\0') {
                        return Err(ClientError::EventProtocol);
                    }
                    id = Some(value.into());
                }
                "data" => {
                    data.push_str(value);
                    data.push('\n');
                }
                _ => {}
            }
        }
        data.pop();
        self.buffer.drain(..end + separator);
        Ok(Some(RawFrame { kind, id, data }))
    }
}

/// One bounded response, with no queued event copies or implicit reconnection/replay.
/// Drop it to cancel the HTTP subscription; callers explicitly reconnect from the last complete cursor.
pub struct EventSubscription {
    response: Option<reqwest::Response>,
    decoder: Decoder,
    epoch: Option<String>,
    cursor: Option<String>,
}
impl EventSubscription {
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }
    pub async fn next(&mut self) -> Result<Option<EventFrame>, ClientError> {
        let result = self.next_inner().await;
        if result.is_err() {
            self.response = None;
            self.decoder.buffer.clear();
        }
        result
    }
    async fn next_inner(&mut self) -> Result<Option<EventFrame>, ClientError> {
        loop {
            while let Some(raw) = self.decoder.next()? {
                if raw.data.is_empty() {
                    continue;
                }
                if raw.kind == "unavailable" {
                    let value: serde_json::Value =
                        serde_json::from_str(&raw.data).map_err(|_| ClientError::EventProtocol)?;
                    if value.get("code").and_then(serde_json::Value::as_str)
                        != Some("event_stream_unavailable")
                    {
                        return Err(ClientError::EventProtocol);
                    }
                    self.response = None;
                    self.decoder.buffer.clear();
                    return Ok(Some(EventFrame::Unavailable {
                        code: "event_stream_unavailable".into(),
                    }));
                }
                let page: EventsOverview =
                    serde_json::from_str(&raw.data).map_err(|_| ClientError::EventProtocol)?;
                let (epoch, sequence) =
                    parse_event_cursor(&page.cursor).ok_or(ClientError::EventProtocol)?;
                if epoch != page.epoch
                    || raw.id.as_deref() != Some(&page.cursor)
                    || sequence > page.head_sequence
                    || page.records.len() > 200
                    || page.oldest_available_sequence < 1
                    || sequence < 0
                    || self.epoch.as_ref().is_some_and(|e| e != epoch)
                    || self
                        .cursor
                        .as_ref()
                        .and_then(|s| parse_event_cursor(s))
                        .is_some_and(|(_, n)| n > sequence)
                    || page
                        .records
                        .windows(2)
                        .any(|r| r[0].sequence >= r[1].sequence)
                    || page.records.iter().any(|r| {
                        r.sequence < page.oldest_available_sequence
                            || r.sequence > sequence
                            || !valid_connection_epoch(&r.recorded_epoch)
                    })
                    || (raw.kind == "checkpoint" && !page.records.is_empty())
                    || ((raw.kind == "reset") != page.reset_reason.is_some())
                    || (page.has_more && sequence >= page.head_sequence)
                {
                    return Err(ClientError::EventProtocol);
                }
                if let Some((_, previous)) =
                    self.cursor.as_ref().and_then(|s| parse_event_cursor(s))
                    && (raw.kind == "snapshot"
                        || (raw.kind != "reset"
                            && page.records.iter().any(|r| r.sequence <= previous)))
                {
                    return Err(ClientError::EventProtocol);
                }
                let epoch = page.epoch.clone();
                let cursor = page.cursor.clone();
                let frame = match raw.kind.as_str() {
                    "snapshot" => EventFrame::Snapshot(page),
                    "events" => EventFrame::Events(page),
                    "checkpoint" => EventFrame::Checkpoint(page),
                    "reset" => EventFrame::Reset(page),
                    _ => return Err(ClientError::EventProtocol),
                };
                self.epoch = Some(epoch);
                self.cursor = Some(cursor);
                return Ok(Some(frame));
            }
            let Some(response) = self.response.as_mut() else {
                return Ok(None);
            };
            match response.chunk().await? {
                Some(bytes) => self.decoder.push(&bytes)?,
                None => {
                    self.response = None;
                    if !self.decoder.buffer.is_empty() {
                        return Err(ClientError::EventProtocol);
                    }
                    return Ok(None);
                }
            }
        }
    }
}
