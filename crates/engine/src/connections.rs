//! Bounded business-flow observations owned by the same registry as node admission.
use crate::{
    BoxIo,
    nodes::{NodeGuard, NodeRegistry},
};
use hps_domain::{
    ConnectionChoice, ConnectionInfo, ConnectionStatistics, RouteDecision, RoutingContext,
};
use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

pub const MAX_ACTIVE_RECORDS: usize = 10000;
pub const MAX_RECENT_RECORDS: usize = 512;
const RECENT_TTL: Duration = Duration::from_secs(300);
fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}
fn add(counter: &AtomicU64, n: usize) {
    counter.fetch_add(n as u64, Ordering::Relaxed);
}

#[derive(Default)]
struct Totals {
    bytes_up: AtomicU64,
    bytes_down: AtomicU64,
}
#[derive(Default)]
struct State {
    next: u64,
    active: usize,
    untracked: usize,
    untracked_total: u64,
    opened: u64,
    closed: u64,
    failed: u64,
    rejected: u64,
    disconnected: u64,
    errors: BTreeMap<String, u64>,
    records: BTreeMap<u64, Arc<FlowRecord>>,
    recent: VecDeque<(Instant, ConnectionInfo)>,
}
impl State {
    fn prune(&mut self) {
        while self
            .recent
            .front()
            .is_some_and(|(when, _)| when.elapsed() > RECENT_TTL)
            || self.recent.len() > MAX_RECENT_RECORDS
        {
            self.recent.pop_front();
        }
    }
}
pub(crate) struct ObservationRuntime {
    epoch: String,
    started_at: u64,
    origin: Instant,
    totals: Arc<Totals>,
    state: Mutex<State>,
}
impl Default for ObservationRuntime {
    fn default() -> Self {
        Self {
            epoch: uuid::Uuid::new_v4().to_string(),
            started_at: wall_ms(),
            origin: Instant::now(),
            totals: Arc::new(Totals::default()),
            state: Mutex::new(State::default()),
        }
    }
}
#[derive(Default, Clone)]
struct Outcome {
    opened: bool,
    phase: &'static str,
    terminal: Option<&'static str>,
    error: Option<&'static str>,
    http_status: Option<u16>,
    disconnect_requested: bool,
    ended_at: Option<u64>,
    ended_elapsed: Option<u64>,
    close_reason: Option<&'static str>,
}
struct FlowRecord {
    sequence: u64,
    info: Mutex<Option<ConnectionInfo>>,
    outcome: Mutex<Outcome>,
    created: Instant,
    up: AtomicU64,
    down: AtomicU64,
    cancel: CancellationToken,
    inbound_cancel: Mutex<Option<CancellationToken>>,
}
impl FlowRecord {
    fn snapshot(&self) -> Option<ConnectionInfo> {
        let mut info = self.info.lock().unwrap().clone()?;
        let outcome = self.outcome.lock().unwrap().clone();
        info.elapsed_ms = outcome
            .ended_elapsed
            .unwrap_or_else(|| millis(self.created.elapsed()));
        info.ended_at_unix_ms = outcome.ended_at;
        info.bytes_up = self.up.load(Ordering::Relaxed);
        info.bytes_down = self.down.load(Ordering::Relaxed);
        info.state = outcome
            .terminal
            .unwrap_or(if outcome.disconnect_requested {
                "disconnecting"
            } else if outcome.opened {
                "open"
            } else {
                "dialing"
            })
            .into();
        info.phase = outcome.phase.into();
        info.http_status = outcome.http_status;
        info.error_code = outcome.error.map(str::to_owned);
        info.close_reason = outcome.close_reason.map(str::to_owned);
        info.disconnect_requested = outcome.disconnect_requested;
        Some(info)
    }
    fn request_disconnect(&self) {
        self.outcome.lock().unwrap().disconnect_requested = true;
        self.cancel.cancel();
        if let Some(token) = self.inbound_cancel.lock().unwrap().as_ref() {
            token.cancel();
        }
    }
}
struct Owner {
    registry: Arc<NodeRegistry>,
    record: Arc<FlowRecord>,
    node: Mutex<Option<NodeGuard>>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        let mut outcome = self.record.outcome.lock().unwrap();
        outcome.ended_at = Some(wall_ms());
        outcome.ended_elapsed = Some(millis(self.record.created.elapsed()));
        let inbound_cancelled = self
            .record
            .inbound_cancel
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled);
        if outcome.disconnect_requested {
            outcome.terminal = Some("disconnected");
            outcome.close_reason = Some("management_disconnect");
        } else if outcome.terminal.is_none() {
            outcome.terminal = Some("closed");
            outcome.close_reason = Some(if inbound_cancelled {
                "ingress_cancelled"
            } else {
                "owner_released"
            });
        }
        let final_state = outcome.terminal;
        let error = outcome.error;
        drop(outcome);
        let info = self.record.snapshot();
        let mut state = self.registry.observations().state.lock().unwrap();
        state.active -= 1;
        if state.records.remove(&self.record.sequence).is_none() {
            state.untracked -= 1;
        }
        match final_state {
            Some("failed") => state.failed += 1,
            Some("rejected") => state.rejected += 1,
            Some("disconnected") => state.disconnected += 1,
            _ => state.closed += 1,
        }
        if let Some(error) = error {
            let key = if state.errors.len() < 32 || state.errors.contains_key(error) {
                error
            } else {
                "other"
            };
            *state.errors.entry(key.into()).or_default() += 1;
        }
        if let Some(info) = info {
            state.recent.push_back((Instant::now(), info));
        }
        state.prune();
    }
}
/// Clones share one lifetime. HTTP response bodies can retain it after the socket finishes.
#[derive(Clone)]
pub struct ConnectionGuard(Arc<Owner>);
impl ConnectionGuard {
    pub fn cancellation(&self) -> CancellationToken {
        self.0.record.cancel.clone()
    }
    pub fn bind_inbound_cancel(&self, token: CancellationToken) {
        let mut bound = self.0.record.inbound_cancel.lock().unwrap();
        if self.0.record.cancel.is_cancelled() {
            token.cancel();
        }
        *bound = Some(token);
    }
    pub(crate) fn select(&self, choice: ConnectionChoice) -> bool {
        let mut node = self.0.node.lock().unwrap();
        if node.is_some() {
            return false;
        }
        let Some(guard) = self.0.registry.begin(&choice.node_id) else {
            return false;
        };
        *node = Some(guard);
        if let Some(info) = self.0.record.info.lock().unwrap().as_mut() {
            info.choice = Some(choice);
        }
        true
    }
    pub(crate) fn open(&self) {
        let first = {
            let mut outcome = self.0.record.outcome.lock().unwrap();
            let first = !outcome.opened;
            outcome.opened = true;
            outcome.phase = "transferring";
            first
        };
        if first {
            self.0.registry.observations().state.lock().unwrap().opened += 1;
        }
    }
    pub(crate) fn selection(&self, decision: &hps_domain::SelectionDecision) {
        if let Some(info) = self.0.record.info.lock().unwrap().as_mut() {
            info.selection = Some(hps_domain::ConnectionSelection {
                group: decision.group.clone(),
                strategy: decision.strategy.clone(),
                reason: decision.reason.clone(),
                selected: decision.selected.clone(),
                session_reused: decision.session_reused,
            });
        }
    }
    pub(crate) fn fail_unless_recorded(
        &self,
        phase: &'static str,
        code: &'static str,
        rejected: bool,
    ) {
        let mut outcome = self.0.record.outcome.lock().unwrap();
        if outcome.terminal.is_none() {
            outcome.phase = phase;
            outcome.error = Some(code);
            outcome.terminal = Some(if rejected { "rejected" } else { "failed" });
        }
    }
    pub(crate) fn fail(&self, phase: &'static str, code: &'static str, rejected: bool) {
        let mut outcome = self.0.record.outcome.lock().unwrap();
        outcome.phase = phase;
        outcome.error = Some(code);
        outcome.terminal = Some(if rejected { "rejected" } else { "failed" });
    }
    pub(crate) fn http_status(&self, status: u16, proxy_auth: bool) {
        let mut outcome = self.0.record.outcome.lock().unwrap();
        outcome.http_status = Some(status);
        if proxy_auth {
            outcome.error = Some("upstream_authentication_failed");
            outcome.terminal = Some("failed");
            outcome.phase = "authentication";
        } else if matches!(status, 403 | 429) {
            outcome.error = Some("target_rejected");
        }
    }
    pub fn wrap(&self, inner: BoxIo) -> BoxIo {
        Box::new(ObservedIo {
            inner,
            guard: self.clone(),
            read_cancel: Box::pin(self.cancellation().cancelled_owned()),
            write_cancel: Box::pin(self.cancellation().cancelled_owned()),
        })
    }
}
impl NodeRegistry {
    pub fn observation_epoch(&self) -> &str {
        &self.observations().epoch
    }
    pub(crate) fn observe(
        self: &Arc<Self>,
        mut context: RoutingContext,
        routing: RouteDecision,
    ) -> Option<ConnectionGuard> {
        let runtime = self.observations();
        let mut state = runtime.state.lock().unwrap();
        state.next = state.next.checked_add(1)?;
        let sequence = state.next;
        state.active += 1;
        let truncated = context.observed_domains.len() > 8;
        context.observed_domains.truncate(8);
        for name in &mut context.observed_domains {
            truncate(name, 253);
        }
        if let Some(process) = context.process.as_mut() {
            truncate(&mut process.command, 256);
        }
        let tracked = state.records.len() < MAX_ACTIVE_RECORDS;
        let info = tracked.then(|| ConnectionInfo {
            id: format!("{}:{sequence:020}", runtime.epoch),
            epoch: runtime.epoch.clone(),
            created_at_unix_ms: wall_ms(),
            ended_at_unix_ms: None,
            elapsed_ms: 0,
            context,
            observed_domains_truncated: truncated,
            routing,
            choice: None,
            selection: None,
            state: "dialing".into(),
            phase: "routing".into(),
            bytes_up: 0,
            bytes_down: 0,
            http_status: None,
            error_code: None,
            close_reason: None,
            disconnect_requested: false,
        });
        let record = Arc::new(FlowRecord {
            sequence,
            info: Mutex::new(info),
            outcome: Mutex::new(Outcome {
                phase: "routing",
                ..Default::default()
            }),
            created: Instant::now(),
            up: AtomicU64::new(0),
            down: AtomicU64::new(0),
            cancel: CancellationToken::new(),
            inbound_cancel: Mutex::new(None),
        });
        if tracked {
            state.records.insert(sequence, record.clone());
        } else {
            state.untracked += 1;
            state.untracked_total += 1;
        }
        Some(ConnectionGuard(Arc::new(Owner {
            registry: self.clone(),
            record,
            node: Mutex::new(None),
        })))
    }
    pub fn connection_records(
        &self,
        include_recent: bool,
    ) -> (ConnectionStatistics, Vec<ConnectionInfo>) {
        let runtime = self.observations();
        let mut state = runtime.state.lock().unwrap();
        state.prune();
        let mut records = state
            .records
            .values()
            .filter_map(|r| r.snapshot())
            .collect::<Vec<_>>();
        if include_recent {
            records.extend(state.recent.iter().map(|(_, r)| r.clone()));
        }
        let totals = &runtime.totals;
        let stats = ConnectionStatistics {
            epoch: runtime.epoch.clone(),
            started_at_unix_ms: runtime.started_at,
            elapsed_ms: millis(runtime.origin.elapsed()),
            active: state.active,
            active_tracked: state.records.len(),
            active_untracked: state.untracked,
            active_record_limit: MAX_ACTIVE_RECORDS,
            recent_record_limit: MAX_RECENT_RECORDS,
            started: state.next,
            opened: state.opened,
            closed: state.closed,
            failed: state.failed,
            rejected: state.rejected,
            disconnected: state.disconnected,
            untracked_total: state.untracked_total,
            bytes_up: totals.bytes_up.load(Ordering::Relaxed),
            bytes_down: totals.bytes_down.load(Ordering::Relaxed),
            errors: state.errors.clone(),
        };
        (stats, records)
    }
    /// IDs are qualified by the process epoch. A stale instance can never target a new flow.
    pub fn disconnect_connection(&self, id: &str) -> bool {
        let runtime = self.observations();
        let Some(sequence) = id
            .strip_prefix(&format!("{}:", runtime.epoch))
            .and_then(|n| n.parse::<u64>().ok())
        else {
            return false;
        };
        if id != format!("{}:{sequence:020}", runtime.epoch) {
            return false;
        }
        let record = runtime
            .state
            .lock()
            .unwrap()
            .records
            .get(&sequence)
            .cloned();
        if let Some(record) = record {
            record.request_disconnect();
            true
        } else {
            false
        }
    }
}
fn truncate(text: &mut String, maximum: usize) {
    if text.len() > maximum {
        let mut end = maximum;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}
struct ObservedIo {
    inner: BoxIo,
    guard: ConnectionGuard,
    read_cancel: Pin<Box<WaitForCancellationFutureOwned>>,
    write_cancel: Pin<Box<WaitForCancellationFutureOwned>>,
}
fn cancelled(future: &mut Pin<Box<WaitForCancellationFutureOwned>>, cx: &mut Context<'_>) -> bool {
    future.as_mut().poll(cx).is_ready()
}
fn disconnected() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "business connection disconnected",
    )
}
impl AsyncRead for ObservedIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if cancelled(&mut self.read_cancel, cx) {
            return Poll::Ready(Err(disconnected()));
        }
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        let bytes = buf.filled().len() - before;
        if bytes > 0 {
            add(&self.guard.0.record.down, bytes);
            add(
                &self.guard.0.registry.observations().totals.bytes_down,
                bytes,
            );
        }
        if matches!(&result, Poll::Ready(Err(_))) {
            self.guard.fail("transferring", "stream_read_failed", false);
        }
        result
    }
}
impl AsyncWrite for ObservedIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if cancelled(&mut self.write_cancel, cx) {
            return Poll::Ready(Err(disconnected()));
        }
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        self.written(&result);
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if cancelled(&mut self.write_cancel, cx) {
            return Poll::Ready(Err(disconnected()));
        }
        let result = Pin::new(&mut self.inner).poll_flush(cx);
        if matches!(&result, Poll::Ready(Err(_))) {
            self.guard
                .fail("transferring", "stream_flush_failed", false);
        }
        result
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if cancelled(&mut self.write_cancel, cx) {
            return Poll::Ready(Err(disconnected()));
        }
        let result = Pin::new(&mut self.inner).poll_shutdown(cx);
        if matches!(&result, Poll::Ready(Err(_))) {
            self.guard
                .fail("transferring", "stream_shutdown_failed", false);
        }
        result
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        if cancelled(&mut self.write_cancel, cx) {
            return Poll::Ready(Err(disconnected()));
        }
        let result = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
        self.written(&result);
        result
    }
}
impl ObservedIo {
    fn written(&self, result: &Poll<io::Result<usize>>) {
        match result {
            Poll::Ready(Ok(bytes)) => {
                add(&self.guard.0.record.up, *bytes);
                add(
                    &self.guard.0.registry.observations().totals.bytes_up,
                    *bytes,
                );
            }
            Poll::Ready(Err(_)) => self
                .guard
                .fail("transferring", "stream_write_failed", false),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hps_domain::{FlowOrigin, RouteAction, SelectionTransport};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn flow(registry: &Arc<NodeRegistry>) -> ConnectionGuard {
        registry
            .observe(
                RoutingContext {
                    destination_host: "example.test".into(),
                    destination_port: 443,
                    transport: SelectionTransport::Tcp,
                    source_ip: Some("192.0.2.1".parse().unwrap()),
                    source_port: Some(30000),
                    inbound: Some("test".into()),
                    inbound_protocol: Some("http".into()),
                    observed_domains: vec![],
                    origin: FlowOrigin::Explicit,
                    process: None,
                },
                RouteDecision {
                    configuration_revision: 7,
                    evaluated_at_unix: 1,
                    rule_id: Some("direct".into()),
                    action: RouteAction::Direct,
                    domain: Some("example.test".into()),
                    domain_source: "explicit_target".into(),
                    trace: vec![],
                },
            )
            .unwrap()
    }
    struct PartialIo {
        input: VecDeque<u8>,
        written: Arc<Mutex<Vec<u8>>>,
    }
    impl AsyncRead for PartialIo {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            for _ in 0..2.min(buf.remaining()).min(self.input.len()) {
                buf.put_slice(&[self.input.pop_front().unwrap()]);
            }
            Poll::Ready(Ok(()))
        }
    }
    impl AsyncWrite for PartialIo {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            let count = bytes.len().min(3);
            self.written
                .lock()
                .unwrap()
                .extend_from_slice(&bytes[..count]);
            Poll::Ready(Ok(count))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn is_write_vectored(&self) -> bool {
            true
        }
        fn poll_write_vectored(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buffers: &[io::IoSlice<'_>],
        ) -> Poll<io::Result<usize>> {
            let mut written = self.written.lock().unwrap();
            let mut remaining = 3;
            for buffer in buffers {
                let count = buffer.len().min(remaining);
                written.extend_from_slice(&buffer[..count]);
                remaining -= count;
                if remaining == 0 {
                    break;
                }
            }
            Poll::Ready(Ok(3 - remaining))
        }
    }
    #[tokio::test]
    async fn actual_partial_read_write_and_vectored_results_count_once_and_lifetime_is_shared() {
        let registry = Arc::new(NodeRegistry::default());
        let guard = flow(&registry);
        guard.open();
        let written = Arc::new(Mutex::new(vec![]));
        let mut io = guard.wrap(Box::new(PartialIo {
            input: b"reply".to_vec().into(),
            written: written.clone(),
        }));
        io.write_all(b"request").await.unwrap();
        assert_eq!(
            io.write_vectored(&[io::IoSlice::new(b"xx"), io::IoSlice::new(b"yy")])
                .await
                .unwrap(),
            3
        );
        let mut reply = vec![];
        io.read_to_end(&mut reply).await.unwrap();
        assert_eq!(reply, b"reply");
        let (stats, records) = registry.connection_records(false);
        assert_eq!(stats.bytes_up, 10);
        assert_eq!(stats.bytes_down, 5);
        assert_eq!(records[0].bytes_up, 10);
        assert_eq!(written.lock().unwrap().len(), 10);
        drop(guard);
        assert_eq!(registry.connection_records(false).0.active, 1);
        drop(io);
        let (stats, recent) = registry.connection_records(true);
        assert_eq!(stats.active, 0);
        assert_eq!(stats.closed, 1);
        assert_eq!(recent[0].state, "closed");
        assert!(recent[0].ended_at_unix_ms.is_some());
    }
    #[tokio::test]
    async fn disconnect_wakes_idle_read_and_old_epoch_never_matches_a_new_flow() {
        let registry = Arc::new(NodeRegistry::default());
        let guard = flow(&registry);
        guard.open();
        let id = registry.connection_records(false).1[0].id.clone();
        let other = Arc::new(NodeRegistry::default());
        let _other_guard = flow(&other);
        assert!(!other.disconnect_connection(&id));
        let (client, _server) = tokio::io::duplex(8);
        let mut io = guard.wrap(Box::new(client));
        let reader = tokio::spawn(async move {
            let mut byte = [0];
            io.read(&mut byte).await
        });
        tokio::task::yield_now().await;
        assert!(registry.disconnect_connection(&id));
        let result = tokio::time::timeout(Duration::from_secs(1), reader)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionAborted);
        drop(guard);
        let (stats, recent) = registry.connection_records(true);
        assert_eq!(stats.active, 0);
        assert_eq!(stats.disconnected, 1);
        assert_eq!(recent[0].state, "disconnected");
        assert_eq!(
            recent[0].close_reason.as_deref(),
            Some("management_disconnect")
        );
    }
    #[test]
    fn active_overflow_remains_explicit_and_recent_records_are_bounded() {
        let registry = Arc::new(NodeRegistry::default());
        let guards = (0..MAX_ACTIVE_RECORDS + 1)
            .map(|_| flow(&registry))
            .collect::<Vec<_>>();
        let (stats, records) = registry.connection_records(false);
        assert_eq!(records.len(), MAX_ACTIVE_RECORDS);
        assert_eq!(stats.active, MAX_ACTIVE_RECORDS + 1);
        assert_eq!(stats.active_untracked, 1);
        assert_eq!(stats.untracked_total, 1);
        drop(guards);
        let (stats, recent) = registry.connection_records(true);
        assert_eq!(stats.active, 0);
        assert_eq!(recent.len(), MAX_RECENT_RECORDS);
        assert_eq!(stats.closed, MAX_ACTIVE_RECORDS as u64 + 1);
    }
}
