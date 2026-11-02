//! One operational gate and live dial/socket count, shared by all engine generations.
use hps_domain::{NodeControl, NodeOperationState};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct State {
    controls: HashMap<String, NodeControl>,
    active: HashMap<String, usize>,
}
#[derive(Default)]
pub struct NodeRegistry(Mutex<State>, crate::connections::ObservationRuntime);
impl NodeRegistry {
    pub(crate) fn observations(&self) -> &crate::connections::ObservationRuntime {
        &self.1
    }
    /// Install an authoritative storage snapshot while retaining existing socket ownership.
    /// The caller serializes this with writes; it is also used to release pruned lease controls.
    pub fn reconcile(&self, controls: Vec<NodeControl>) {
        self.0.lock().unwrap().controls = controls
            .into_iter()
            .map(|c| (c.node_id.clone(), c))
            .collect();
    }
    pub fn blocking_reason(&self, id: &str) -> Option<&'static str> {
        self.0
            .lock()
            .unwrap()
            .controls
            .get(id)
            .and_then(|c| c.state.blocking_reason())
    }
    pub fn publish(&self, control: NodeControl) {
        let mut state = self.0.lock().unwrap();
        if state
            .controls
            .get(&control.node_id)
            .is_none_or(|old| old.version <= control.version)
        {
            state.controls.insert(control.node_id.clone(), control);
        }
    }
    pub fn control(&self, id: &str) -> NodeControl {
        self.0
            .lock()
            .unwrap()
            .controls
            .get(id)
            .cloned()
            .unwrap_or_else(|| NodeControl {
                node_id: id.into(),
                state: NodeOperationState::Enabled,
                version: 0,
                updated_at: 0,
            })
    }
    pub fn active(&self, id: &str) -> usize {
        self.0.lock().unwrap().active.get(id).copied().unwrap_or(0)
    }
    pub fn total_active(&self) -> usize {
        self.0.lock().unwrap().active.values().sum()
    }
    /// Serializes admission with restriction publication. Already admitted dials drain naturally.
    pub(crate) fn begin(self: &Arc<Self>, id: &str) -> Option<NodeGuard> {
        let mut state = self.0.lock().unwrap();
        if state
            .controls
            .get(id)
            .is_some_and(|c| c.state.blocking_reason().is_some())
        {
            return None;
        }
        *state.active.entry(id.into()).or_default() += 1;
        Some(NodeGuard {
            registry: self.clone(),
            id: id.into(),
        })
    }
}
pub(crate) struct NodeGuard {
    registry: Arc<NodeRegistry>,
    id: String,
}
impl Drop for NodeGuard {
    fn drop(&mut self) {
        let mut state = self.registry.0.lock().unwrap();
        if let Some(count) = state.active.get_mut(&self.id) {
            *count -= 1;
            if *count == 0 {
                state.active.remove(&self.id);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restrictions_do_not_erase_active_dials_and_stale_publications_cannot_reenable() {
        let registry = Arc::new(NodeRegistry::default());
        let guard = registry.begin("a").unwrap();
        registry.publish(NodeControl {
            node_id: "a".into(),
            state: NodeOperationState::Draining,
            version: 2,
            updated_at: 1,
        });
        assert!(registry.begin("a").is_none());
        assert_eq!(registry.active("a"), 1);
        registry.publish(NodeControl {
            node_id: "a".into(),
            state: NodeOperationState::Enabled,
            version: 1,
            updated_at: 1,
        });
        assert!(registry.begin("a").is_none());
        drop(guard);
        assert_eq!(registry.active("a"), 0);
        assert!(registry.0.lock().unwrap().active.is_empty());
    }
}
