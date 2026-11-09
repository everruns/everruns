//! Bounded cache of idle hosted sessions (data plane v2 plan step 1).
//!
//! Decision: an Engine that runs hosted sessions keeps each one's cache
//! entry (Agent, Harness, binding) and live controls after its turn ends.
//! An executor sees far more sessions than it runs at once, so without a
//! bound that memory grows with every session the process has ever run.
//! Everything in a hosted entry is rebuilt from the authority on the next
//! activation, query or resume (`Engine::cache_hosted`), so an idle one can
//! be dropped at any time. Not dropped: entries with a live `Session`
//! handle, volatile sessions (their creation cell and process-local
//! Environment are the only copy), and live controls that carry MCP edits
//! or are held by a running turn.
//!
//! Eviction is least recently used, in batches down to three quarters of
//! the capacity, so the sort is paid once per quarter capacity of new
//! sessions rather than on every insert.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{EngineSessionEntry, SessionId};

pub(super) const DEFAULT_CAPACITY: usize = 1_024;

pub(super) struct SessionCache {
    capacity: usize,
    clock: AtomicU64,
}

impl SessionCache {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            clock: AtomicU64::new(1),
        }
    }

    /// A recency stamp for an entry the Engine just used.
    pub(super) fn tick(&self) -> u64 {
        self.clock.fetch_add(1, Ordering::Relaxed)
    }

    /// Drop least recently used idle hosted entries once the cache is over
    /// capacity; returns whether it trimmed.
    pub(super) fn trim(&self, sessions: &mut HashMap<SessionId, EngineSessionEntry>) -> bool {
        if sessions.len() <= self.capacity {
            return false;
        }
        let target = self.capacity - self.capacity / 4;
        let mut idle: Vec<(u64, SessionId)> = sessions
            .iter()
            .filter(|(_, entry)| entry.evictable())
            .map(|(id, entry)| (entry.used, *id))
            .collect();
        idle.sort_unstable_by_key(|(used, _)| *used);
        let excess = sessions.len().saturating_sub(target);
        idle.truncate(excess);
        for (_, id) in &idle {
            sessions.remove(id);
        }
        true
    }
}

impl EngineSessionEntry {
    fn evictable(&self) -> bool {
        self.durable_binding.is_some()
            && self.creation.is_none()
            && !self.provisional
            && self.selected_environment.is_none()
            && self
                .state
                .as_ref()
                .is_none_or(|state| state.strong_count() == 0)
    }
}

/// Live controls of sessions no longer cached go too, unless a turn or
/// caller still holds them or they carry process-local MCP edits; model and
/// effort are re-read from the authority on the next use
/// (`hosted_live_controls`). A sweep rather than the dropped list: a session
/// dropped while its turn ran (no open handle) still held its controls then.
pub(super) fn trim_live_controls(
    controls: &mut HashMap<SessionId, Arc<crate::session::LiveControls>>,
    sessions: &HashMap<SessionId, EngineSessionEntry>,
) {
    controls.retain(|id, live| {
        sessions.contains_key(id) || Arc::strong_count(live) > 1 || live.has_mcp_edits()
    });
}
