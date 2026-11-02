use crate::BoxIo;
use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
pub(crate) struct Demand {
    origin: Instant,
    last_ms: AtomicU64,
    active: AtomicUsize,
}
impl Default for Demand {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            last_ms: AtomicU64::new(0),
            active: AtomicUsize::new(0),
        }
    }
}
impl Demand {
    pub fn begin(self: &Arc<Self>) -> DemandGuard {
        let at = (self
            .origin
            .elapsed()
            .as_millis()
            .min((u64::MAX - 1) as u128) as u64)
            + 1;
        self.last_ms.fetch_max(at, Ordering::Relaxed);
        self.active.fetch_add(1, Ordering::Relaxed);
        DemandGuard(self.clone())
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
pub(crate) struct DemandGuard(Arc<Demand>);
impl Drop for DemandGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
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
