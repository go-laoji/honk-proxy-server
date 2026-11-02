use crate::BoxIo;
use std::{
    io,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    time::Instant,
};

pub(crate) struct Pacer {
    rate: u64,
    queue: tokio::sync::Mutex<()>,
}
impl Pacer {
    pub fn new(rate: u64) -> Arc<Self> {
        Arc::new(Self {
            rate,
            queue: tokio::sync::Mutex::new(()),
        })
    }
    async fn wait(&self, bytes: usize) {
        if self.rate == 0 || bytes == 0 {
            return;
        }
        let _turn = self.queue.lock().await;
        tokio::time::sleep(Duration::from_secs_f64(bytes as f64 / self.rate as f64)).await;
    }
}
#[derive(Clone)]
pub(crate) struct Activity {
    last: Arc<Mutex<Instant>>,
    throttling: Arc<AtomicUsize>,
}
impl Activity {
    pub fn new() -> Self {
        Self {
            last: Arc::new(Mutex::new(Instant::now())),
            throttling: Arc::new(AtomicUsize::new(0)),
        }
    }
    pub fn touch(&self) {
        *self.last.lock().unwrap() = Instant::now();
    }
    pub async fn idle(&self, duration: Duration) {
        loop {
            let deadline = *self.last.lock().unwrap() + duration;
            tokio::time::sleep_until(deadline).await;
            if self.throttling.load(Ordering::Relaxed) == 0
                && self.last.lock().unwrap().elapsed() >= duration
            {
                return;
            }
            tokio::time::sleep(duration.min(Duration::from_secs(1))).await;
        }
    }
}
struct ThrottleGuard(Activity);
impl Drop for ThrottleGuard {
    fn drop(&mut self) {
        self.0.touch();
        self.0.throttling.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub(crate) struct Budget {
    global: Arc<Pacer>,
    inbound: Arc<Pacer>,
    activity: Activity,
}
impl Budget {
    pub fn new(global: Arc<Pacer>, inbound: Arc<Pacer>, activity: Activity) -> Self {
        Self {
            global,
            inbound,
            activity,
        }
    }
    pub async fn wait(&self, bytes: usize) {
        self.activity.throttling.fetch_add(1, Ordering::Relaxed);
        let _guard = ThrottleGuard(self.activity.clone());
        tokio::join!(self.global.wait(bytes), self.inbound.wait(bytes));
    }
}

pub(crate) struct ActiveIo<T> {
    inner: T,
    activity: Activity,
}
impl<T> ActiveIo<T> {
    pub fn new(inner: T, activity: Activity) -> Self {
        Self { inner, activity }
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for ActiveIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if buf.filled().len() > before {
            self.activity.touch();
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for ActiveIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, bytes);
        if matches!(result,Poll::Ready(Ok(n)) if n>0) {
            self.activity.touch();
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub(crate) async fn relay(
    left: BoxIo,
    right: BoxIo,
    buffer_bytes: usize,
    budget: Budget,
) -> io::Result<()> {
    let (lr, lw) = tokio::io::split(left);
    let (rr, rw) = tokio::io::split(right);
    tokio::try_join!(
        pump(lr, rw, buffer_bytes, budget.clone()),
        pump(rr, lw, buffer_bytes, budget)
    )?;
    Ok(())
}
async fn pump(
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    buffer_bytes: usize,
    budget: Budget,
) -> io::Result<()> {
    let mut buffer = vec![0; buffer_bytes];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            writer.shutdown().await?;
            return Ok(());
        }
        budget.wait(count).await;
        writer.write_all(&buffer[..count]).await?;
        writer.flush().await?;
    }
}
