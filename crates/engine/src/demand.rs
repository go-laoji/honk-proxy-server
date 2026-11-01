use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
pub(crate) struct Demand {
    origin: Instant,
    last_ms: AtomicU64,
}
impl Default for Demand {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            last_ms: AtomicU64::new(0),
        }
    }
}
impl Demand {
    pub fn mark(&self) {
        let at = (self
            .origin
            .elapsed()
            .as_millis()
            .min((u64::MAX - 1) as u128) as u64)
            + 1;
        self.last_ms.fetch_max(at, Ordering::Relaxed);
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
