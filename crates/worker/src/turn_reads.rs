// Turn-scoped reads: what one phase's setup read, kept for the turn's next
// phases on this worker.
//
// Why: every phase of a turn (reason, act, reason, ...) builds a fresh host
// whose setup reads the agent, harness, session, turn context, model and
// provider again (see `phase_reads`). Within a turn those barely change, yet
// for a gRPC worker each read is a control-plane round trip, and the turn
// context alone is a dozen sequential queries on the server. In production that
// setup cost 90-380 ms per phase, paid again at every tool hand-off.
//
// Decision: since the worker that finished a phase runs the turn's next phase
// itself (the turn driver chains steps), the values a phase's setup read are
// kept per turn, keyed by the turn's org, session and input message, and the
// next phase's setup starts from them. The turn's configuration is pinned for
// the turn on this worker, the way the agent definition already is: an edit
// made while the turn runs applies from the next turn. Session writes made
// through the worker (title, status) drop the session's entries, as they do
// for the phase memo. Only setup reads go through it; once a phase has started,
// reads go to the store as before.
//
// Decision: entries end with the turn (the driver plans `Complete` or a pause),
// and a turn that never reaches that (a crash, a reclaim elsewhere) ages out
// after `IDLE_FOR`. At most `MAX_TURNS` turns are kept; the least recently
// used goes first. Errors are never kept.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::core::{AgentDefinition, DependencyBlocker, ExecutionSession, HarnessDefinition};
use crate::worker_adapters::TurnContext;
use everruns_contracts::driver_registry::ProviderConfig;
use everruns_contracts::model_spec::ModelSpec;
use uuid::Uuid;

/// A turn with no phase for this long is dropped.
const IDLE_FOR: Duration = Duration::from_secs(10 * 60);

/// Turns kept at once per worker.
const MAX_TURNS: usize = 512;

/// Org and record id.
pub(crate) type Key = (i64, Uuid);
/// Org, session, and input message.
pub(crate) type TurnKey = (i64, Uuid, Uuid);
/// Org, provider, and the session whose overrides apply, if any.
pub(crate) type ProviderKeyed = (i64, String, Option<Uuid>);
/// A definition read: the definition (absent when none exists) and its
/// lifecycle blocker.
pub(crate) type Definition<T> = (Option<T>, Option<DependencyBlocker>);

/// What one turn's setup reads returned.
#[derive(Default)]
pub(crate) struct TurnValues {
    pub agents: HashMap<Key, Definition<AgentDefinition>>,
    pub harnesses: HashMap<Key, Definition<HarnessDefinition>>,
    pub sessions: HashMap<Key, Option<ExecutionSession>>,
    pub turn_contexts: HashMap<TurnKey, TurnContext>,
    pub models: HashMap<Key, Option<ModelSpec>>,
    pub providers: HashMap<ProviderKeyed, Option<ProviderConfig>>,
}

pub(crate) type Select<K, T> = fn(&mut TurnValues) -> &mut HashMap<K, T>;

/// One turn's kept reads, shared by its phases' memos.
#[derive(Clone, Default)]
pub struct TurnSlot(Arc<Mutex<TurnValues>>);

impl TurnSlot {
    pub(crate) fn get<K: Hash + Eq, T: Clone>(&self, select: Select<K, T>, key: &K) -> Option<T> {
        self.with(|values| select(values).get(key).cloned())
    }

    pub(crate) fn put<K: Hash + Eq, T>(&self, select: Select<K, T>, key: K, value: T) {
        self.with(|values| {
            select(values).insert(key, value);
        });
    }

    /// Forget the session after a write. The turn context carries it too.
    pub(crate) fn invalidate_session(&self, org_id: i64, session_id: Uuid) {
        self.with(|values| {
            values.sessions.remove(&(org_id, session_id));
            values
                .turn_contexts
                .retain(|&(org, session, _), _| (org, session) != (org_id, session_id));
        });
    }

    fn with<R>(&self, f: impl FnOnce(&mut TurnValues) -> R) -> R {
        // A poisoned lock only means a reader panicked mid-insert; the maps
        // are still valid, and a cache must never fail a phase.
        let mut values = self.0.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut values)
    }
}

/// The worker's kept turns.
#[derive(Default)]
pub struct TurnReads {
    turns: Mutex<HashMap<TurnKey, (Instant, TurnSlot)>>,
}

impl TurnReads {
    /// The slot for a turn, created when missing.
    pub fn slot(&self, org_id: i64, session_id: Uuid, input_message_id: Uuid) -> TurnSlot {
        let now = Instant::now();
        let mut turns = self.turns.lock().unwrap_or_else(|p| p.into_inner());
        turns.retain(|_, (used, _)| now.duration_since(*used) < IDLE_FOR);
        let key = (org_id, session_id, input_message_id);
        if !turns.contains_key(&key)
            && turns.len() >= MAX_TURNS
            && let Some(oldest) = turns
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .map(|(key, _)| *key)
        {
            turns.remove(&oldest);
        }
        let (used, slot) = turns
            .entry(key)
            .or_insert_with(|| (now, TurnSlot::default()));
        *used = now;
        slot.clone()
    }

    /// Drop a turn's reads once it has ended.
    pub fn end(&self, org_id: i64, session_id: Uuid, input_message_id: Uuid) {
        self.turns
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&(org_id, session_id, input_message_id));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.turns.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sessions(values: &mut TurnValues) -> &mut HashMap<Key, Option<ExecutionSession>> {
        &mut values.sessions
    }

    #[test]
    fn a_turn_shares_one_slot_until_it_ends() {
        let reads = TurnReads::default();
        let (session, message) = (Uuid::now_v7(), Uuid::now_v7());
        reads
            .slot(1, session, message)
            .put(sessions, (1, session), None);
        assert!(
            reads
                .slot(1, session, message)
                .get(sessions, &(1, session))
                .is_some()
        );
        // Another turn of the same session starts empty.
        assert!(
            reads
                .slot(1, session, Uuid::now_v7())
                .get(sessions, &(1, session))
                .is_none()
        );
        reads.end(1, session, message);
        assert!(
            reads
                .slot(1, session, message)
                .get(sessions, &(1, session))
                .is_none()
        );
    }

    #[test]
    fn a_session_write_drops_its_reads() {
        let slot = TurnSlot::default();
        let session = Uuid::now_v7();
        slot.put(sessions, (1, session), None);
        slot.invalidate_session(1, session);
        assert!(slot.get(sessions, &(1, session)).is_none());
    }

    #[test]
    fn the_least_recently_used_turn_goes_first() {
        let reads = TurnReads::default();
        let first = (Uuid::now_v7(), Uuid::now_v7());
        reads.slot(1, first.0, first.1);
        for _ in 1..MAX_TURNS {
            reads.slot(1, Uuid::now_v7(), Uuid::now_v7());
        }
        assert_eq!(reads.len(), MAX_TURNS);
        // Using the first turn again keeps it; the next new turn evicts
        // another.
        let kept = reads.slot(1, first.0, first.1);
        kept.put(sessions, (1, first.0), None);
        reads.slot(1, Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(reads.len(), MAX_TURNS);
        assert!(
            reads
                .slot(1, first.0, first.1)
                .get(sessions, &(1, first.0))
                .is_some()
        );
    }
}
