use crate::BoxIo;
use std::{
    collections::HashMap,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
struct Counter {
    origin: Instant,
    last_ms: AtomicU64,
    active: AtomicUsize,
}
impl Default for Counter {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            last_ms: AtomicU64::new(0),
            active: AtomicUsize::new(0),
        }
    }
}
impl Counter {
    fn mark(&self) {
        let at = (self
            .origin
            .elapsed()
            .as_millis()
            .min((u64::MAX - 1) as u128) as u64)
            + 1;
        self.last_ms.fetch_max(at, Ordering::Relaxed);
        self.active.fetch_add(1, Ordering::Relaxed);
    }
    pub fn active(&self) -> bool {
        self.active.load(Ordering::Relaxed) > 0
    }
    pub fn age(&self) -> Option<Duration> {
        let at = self.last_ms.load(Ordering::Relaxed);
        (at > 0).then(|| {
            Duration::from_millis(
                (self.origin.elapsed().as_millis().min(u64::MAX as u128) as u64)
                    .saturating_sub(at - 1),
            )
        })
    }
}
#[derive(Default)]
pub(crate) struct Demand {
    global: Arc<Counter>,
    sources: Mutex<HashMap<(String, String), Arc<Counter>>>,
}
impl Demand {
    pub fn active(&self) -> bool {
        self.global.active()
    }
    pub fn age(&self) -> Option<Duration> {
        self.global.age()
    }
    pub fn source(&self, id: &str, fingerprint: &str) -> (bool, Option<Duration>) {
        self.sources
            .lock()
            .unwrap()
            .get(&(id.into(), fingerprint.into()))
            .map_or((false, None), |c| (c.active(), c.age()))
    }
    pub fn begin(&self, sources: Vec<(String, String)>) -> DemandGuard {
        self.global.mark();
        let mut counters = self.sources.lock().unwrap();
        if counters.len() >= 4096 {
            counters.retain(|_, c| {
                c.active() || c.age().is_some_and(|age| age < Duration::from_secs(60))
            });
        }
        let mut selected = vec![];
        for key in sources.into_iter().take(64) {
            if counters.len() >= 4096 && !counters.contains_key(&key) {
                continue;
            }
            let counter = counters.entry(key).or_default().clone();
            counter.mark();
            selected.push(counter);
        }
        DemandGuard {
            global: self.global.clone(),
            sources: selected,
        }
    }
}
pub(crate) struct DemandGuard {
    global: Arc<Counter>,
    sources: Vec<Arc<Counter>>,
}
impl Drop for DemandGuard {
    fn drop(&mut self) {
        self.global.active.fetch_sub(1, Ordering::Relaxed);
        for source in &self.sources {
            source.active.fetch_sub(1, Ordering::Relaxed);
        }
    }
}
impl DemandGuard {
    pub fn wrap(self, io: BoxIo) -> BoxIo {
        Box::new(DemandIo {
            inner: io,
            _guard: self,
        })
    }
}
struct DemandIo {
    inner: BoxIo,
    _guard: DemandGuard,
}
impl AsyncRead for DemandIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
impl AsyncWrite for DemandIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_scope_and_configuration_fingerprint_do_not_share_purchase_signals() {
        let demand = Demand::default();
        let first = demand.begin(vec![("selected".into(), "v1".into())]);
        assert!(demand.active());
        assert!(demand.source("selected", "v1").0);
        assert_eq!(demand.source("other", "v1"), (false, None));
        assert_eq!(demand.source("selected", "v2"), (false, None));
        let business_only = demand.begin(vec![]);
        drop(first);
        assert!(!demand.source("selected", "v1").0);
        assert!(demand.source("selected", "v1").1.is_some());
        assert!(demand.active());
        drop(business_only);
        assert!(!demand.active());
    }
}
