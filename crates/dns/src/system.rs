//! getaddrinfo cannot be cancelled once libc starts it. A fixed, independent
//! thread pool bounds both active work and the queue after callers time out.
use crate::{DnsError, Result};
use std::{
    net::{IpAddr, ToSocketAddrs},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};
use tokio::sync::oneshot;
const WORKERS: usize = 8;
const QUEUE: usize = 32;
struct Request {
    host: String,
    reply: oneshot::Sender<Result<Vec<IpAddr>>>,
}
#[derive(Default)]
struct Counters {
    active: AtomicUsize,
    queued: AtomicUsize,
}
struct Pool {
    sender: mpsc::SyncSender<Request>,
    counts: Arc<Counters>,
    limit: usize,
}
static POOL: OnceLock<Result<Pool>> = OnceLock::new();
impl Pool {
    fn new(
        workers: usize,
        queue: usize,
        resolve: impl Fn(&str) -> Result<Vec<IpAddr>> + Send + Sync + 'static,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Request>(queue);
        let receiver = Arc::new(Mutex::new(receiver));
        let counts = Arc::new(Counters::default());
        let resolve = Arc::new(resolve);
        for index in 0..workers {
            let receiver = receiver.clone();
            let counts = counts.clone();
            let resolve = resolve.clone();
            std::thread::Builder::new()
                .name(format!("hps-system-dns-{index}"))
                .spawn(move || {
                    loop {
                        let request = match receiver.lock().unwrap().recv() {
                            Ok(request) => request,
                            Err(_) => break,
                        };
                        counts.queued.fetch_sub(1, Ordering::AcqRel);
                        if request.reply.is_closed() {
                            continue;
                        }
                        counts.active.fetch_add(1, Ordering::AcqRel);
                        let result = resolve(&request.host);
                        counts.active.fetch_sub(1, Ordering::AcqRel);
                        let _ = request.reply.send(result);
                    }
                })
                .map_err(|_| DnsError::Worker)?;
        }
        Ok(Self {
            sender,
            counts,
            limit: workers,
        })
    }
    fn enqueue(&self, host: String) -> Result<oneshot::Receiver<Result<Vec<IpAddr>>>> {
        let (reply, receiver) = oneshot::channel();
        self.counts.queued.fetch_add(1, Ordering::AcqRel);
        if let Err(error) = self.sender.try_send(Request { host, reply }) {
            self.counts.queued.fetch_sub(1, Ordering::AcqRel);
            return Err(match error {
                mpsc::TrySendError::Full(_) => DnsError::Capacity,
                mpsc::TrySendError::Disconnected(_) => DnsError::Worker,
            });
        }
        Ok(receiver)
    }
}
pub(crate) async fn lookup(host: String) -> Result<Vec<IpAddr>> {
    let pool = POOL
        .get_or_init(|| {
            Pool::new(WORKERS, QUEUE, |host| {
                // Read/iteration is bounded before collecting; a system result has no DNS TTL.
                (host, 0)
                    .to_socket_addrs()
                    .map(|addresses| addresses.take(128).map(|a| a.ip()).collect())
                    .map_err(|_| DnsError::System)
            })
        })
        .as_ref()
        .map_err(|e| *e)?;
    pool.enqueue(host)?.await.map_err(|_| DnsError::Worker)?
}
pub(crate) fn stats() -> (usize, usize, usize) {
    POOL.get()
        .and_then(|pool| pool.as_ref().ok())
        .map_or((0, 0, WORKERS), |pool| {
            (
                pool.counts.active.load(Ordering::Acquire),
                pool.counts.queued.load(Ordering::Acquire),
                pool.limit,
            )
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_lookups_keep_blocking_work_bounded_and_skip_cancelled_queue_items() {
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let block = gate.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let pool = Pool::new(1, 2, move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            let (lock, wake) = &*block;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = wake.wait(open).unwrap();
            }
            Ok(vec!["127.0.0.1".parse().unwrap()])
        })
        .unwrap();
        let first = pool.enqueue("first".into()).unwrap();
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        drop(first);
        drop(pool.enqueue("queued-one".into()).unwrap());
        drop(pool.enqueue("queued-two".into()).unwrap());
        assert!(matches!(
            pool.enqueue("overflow".into()),
            Err(DnsError::Capacity)
        ));
        assert_eq!(pool.counts.active.load(Ordering::Acquire), 1);
        assert_eq!(pool.counts.queued.load(Ordering::Acquire), 2);
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        while pool.counts.queued.load(Ordering::Acquire) > 0 {
            tokio::task::yield_now().await;
        }
        let next = pool.enqueue("fresh".into()).unwrap();
        assert_eq!(next.await.unwrap().unwrap().len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(pool.counts.active.load(Ordering::Acquire), 0);
    }
}
