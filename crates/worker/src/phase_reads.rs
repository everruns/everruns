// Per-phase memo of the agent, harness, and session reads that phase setup
// repeats, plus a `phase setup` timing line.
//
// Why: before a reason phase reaches the model, host setup asks for the same
// session four times, the harness three times, and the agent twice (dependency
// check, capability loading, snapshot projection, tool augmentation). For a gRPC
// worker every read is a round trip to the control plane and from there to the
// database. In production (database a few milliseconds away) that made reason
// setup ~600 ms of every ~700 ms phase hand-off, against ~130 ms locally.
//
// Decision: one memo per `WorkerRuntimeHost`, which the worker builds per
// activity, so nothing is shared across phases or turns. It serves setup only:
// once the phase's start event goes out it empties and stops memoizing, so the
// model call, tools, and anything after them read fresh state. Entries also
// expire after `FRESH_FOR`, for hosts that never emit a start event (activity
// scheduling). Session writes made through the host drop the session entry.
// Errors are never memoized.
//
// Decision: the host's setup has no timing of its own, and production ships logs
// only, so the memo also logs once per phase how long setup took (host creation
// to the phase's start event) and how many reads it made and saved.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use everruns_contracts::error::Result;
use everruns_core::{AgentDefinition, DependencyBlocker, ExecutionSession, HarnessDefinition};
use uuid::Uuid;

use crate::worker_adapters::WorkerAdapters;

/// How long a memoized read stays usable. Covers setup (under a second even
/// with a remote database).
const FRESH_FOR: Duration = Duration::from_secs(2);

type Key = (i64, Uuid);

struct Entry<T> {
    at: Instant,
    value: T,
}

// Cache successful projections (including absence) separately from lifecycle
// probes. A failed projection is retried, while its successful blocker read can
// still be reused; AgentLoopError is deliberately never cloned or cached.
#[derive(Clone)]
struct DefinitionRead<T> {
    definition: Option<Option<T>>,
    blocker: Option<DependencyBlocker>,
}

type ResolvedRead<T> = (Result<Option<T>>, Option<DependencyBlocker>);

#[derive(Default)]
struct State {
    agents: HashMap<Key, Entry<DefinitionRead<AgentDefinition>>>,
    harnesses: HashMap<Key, Entry<DefinitionRead<HarnessDefinition>>>,
    sessions: HashMap<Key, Entry<Option<ExecutionSession>>>,
    fetched: u32,
    saved: u32,
    /// The phase started: stop memoizing.
    setup_done: bool,
}

fn agents(s: &mut State) -> &mut HashMap<Key, Entry<DefinitionRead<AgentDefinition>>> {
    &mut s.agents
}

fn harnesses(s: &mut State) -> &mut HashMap<Key, Entry<DefinitionRead<HarnessDefinition>>> {
    &mut s.harnesses
}

fn sessions(s: &mut State) -> &mut HashMap<Key, Entry<Option<ExecutionSession>>> {
    &mut s.sessions
}

/// Agent, harness, and session reads for one phase, memoized briefly.
#[derive(Clone)]
pub struct PhaseReads {
    started: Instant,
    state: Arc<Mutex<State>>,
}

impl Default for PhaseReads {
    fn default() -> Self {
        Self::new()
    }
}

impl PhaseReads {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            state: Arc::default(),
        }
    }

    pub async fn agent<A: WorkerAdapters>(
        &self,
        adapters: &A,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<AgentDefinition>> {
        self.definition(agents, (org_id, id), || {
            adapters.resolve_agent_read(org_id, id)
        })
        .await
    }

    pub async fn agent_blocker<A: WorkerAdapters>(
        &self,
        adapters: &A,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<DependencyBlocker>> {
        self.blocker(agents, (org_id, id), || {
            adapters.resolve_agent_read(org_id, id)
        })
        .await
    }

    pub async fn harness<A: WorkerAdapters>(
        &self,
        adapters: &A,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<HarnessDefinition>> {
        self.definition(harnesses, (org_id, id), || {
            adapters.resolve_harness_read(org_id, id)
        })
        .await
    }

    pub async fn harness_blocker<A: WorkerAdapters>(
        &self,
        adapters: &A,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<DependencyBlocker>> {
        self.blocker(harnesses, (org_id, id), || {
            adapters.resolve_harness_read(org_id, id)
        })
        .await
    }

    pub async fn session<A: WorkerAdapters>(
        &self,
        adapters: &A,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<ExecutionSession>> {
        self.read_through(sessions, (org_id, id), || adapters.get_session(org_id, id))
            .await
    }

    /// Keep execution views another read returned (the batched turn context
    /// carries the session and agent), so later lookups reuse them.
    pub fn seed(&self, org_id: i64, session: &ExecutionSession, agent: Option<&AgentDefinition>) {
        let session_id = session.id.uuid();
        self.store(sessions, (org_id, session_id), Some(session.clone()), false);
        if let Some(agent) = agent {
            let key = (org_id, agent.id.uuid());
            self.store(
                agents,
                key,
                DefinitionRead {
                    definition: Some(Some(agent.clone())),
                    blocker: None,
                },
                false,
            );
        }
    }

    /// Forget the session after a write, so the next read sees it.
    pub fn invalidate_session(&self, org_id: i64, id: Uuid) {
        self.with_state(|s| s.sessions.remove(&(org_id, id)));
    }

    /// Log the phase's setup time when its start event goes out. Once per memo.
    pub fn note_event(&self, event_type: &str) {
        if event_type != everruns_core::REASON_STARTED && event_type != everruns_core::ACT_STARTED {
            return;
        }
        let (fetched, saved, first) = self.with_state(|s| {
            let counts = (s.fetched, s.saved, !s.setup_done);
            *s = State {
                setup_done: true,
                ..State::default()
            };
            counts
        });
        if first {
            tracing::info!(
                phase = event_type,
                setup_ms = self.started.elapsed().as_millis() as u64,
                reads_fetched = fetched,
                reads_saved = saved,
                "phase setup"
            );
        }
    }

    async fn definition<T: Clone, F: Future<Output = Result<ResolvedRead<T>>>>(
        &self,
        map: fn(&mut State) -> &mut HashMap<Key, Entry<DefinitionRead<T>>>,
        key: Key,
        fetch: impl FnOnce() -> F,
    ) -> Result<Option<T>> {
        if let Some(hit) = self.lookup_selected(map, key, |value| value.definition.clone()) {
            return Ok(hit);
        }
        self.resolve(map, key, fetch).await?.0
    }

    async fn blocker<T: Clone, F: Future<Output = Result<ResolvedRead<T>>>>(
        &self,
        map: fn(&mut State) -> &mut HashMap<Key, Entry<DefinitionRead<T>>>,
        key: Key,
        fetch: impl FnOnce() -> F,
    ) -> Result<Option<DependencyBlocker>> {
        if let Some(hit) = self.lookup_selected(map, key, |value| Some(value.blocker)) {
            return Ok(hit);
        }
        Ok(self.resolve(map, key, fetch).await?.1)
    }

    async fn resolve<T: Clone, F: Future<Output = Result<ResolvedRead<T>>>>(
        &self,
        map: fn(&mut State) -> &mut HashMap<Key, Entry<DefinitionRead<T>>>,
        key: Key,
        fetch: impl FnOnce() -> F,
    ) -> Result<ResolvedRead<T>> {
        let (definition, blocker) = fetch().await?;
        self.store(
            map,
            key,
            DefinitionRead {
                definition: definition.as_ref().ok().cloned(),
                blocker,
            },
            true,
        );
        Ok((definition, blocker))
    }

    async fn read_through<T: Clone, F: Future<Output = Result<T>>>(
        &self,
        map: fn(&mut State) -> &mut HashMap<Key, Entry<T>>,
        key: Key,
        fetch: impl FnOnce() -> F,
    ) -> Result<T> {
        if let Some(hit) = self.lookup(map, key) {
            return Ok(hit);
        }
        let value = fetch().await?;
        self.store(map, key, value.clone(), true);
        Ok(value)
    }

    fn lookup<T: Clone>(
        &self,
        map: fn(&mut State) -> &mut HashMap<Key, Entry<T>>,
        key: Key,
    ) -> Option<T> {
        self.lookup_selected(map, key, |value| Some(value.clone()))
    }

    fn lookup_selected<T, R>(
        &self,
        map: fn(&mut State) -> &mut HashMap<Key, Entry<T>>,
        key: Key,
        select: impl FnOnce(&T) -> Option<R>,
    ) -> Option<R> {
        self.with_state(|s| {
            let hit = map(s)
                .get(&key)
                .filter(|entry| entry.at.elapsed() < FRESH_FOR)
                .and_then(|entry| select(&entry.value));
            if hit.is_some() {
                s.saved += 1;
            }
            hit
        })
    }

    fn store<T>(
        &self,
        map: fn(&mut State) -> &mut HashMap<Key, Entry<T>>,
        key: Key,
        value: T,
        fetched: bool,
    ) {
        self.with_state(|s| {
            if s.setup_done {
                return;
            }
            if fetched {
                s.fetched += 1;
            }
            map(s).insert(
                key,
                Entry {
                    at: Instant::now(),
                    value,
                },
            );
        });
    }

    fn with_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        // A poisoned lock only means another reader panicked mid-insert; the
        // maps are still valid, and a memo must never fail a phase.
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::typed_id::{HarnessId, SessionId};
    use std::sync::atomic::{AtomicU32, Ordering};

    fn session() -> ExecutionSession {
        ExecutionSession::with_own_workspace(SessionId::new(), HarnessId::new())
    }

    /// Read `key` through the memo, counting how often it reaches the store.
    async fn read(
        reads: &PhaseReads,
        key: Key,
        calls: &AtomicU32,
    ) -> Result<Option<ExecutionSession>> {
        reads
            .read_through(sessions, key, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(Some(session()))
            })
            .await
    }

    #[tokio::test]
    async fn setup_reads_reach_the_store_once() {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let key = (1, Uuid::now_v7());
        let first = read(&reads, key, &calls).await.unwrap().unwrap();
        let again = read(&reads, key, &calls).await.unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(first.id, again.id);
        // Another org's record with the same id is a different read.
        read(&reads, (2, key.1), &calls).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn the_phase_start_event_ends_memoizing() {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let key = (1, Uuid::now_v7());
        read(&reads, key, &calls).await.unwrap();
        // Events before the start event leave setup running.
        reads.note_event(everruns_core::INPUT_MESSAGE);
        read(&reads, key, &calls).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        reads.note_event(everruns_core::REASON_STARTED);
        read(&reads, key, &calls).await.unwrap();
        read(&reads, key, &calls).await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "reads after setup are fresh"
        );
    }

    #[tokio::test]
    async fn a_session_write_drops_the_memoized_session() {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let key = (1, Uuid::now_v7());
        read(&reads, key, &calls).await.unwrap();
        reads.invalidate_session(key.0, key.1);
        read(&reads, key, &calls).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn failed_reads_are_not_memoized() {
        let reads = PhaseReads::new();
        let key = (1, Uuid::now_v7());
        let failed = reads
            .read_through(sessions, key, || async {
                Err(everruns_contracts::error::AgentLoopError::store("down"))
            })
            .await;
        assert!(failed.is_err());
        let calls = AtomicU32::new(0);
        read(&reads, key, &calls).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_seeded_session_needs_no_fetch() {
        let reads = PhaseReads::new();
        let seeded = session();
        reads.seed(1, &seeded, None);
        let calls = AtomicU32::new(0);
        let got = read(&reads, (1, seeded.id.uuid()), &calls)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.id, seeded.id);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn lifecycle_and_definition_setup_share_one_source_read() {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let id = everruns_contracts::typed_id::AgentId::new();
        let key = (1, id.uuid());
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((
                Ok(Some(AgentDefinition::new(id, "agent", "pinned prompt"))),
                None,
            ))
        };
        assert!(reads.blocker(agents, key, fetch).await.unwrap().is_none());
        assert_eq!(
            reads
                .definition(agents, key, fetch)
                .await
                .unwrap()
                .unwrap()
                .system_prompt,
            "pinned prompt"
        );
        assert!(reads.blocker(agents, key, fetch).await.unwrap().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        reads.definition(agents, (2, key.1), fetch).await.unwrap();
        PhaseReads::new()
            .definition(agents, key, fetch)
            .await
            .unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "org and phase boundaries remain isolated"
        );
    }

    #[tokio::test]
    async fn missing_harness_is_a_cached_negative_definition_and_deleted_blocker() {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let key = (1, Uuid::now_v7());
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((Ok(None), Some(DependencyBlocker::HarnessDeleted)))
        };
        assert!(
            reads
                .definition(harnesses, key, fetch)
                .await
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            reads.blocker(harnesses, key, fetch).await.unwrap(),
            Some(DependencyBlocker::HarnessDeleted)
        ));
        assert!(
            reads
                .definition(harnesses, key, fetch)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn projection_failures_retry_without_losing_the_successful_lifecycle_probe() {
        use everruns_contracts::error::AgentLoopError;
        for blocker in [
            None,
            Some(DependencyBlocker::AgentArchived),
            Some(DependencyBlocker::AgentDeleted),
        ] {
            let reads = PhaseReads::new();
            let calls = AtomicU32::new(0);
            let key = (1, Uuid::now_v7());
            let fetch = || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok((
                    Err(AgentLoopError::config("original projection refusal")),
                    blocker,
                ))
            };
            let got = reads.blocker(agents, key, fetch).await.unwrap();
            assert_eq!(
                got.map(DependencyBlocker::message),
                blocker.map(DependencyBlocker::message)
            );
            for _ in 0..2 {
                let error = reads.definition(agents, key, fetch).await.unwrap_err();
                assert!(
                    matches!(error, AgentLoopError::Configuration(message) if message == "original projection refusal")
                );
                reads.blocker(agents, key, fetch).await.unwrap();
            }
            assert_eq!(calls.load(Ordering::SeqCst), 3);
            assert_eq!(
                reads.with_state(|state| state.saved),
                2,
                "failed projections are not saved reads"
            );
            let recovered = || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok((
                    Ok(Some(AgentDefinition::new(
                        everruns_contracts::typed_id::AgentId::from_uuid(key.1),
                        "recovered",
                        "valid projection",
                    ))),
                    None,
                ))
            };
            let got = reads
                .definition(agents, key, recovered)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(got.system_prompt, "valid projection");
            assert!(
                reads
                    .blocker(agents, key, recovered)
                    .await
                    .unwrap()
                    .is_none()
            );
            reads.definition(agents, key, recovered).await.unwrap();
            assert_eq!(
                calls.load(Ordering::SeqCst),
                4,
                "recovery becomes a successful memoized projection"
            );
        }
    }

    #[tokio::test]
    async fn failed_combined_source_reads_are_never_cached() {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let key = (1, Uuid::now_v7());
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(everruns_contracts::error::AgentLoopError::store(
                "source unavailable",
            ))
        };
        assert!(reads.blocker(agents, key, fetch).await.is_err());
        assert!(reads.definition(agents, key, fetch).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(reads.with_state(|state| state.agents.is_empty()));
    }

    #[tokio::test]
    async fn expired_definitions_and_blockers_are_refetched() {
        let reads = PhaseReads::new();
        let key = (1, Uuid::now_v7());
        let calls = AtomicU32::new(0);
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((Ok(Some(HarnessDefinition::new("harness", "prompt"))), None))
        };
        reads.definition(harnesses, key, fetch).await.unwrap();
        reads.with_state(|state| {
            state.harnesses.get_mut(&key).unwrap().at = Instant::now() - FRESH_FOR
        });
        reads.blocker(harnesses, key, fetch).await.unwrap();
        reads.definition(harnesses, key, fetch).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn phase_start_discards_an_in_flight_source_read_and_later_seeds() {
        for event in [everruns_core::REASON_STARTED, everruns_core::ACT_STARTED] {
            let reads = PhaseReads::new();
            let id = everruns_contracts::typed_id::AgentId::new();
            let definition = AgentDefinition::new(id, "agent", "pinned");
            let key = (1, id.uuid());
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
            let pending_reads = reads.clone();
            let pending_definition = definition.clone();
            let pending = tokio::spawn(async move {
                pending_reads
                    .definition(agents, key, || async {
                        started_tx.send(()).unwrap();
                        finish_rx.await.unwrap();
                        Ok((Ok(Some(pending_definition)), None))
                    })
                    .await
            });
            started_rx.await.unwrap();
            reads.note_event(event);
            finish_tx.send(()).unwrap();
            assert_eq!(pending.await.unwrap().unwrap().unwrap().id, id);
            reads.seed(1, &session(), Some(&definition));
            assert!(reads.with_state(|state| state.agents.is_empty() && state.sessions.is_empty()));
            let calls = AtomicU32::new(0);
            let fetch = || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok((Ok(Some(definition.clone())), None))
            };
            reads.definition(agents, key, fetch).await.unwrap();
            reads.blocker(agents, key, fetch).await.unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        }
    }

    #[tokio::test]
    async fn seeded_batch_agent_keeps_its_pinned_definition_without_a_fetch() {
        let reads = PhaseReads::new();
        let id = everruns_contracts::typed_id::AgentId::new();
        let pinned = AgentDefinition::new(id, "agent", "pinned batch version");
        reads.seed(1, &session(), Some(&pinned));
        let fetch = || async {
            Err(everruns_contracts::error::AgentLoopError::store(
                "must not fetch current version",
            ))
        };
        assert!(
            reads
                .blocker(agents, (1, id.uuid()), fetch)
                .await
                .unwrap()
                .is_none()
        );
        let got = reads
            .definition(agents, (1, id.uuid()), fetch)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.system_prompt, pinned.system_prompt);
    }
}
