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
// Decision: the worker knows every id these reads need when it claims the
// task, so `prefetch` starts them all at once instead of letting setup issue
// them one after another (harness, agent, session, then the turn context).
// Each read is a cell that later callers wait on while it is in flight, so a
// prefetched read is never repeated. With the control plane tens of
// milliseconds away, that turns four round trips before the model call into
// one.
//
// Decision: the host's setup has no timing of its own, and production ships logs
// only, so the memo also logs once per phase how long setup took (host creation
// to the phase's start event) and how many reads it made and saved.

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::core::{AgentDefinition, DependencyBlocker, ExecutionSession, HarnessDefinition};
use everruns_contracts::error::Result;
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::worker_adapters::{TurnContext, WorkerAdapters};
use everruns_contracts::driver_registry::ProviderConfig;
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::runtime_provider::ProviderKey;
use everruns_contracts::typed_id::SessionId;

/// How long a memoized read stays usable. Covers setup (under a second even
/// with a remote database).
const FRESH_FOR: Duration = Duration::from_secs(2);

use crate::turn_reads::{
    Definition as TurnDefinition, Key, ProviderKeyed, Select, TurnKey, TurnSlot, TurnValues,
};

struct Entry<T> {
    at: Instant,
    cell: Arc<OnceCell<T>>,
    pinned: Option<T>,
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
    turn_contexts: HashMap<TurnKey, Entry<TurnContext>>,
    models: HashMap<Key, Entry<Option<ModelSpec>>>,
    providers: HashMap<ProviderKeyed, Entry<Option<ProviderConfig>>>,
    fetched: u32,
    saved: u32,
    /// Fetches the turn's earlier phases answered (counted in `fetched` too).
    from_turn: u32,
    /// The phase started: stop memoizing.
    setup_done: bool,
}

type Map<K, T> = fn(&mut State) -> &mut HashMap<K, Entry<T>>;

fn agents(s: &mut State) -> &mut HashMap<Key, Entry<DefinitionRead<AgentDefinition>>> {
    &mut s.agents
}

fn harnesses(s: &mut State) -> &mut HashMap<Key, Entry<DefinitionRead<HarnessDefinition>>> {
    &mut s.harnesses
}

fn sessions(s: &mut State) -> &mut HashMap<Key, Entry<Option<ExecutionSession>>> {
    &mut s.sessions
}

fn turn_contexts(s: &mut State) -> &mut HashMap<TurnKey, Entry<TurnContext>> {
    &mut s.turn_contexts
}

fn models(s: &mut State) -> &mut HashMap<Key, Entry<Option<ModelSpec>>> {
    &mut s.models
}

fn providers(s: &mut State) -> &mut HashMap<ProviderKeyed, Entry<Option<ProviderConfig>>> {
    &mut s.providers
}

fn turn_agents(v: &mut TurnValues) -> &mut HashMap<Key, TurnDefinition<AgentDefinition>> {
    &mut v.agents
}

fn turn_harnesses(v: &mut TurnValues) -> &mut HashMap<Key, TurnDefinition<HarnessDefinition>> {
    &mut v.harnesses
}

fn turn_sessions(v: &mut TurnValues) -> &mut HashMap<Key, Option<ExecutionSession>> {
    &mut v.sessions
}

fn turn_turn_contexts(v: &mut TurnValues) -> &mut HashMap<TurnKey, TurnContext> {
    &mut v.turn_contexts
}

fn turn_models(v: &mut TurnValues) -> &mut HashMap<Key, Option<ModelSpec>> {
    &mut v.models
}

fn turn_providers(v: &mut TurnValues) -> &mut HashMap<ProviderKeyed, Option<ProviderConfig>> {
    &mut v.providers
}

/// The ids a phase's setup reads. Known when the worker claims the task.
pub struct PhaseIds {
    pub org_id: i64,
    pub session_id: Uuid,
    pub harness_id: Uuid,
    pub agent_id: Option<Uuid>,
    /// Set for a reason phase, which also loads the turn context.
    pub input_message_id: Option<Uuid>,
}

impl PhaseIds {
    pub fn reason(input: &crate::core::engine::ReasonInput) -> Option<Self> {
        let message_id = input.context.input_message_id.uuid();
        Some(Self {
            org_id: input.org_id,
            session_id: input.context.session_id.uuid(),
            harness_id: input.harness_id.uuid(),
            agent_id: input.agent_id.map(|id| id.uuid()),
            input_message_id: (!message_id.is_nil()).then_some(message_id),
        })
    }

    pub fn act(input: &crate::core::engine::ActInput) -> Option<Self> {
        Some(Self {
            org_id: input.org_id?,
            session_id: input.context.session_id.uuid(),
            harness_id: input.harness_id.uuid(),
            agent_id: input.agent_id.map(|id| id.uuid()),
            input_message_id: None,
        })
    }
}

/// Agent, harness, session, and turn-context reads for one phase, memoized
/// briefly.
#[derive(Clone)]
pub struct PhaseReads {
    started: Instant,
    state: Arc<Mutex<State>>,
    /// What earlier phases of this turn read on this worker (see
    /// `turn_reads`). Setup starts from it and adds what it fetches.
    turn: Option<TurnSlot>,
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
            turn: None,
        }
    }

    /// Share setup reads with the turn's other phases on this worker.
    #[must_use]
    pub fn with_turn(mut self, turn: TurnSlot) -> Self {
        self.turn = Some(turn);
        self
    }

    /// Start every read setup will make, concurrently, so setup waits on them
    /// instead of issuing them in turn.
    pub fn prefetch<A: WorkerAdapters>(&self, adapters: &A, ids: PhaseIds) {
        let PhaseIds {
            org_id,
            session_id,
            harness_id,
            agent_id,
            input_message_id,
        } = ids;
        let (reads, a) = (self.clone(), adapters.clone());
        tokio::spawn(async move {
            let _ = reads.session(&a, org_id, session_id).await;
        });
        let (reads, a) = (self.clone(), adapters.clone());
        tokio::spawn(async move {
            let _ = reads.harness(&a, org_id, harness_id).await;
        });
        if let Some(agent_id) = agent_id {
            let (reads, a) = (self.clone(), adapters.clone());
            tokio::spawn(async move {
                let _ = reads.agent(&a, org_id, agent_id).await;
            });
        }
        if let Some(message_id) = input_message_id {
            let (reads, a) = (self.clone(), adapters.clone());
            tokio::spawn(async move {
                let _ = reads.turn_context(&a, org_id, session_id, message_id).await;
            });
        }
    }

    pub async fn agent<A: WorkerAdapters>(
        &self,
        adapters: &A,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<AgentDefinition>> {
        self.definition(agents, (org_id, id), || {
            self.turn_definition(turn_agents, (org_id, id), || {
                adapters.resolve_agent_read(org_id, id)
            })
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
            self.turn_definition(turn_agents, (org_id, id), || {
                adapters.resolve_agent_read(org_id, id)
            })
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
            self.turn_definition(turn_harnesses, (org_id, id), || {
                adapters.resolve_harness_read(org_id, id)
            })
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
            self.turn_definition(turn_harnesses, (org_id, id), || {
                adapters.resolve_harness_read(org_id, id)
            })
        })
        .await
    }

    pub async fn session<A: WorkerAdapters>(
        &self,
        adapters: &A,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<ExecutionSession>> {
        self.read_through(sessions, (org_id, id), || {
            self.turn_read(turn_sessions, (org_id, id), || {
                adapters.get_session(org_id, id)
            })
        })
        .await
    }

    /// The batched turn context a reason phase executes from.
    pub async fn turn_context<A: WorkerAdapters>(
        &self,
        adapters: &A,
        org_id: i64,
        session_id: Uuid,
        input_message_id: Uuid,
    ) -> Result<TurnContext> {
        self.read_through(
            turn_contexts,
            (org_id, session_id, input_message_id),
            || {
                self.turn_read(
                    turn_turn_contexts,
                    (org_id, session_id, input_message_id),
                    || {
                        adapters.load_turn_context_for_execution(
                            org_id,
                            session_id,
                            input_message_id,
                        )
                    },
                )
            },
        )
        .await
    }

    pub async fn model_spec<A: WorkerAdapters>(
        &self,
        adapters: &A,
        org_id: i64,
        model_id: Uuid,
    ) -> Result<Option<ModelSpec>> {
        self.read_through(models, (org_id, model_id), || {
            self.turn_read(turn_models, (org_id, model_id), || {
                adapters.get_model_spec(org_id, model_id)
            })
        })
        .await
    }

    /// The provider's configuration, credentials included, as seen by
    /// `session` when one is given.
    pub async fn provider_config<A: WorkerAdapters>(
        &self,
        adapters: &A,
        org_id: i64,
        provider: &ProviderKey,
        session: Option<SessionId>,
    ) -> Result<Option<ProviderConfig>> {
        let key = (
            org_id,
            provider.as_str().to_string(),
            session.map(|id| id.uuid()),
        );
        self.read_through(providers, key.clone(), || {
            self.turn_read(turn_providers, key, || async move {
                match session {
                    Some(session) => {
                        adapters
                            .get_provider_config_for_session(org_id, provider, session)
                            .await
                    }
                    None => adapters.get_provider_config(org_id, provider).await,
                }
            })
        })
        .await
    }

    /// Start resolving the turn's model and its provider as soon as the
    /// snapshot names it, so they are ready when context assembly asks after
    /// loading history. A model switched by an earlier message wastes this read.
    pub fn prefetch_model<A: WorkerAdapters>(
        &self,
        adapters: &A,
        org_id: i64,
        model_id: Uuid,
        session: SessionId,
    ) {
        let (reads, adapters) = (self.clone(), adapters.clone());
        tokio::spawn(async move {
            if let Ok(Some(spec)) = reads.model_spec(&adapters, org_id, model_id).await {
                let provider = &spec.provider;
                let _ = reads
                    .provider_config(&adapters, org_id, provider, Some(session))
                    .await;
            }
        });
    }

    /// Keep records another read already returned (the batched turn context
    /// carries the session and agent), so later lookups reuse them.
    pub fn seed(&self, org_id: i64, session: &ExecutionSession, agent: Option<&AgentDefinition>) {
        let session_id = session.id.uuid();
        self.store(sessions, (org_id, session_id), Some(session.clone()));
        if let Some(agent) = agent {
            let key = (org_id, agent.id.uuid());
            self.with_state(|state| {
                if state.setup_done {
                    return;
                }
                if state
                    .agents
                    .get(&key)
                    .is_some_and(|entry| entry.at.elapsed() >= FRESH_FOR)
                {
                    state.agents.remove(&key);
                }
                let entry = state.agents.entry(key).or_insert_with(|| Entry {
                    at: Instant::now(),
                    cell: Arc::new(OnceCell::new()),
                    pinned: None,
                });
                // The batched execution definition is pinned to this turn.
                // Keep the prefetch cell for its lifecycle probe, but overlay
                // its current definition even if that fetch completes later.
                let pinned = DefinitionRead {
                    definition: Some(Some(agent.clone())),
                    blocker: None,
                };
                entry.pinned = Some(pinned.clone());
                let _ = entry.cell.set(pinned);
            });
        }
    }

    /// Forget the session after a write, so the next read sees it. The turn
    /// context carries the session too.
    pub fn invalidate_session(&self, org_id: i64, id: Uuid) {
        if let Some(turn) = &self.turn {
            turn.invalidate_session(org_id, id);
        }
        self.with_state(|s| {
            s.sessions.remove(&(org_id, id));
            s.turn_contexts
                .retain(|&(org, session, _), _| (org, session) != (org_id, id));
        });
    }

    /// Log the phase's setup time when its start event goes out. Once per memo.
    pub fn note_event(&self, event_type: &str) {
        if event_type != crate::core::REASON_STARTED && event_type != crate::core::ACT_STARTED {
            return;
        }
        let (fetched, saved, from_turn, first) = self.with_state(|s| {
            let counts = (
                s.fetched.saturating_sub(s.from_turn),
                s.saved,
                s.from_turn,
                !s.setup_done,
            );
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
                reads_from_turn = from_turn,
                "phase setup"
            );
        }
    }

    /// The turn's slot, while setup runs.
    fn turn_slot(&self) -> Option<TurnSlot> {
        let turn = self.turn.clone()?;
        (!self.with_state(|s| s.setup_done)).then_some(turn)
    }

    /// `fetch`, answered from the turn's earlier phases when they read it.
    async fn turn_read<K, T, F>(
        &self,
        select: Select<K, T>,
        key: K,
        fetch: impl FnOnce() -> F,
    ) -> Result<T>
    where
        K: Hash + Eq,
        T: Clone,
        F: Future<Output = Result<T>>,
    {
        let Some(turn) = self.turn_slot() else {
            return fetch().await;
        };
        if let Some(value) = turn.get(select, &key) {
            self.with_state(|s| s.from_turn += 1);
            return Ok(value);
        }
        let value = fetch().await?;
        turn.put(select, key, value.clone());
        Ok(value)
    }

    /// A definition read, answered from the turn's earlier phases when they
    /// read it. A failed projection is not kept.
    async fn turn_definition<T, F>(
        &self,
        select: Select<Key, TurnDefinition<T>>,
        key: Key,
        fetch: impl FnOnce() -> F,
    ) -> Result<ResolvedRead<T>>
    where
        T: Clone,
        F: Future<Output = Result<ResolvedRead<T>>>,
    {
        let Some(turn) = self.turn_slot() else {
            return fetch().await;
        };
        if let Some((definition, blocker)) = turn.get(select, &key) {
            self.with_state(|s| s.from_turn += 1);
            return Ok((Ok(definition), blocker));
        }
        let (definition, blocker) = fetch().await?;
        if let Ok(value) = &definition {
            turn.put(select, key, (value.clone(), blocker));
        }
        Ok((definition, blocker))
    }

    async fn definition<T: Clone, F: Future<Output = Result<ResolvedRead<T>>>>(
        &self,
        map: Map<Key, DefinitionRead<T>>,
        key: Key,
        fetch: impl FnOnce() -> F,
    ) -> Result<Option<T>> {
        self.resolve(map, key, true, fetch).await?.0
    }

    async fn blocker<T: Clone, F: Future<Output = Result<ResolvedRead<T>>>>(
        &self,
        map: Map<Key, DefinitionRead<T>>,
        key: Key,
        fetch: impl FnOnce() -> F,
    ) -> Result<Option<DependencyBlocker>> {
        Ok(self.resolve(map, key, false, fetch).await?.1)
    }

    #[expect(
        clippy::expect_used,
        reason = "OnceCell invokes its initializer at most once; retry only follows another reader's failed projection"
    )]
    async fn resolve<T: Clone, F: Future<Output = Result<ResolvedRead<T>>>>(
        &self,
        map: Map<Key, DefinitionRead<T>>,
        key: Key,
        want_definition: bool,
        fetch: impl FnOnce() -> F,
    ) -> Result<ResolvedRead<T>> {
        let mut fetch = Some(fetch);
        loop {
            // A projection failure leaves a usable lifecycle probe, but the
            // original non-cloneable error belongs only to its initializer.
            // A subsequent definition read starts a fresh cell and retries.
            self.with_state(|state| {
                let entries = map(state);
                if want_definition
                    && entries.get(&key).is_some_and(|entry| {
                        entry.pinned.is_none()
                            && entry
                                .cell
                                .get()
                                .is_some_and(|value| value.definition.is_none())
                    })
                {
                    entries.remove(&key);
                }
            });
            let Some(cell) = self.cell(map, key) else {
                return fetch.take().expect("fetch not yet invoked")().await;
            };
            let mut resolved = None;
            let cached = cell
                .get_or_try_init(|| async {
                    let (definition, blocker) =
                        fetch.take().expect("initializer owns fetch")().await?;
                    let value = DefinitionRead {
                        definition: definition.as_ref().ok().cloned(),
                        blocker,
                    };
                    resolved = Some((definition, blocker));
                    Ok::<_, everruns_contracts::error::AgentLoopError>(value)
                })
                .await;
            let value = match cached {
                Ok(value) => value,
                Err(error) => {
                    self.with_state(|state| {
                        let entries = map(state);
                        if entries.get(&key).is_some_and(|entry| {
                            Arc::ptr_eq(&entry.cell, &cell)
                                && entry.cell.get().is_none()
                                && entry.pinned.is_none()
                        }) {
                            entries.remove(&key);
                        }
                    });
                    return Err(error);
                }
            };
            let pinned = self.with_state(|state| {
                map(state)
                    .get(&key)
                    .filter(|entry| Arc::ptr_eq(&entry.cell, &cell))
                    .and_then(|entry| entry.pinned.as_ref())
                    .and_then(|value| value.definition.clone())
            });
            if let Some((definition, blocker)) = resolved {
                self.with_state(|state| state.fetched += 1);
                return Ok((pinned.map(Ok).unwrap_or(definition), blocker));
            }
            if let Some(definition) = pinned.or_else(|| value.definition.clone()) {
                self.with_state(|state| state.saved += 1);
                return Ok((Ok(definition), value.blocker));
            }
            if !want_definition {
                self.with_state(|state| state.saved += 1);
                return Ok((Ok(None), value.blocker));
            }
            // We waited on another reader whose projection failed. Its error
            // cannot be cloned; our initializer remains available for retry.
        }
    }

    async fn read_through<K, T, F>(
        &self,
        map: Map<K, T>,
        key: K,
        fetch: impl FnOnce() -> F,
    ) -> Result<T>
    where
        K: Hash + Eq,
        T: Clone,
        F: Future<Output = Result<T>>,
    {
        let Some(cell) = self.cell(map, key) else {
            return fetch().await;
        };
        let mut fetched = false;
        let value = cell
            .get_or_try_init(|| {
                fetched = true;
                fetch()
            })
            .await?
            .clone();
        self.with_state(|s| {
            if fetched {
                s.fetched += 1;
            } else {
                s.saved += 1;
            }
        });
        Ok(value)
    }

    /// The fresh cell for `key`, created if missing. `None` once setup is done.
    fn cell<K: Hash + Eq, T>(&self, map: Map<K, T>, key: K) -> Option<Arc<OnceCell<T>>> {
        self.with_state(|s| {
            if s.setup_done {
                return None;
            }
            let map = map(s);
            if let Some(entry) = map.get(&key)
                && entry.at.elapsed() < FRESH_FOR
            {
                return Some(entry.cell.clone());
            }
            let cell = Arc::new(OnceCell::new());
            let at = Instant::now();
            map.insert(
                key,
                Entry {
                    at,
                    cell: cell.clone(),
                    pinned: None,
                },
            );
            Some(cell)
        })
    }

    /// Fill `key` with a value another read returned, unless it already has
    /// one or a read of it is in flight.
    fn store<K: Hash + Eq, T>(&self, map: Map<K, T>, key: K, value: T) {
        if let Some(cell) = self.cell(map, key) {
            let _ = cell.set(value);
        }
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

    /// Read `key` as `session` does: through the memo, then the turn.
    async fn read_in_turn(
        reads: &PhaseReads,
        key: Key,
        calls: &AtomicU32,
    ) -> Result<Option<ExecutionSession>> {
        reads
            .read_through(sessions, key, || {
                reads.turn_read(turn_sessions, key, || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(Some(session()))
                })
            })
            .await
    }

    #[tokio::test]
    async fn a_later_phase_starts_from_the_turns_reads() {
        let turn = TurnSlot::default();
        let calls = AtomicU32::new(0);
        let key = (1, Uuid::now_v7());

        let reason = PhaseReads::new().with_turn(turn.clone());
        read_in_turn(&reason, key, &calls).await.unwrap();
        reason.note_event(crate::core::REASON_STARTED);

        // The act phase's setup reuses what the reason phase read.
        let act = PhaseReads::new().with_turn(turn.clone());
        read_in_turn(&act, key, &calls).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(act.with_state(|s| s.from_turn), 1);

        // Once the phase started, reads go to the store again.
        act.note_event(crate::core::ACT_STARTED);
        read_in_turn(&act, key, &calls).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        // A session write drops the turn's copy.
        let next = PhaseReads::new().with_turn(turn);
        next.invalidate_session(key.0, key.1);
        read_in_turn(&next, key, &calls).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
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
    async fn a_read_in_flight_is_awaited_not_repeated() {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let key = (1, Uuid::now_v7());
        let (open, opened) = tokio::sync::oneshot::channel::<()>();
        // The first read stays in flight until the second has started waiting.
        let first = reads.read_through(sessions, key, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            opened.await.ok();
            Ok(Some(session()))
        });
        let second = async {
            tokio::task::yield_now().await;
            read(&reads, key, &calls).await
        };
        let release = async {
            for _ in 0..3 {
                tokio::task::yield_now().await;
            }
            open.send(()).ok();
        };
        let (first, second, ()) = tokio::join!(first, second, release);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(first.unwrap().unwrap().id, second.unwrap().unwrap().id);
    }

    #[tokio::test]
    async fn a_seed_does_not_replace_a_fetched_record() {
        let reads = PhaseReads::new();
        let fetched = session();
        let key = (1, fetched.id.uuid());
        let stored = fetched.clone();
        reads
            .read_through(sessions, key, || async { Ok(Some(stored)) })
            .await
            .unwrap();
        let mut other = fetched.clone();
        other.locale = Some("uk".to_string());
        reads.seed(1, &other, None);
        let calls = AtomicU32::new(0);
        let got = read(&reads, key, &calls).await.unwrap().unwrap();
        assert_eq!(got.locale, fetched.locale);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_phase_start_event_ends_memoizing() {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let key = (1, Uuid::now_v7());
        read(&reads, key, &calls).await.unwrap();
        // Events before the start event leave setup running.
        reads.note_event(crate::core::INPUT_MESSAGE);
        read(&reads, key, &calls).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        reads.note_event(crate::core::REASON_STARTED);
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
        for event in [crate::core::REASON_STARTED, crate::core::ACT_STARTED] {
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

    #[tokio::test]
    async fn simultaneous_definition_and_lifecycle_reads_share_the_in_flight_cell() {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let id = everruns_contracts::typed_id::AgentId::new();
        let key = (1, id.uuid());
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            Ok((
                Ok(Some(AgentDefinition::new(id, "agent", "current"))),
                Some(DependencyBlocker::AgentArchived),
            ))
        };
        let (definition, blocker) = tokio::join!(
            reads.definition(agents, key, fetch),
            reads.blocker(agents, key, fetch)
        );
        assert_eq!(definition.unwrap().unwrap().id, id);
        assert!(matches!(
            blocker.unwrap(),
            Some(DependencyBlocker::AgentArchived)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn pinned_batch_agent_wins_an_in_flight_current_prefetch_without_losing_lifecycle() {
        let reads = PhaseReads::new();
        let id = everruns_contracts::typed_id::AgentId::new();
        let key = (1, id.uuid());
        let current = AgentDefinition::new(id, "agent", "current version");
        let pinned = AgentDefinition::new(id, "agent", "pinned batch version");
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let pending_reads = reads.clone();
        let pending = tokio::spawn(async move {
            pending_reads
                .definition(agents, key, || async {
                    started.send(()).unwrap();
                    released.await.unwrap();
                    Ok((Ok(Some(current)), Some(DependencyBlocker::AgentArchived)))
                })
                .await
        });
        ready.await.unwrap();
        reads.seed(1, &session(), Some(&pinned));
        release.send(()).unwrap();
        assert_eq!(
            pending.await.unwrap().unwrap().unwrap().system_prompt,
            pinned.system_prompt
        );
        let must_not_fetch = || async {
            Err(everruns_contracts::error::AgentLoopError::store(
                "prefetch must be reused",
            ))
        };
        assert!(matches!(
            reads.blocker(agents, key, must_not_fetch).await.unwrap(),
            Some(DependencyBlocker::AgentArchived)
        ));
        assert_eq!(
            reads
                .definition(agents, key, must_not_fetch)
                .await
                .unwrap()
                .unwrap()
                .system_prompt,
            pinned.system_prompt
        );
        // Closing the phase also detaches the pinned overlay, so later reads
        // cannot reuse either its definition or its lifecycle probe.
        reads.note_event(crate::core::REASON_STARTED);
        assert!(reads.definition(agents, key, must_not_fetch).await.is_err());
    }

    #[tokio::test]
    async fn pinned_batch_agent_replaces_an_initialized_current_definition_only() {
        let reads = PhaseReads::new();
        let id = everruns_contracts::typed_id::AgentId::new();
        let key = (1, id.uuid());
        reads
            .definition(agents, key, || async {
                Ok((
                    Ok(Some(AgentDefinition::new(id, "agent", "current"))),
                    Some(DependencyBlocker::AgentDeleted),
                ))
            })
            .await
            .unwrap();
        let pinned = AgentDefinition::new(id, "agent", "pinned");
        reads.seed(1, &session(), Some(&pinned));
        let must_not_fetch =
            || async { Err(everruns_contracts::error::AgentLoopError::store("cached")) };
        assert_eq!(
            reads
                .definition(agents, key, must_not_fetch)
                .await
                .unwrap()
                .unwrap()
                .system_prompt,
            "pinned"
        );
        assert!(matches!(
            reads.blocker(agents, key, must_not_fetch).await.unwrap(),
            Some(DependencyBlocker::AgentDeleted)
        ));
    }

    #[tokio::test]
    async fn a_waiting_definition_retries_an_uncloneable_projection_error() {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let key = (1, Uuid::now_v7());
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            Ok((
                Err(everruns_contracts::error::AgentLoopError::config(
                    "original projection error",
                )),
                Some(DependencyBlocker::AgentArchived),
            ))
        };
        let (first, second) = tokio::join!(
            reads.definition(agents, key, fetch),
            reads.definition(agents, key, fetch)
        );
        for error in [first.unwrap_err(), second.unwrap_err()] {
            assert!(
                matches!(error, everruns_contracts::error::AgentLoopError::Configuration(message) if message == "original projection error")
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(matches!(
            reads.blocker(agents, key, fetch).await.unwrap(),
            Some(DependencyBlocker::AgentArchived)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn turn_context_cells_are_org_session_input_scoped_and_invalidated_on_write() {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let session = session();
        let sid = session.id.uuid();
        let input = Uuid::now_v7();
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(TurnContext {
                agent: None,
                session: session.clone(),
                messages: vec![],
                model: None,
                mcp_tool_definitions: vec![],
            })
        };
        for key in [
            (1, sid, input),
            (1, sid, input),
            (2, sid, input),
            (1, sid, Uuid::now_v7()),
            (1, Uuid::now_v7(), input),
        ] {
            reads.read_through(turn_contexts, key, fetch).await.unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        reads.invalidate_session(1, sid);
        reads
            .read_through(turn_contexts, (1, sid, input), fetch)
            .await
            .unwrap();
        reads
            .read_through(turn_contexts, (2, sid, input), fetch)
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 5);
        reads.note_event(crate::core::ACT_STARTED);
        reads
            .read_through(turn_contexts, (2, sid, input), fetch)
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 6);
    }

    #[tokio::test]
    async fn provider_credentials_never_cross_org_provider_or_session_keys() {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let first = Uuid::now_v7();
        let second = Uuid::now_v7();
        let keys = [
            (1, "provider-a".to_owned(), Some(first)),
            (1, "provider-a".to_owned(), Some(first)),
            (1, "provider-a".to_owned(), Some(second)),
            (1, "provider-a".to_owned(), None),
            (2, "provider-a".to_owned(), Some(first)),
            (1, "provider-b".to_owned(), Some(first)),
        ];
        for key in keys {
            let expected = format!("test-credential-{key:?}");
            let mut config = ProviderConfig::for_provider(
                key.1.clone(),
                everruns_contracts::provider::DriverId::OpenAI,
            );
            config.api_key = Some(expected.clone());
            let got = reads
                .read_through(providers, key, || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(Some(config))
                })
                .await
                .unwrap()
                .unwrap();
            assert_eq!(got.api_key.as_deref(), Some(expected.as_str()));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn provider_credentials_retry_errors_expire_and_are_detached_at_phase_start() {
        let reads = PhaseReads::new();
        let key = (1, "provider".to_owned(), Some(Uuid::now_v7()));
        let failed: Result<Option<ProviderConfig>> = reads
            .read_through(providers, key.clone(), || async {
                Err(everruns_contracts::error::AgentLoopError::store(
                    "credential read refused",
                ))
            })
            .await;
        assert!(failed.is_err());
        let calls = AtomicU32::new(0);
        let fetch = || async {
            let version = calls.fetch_add(1, Ordering::SeqCst);
            let mut config = ProviderConfig::new(everruns_contracts::provider::DriverId::OpenAI);
            config.api_key = Some(format!("test-credential-{version}"));
            Ok(Some(config))
        };
        let first = reads
            .read_through(providers, key.clone(), fetch)
            .await
            .unwrap()
            .unwrap();
        let cached = reads
            .read_through(providers, key.clone(), fetch)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cached.api_key, first.api_key);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        reads.with_state(|state| {
            state.providers.get_mut(&key).unwrap().at = Instant::now() - FRESH_FOR
        });
        let fresh = reads
            .read_through(providers, key.clone(), fetch)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(fresh.api_key, first.api_key);
        reads.note_event(crate::core::REASON_STARTED);
        assert!(reads.with_state(|state| state.providers.is_empty()));
        let after = reads
            .read_through(providers, key.clone(), fetch)
            .await
            .unwrap()
            .unwrap();
        let again = reads
            .read_through(providers, key, fetch)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(after.api_key, again.api_key);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert!(reads.with_state(|state| state.providers.is_empty()));
    }
}
