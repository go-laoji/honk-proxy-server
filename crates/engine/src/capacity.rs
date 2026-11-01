use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Shared across configuration generations so draining connections still count
/// against admission after a limit is reduced.
#[derive(Default)]
pub(crate) struct Capacity(AtomicUsize);
impl Capacity {
    pub fn active(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
    pub fn acquire(self: &Arc<Self>, limit: usize) -> Option<Permit> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < limit).then_some(active + 1)
            })
            .ok()?;
        Some(Permit(self.clone()))
    }
}
pub(crate) struct Permit(Arc<Capacity>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.0.fetch_sub(1, Ordering::AcqRel);
    }
}
