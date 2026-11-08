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
// Decision: one memo per phase, re-armed at each phase boundary of a leased
// turn (`begin_phase`), so nothing is shared across phases or turns except
// through the turn's own reads (`turn_reads`). It serves setup only:
// once the phase's start event goes out it empties and stops memoizing, so the
// model call, tools, and anything after them read fresh state. The session is
// the exception: in a leased turn, reads after setup get the session the turn
// holds (`session_with`). Entries also
// expire after `FRESH_FOR`, for hosts that never emit a start event (activity
// scheduling). Session writes made through the host drop the session entry.
// Errors are never memoized.
//
// Decision: the worker knows every id these reads need when the phase
// begins, so `prefetch` starts them all at once instead of letting setup issue
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
    /// When the current phase's setup began, once a host that runs a whole
    /// turn began a later phase ([`PhaseReads::begin_phase`]).
    phase_began: Option<Instant>,
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

/// The ids a phase's setup reads, known when the turn's phase begins.
pub struct PhaseIds {
    pub org_id: i64,
    pub session_id: Uuid,
    pub harness_id: Uuid,
    pub agent_id: Option<Uuid>,
    /// Set for a reason phase, which also loads the turn context.
    pub input_message_id: Option<Uuid>,
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
        self.session_with((org_id, id), || adapters.get_session(org_id, id))
            .await
    }

    /// A session read: the phase memo and the turn's reads during setup, and
    /// after it the session the turn holds.
    ///
    /// Decision: once a phase has started, its session reads (act scheduling,
    /// pause hints, the session capability, ...) are answered from the
    /// session the turn already holds. The worker holding the lease is the
    /// session's owner for the turn and the turn's configuration is pinned
    /// anyway; before this, each of those reads was a control-plane round
    /// trip and a session-row query (about 7 per turn). The worker's own
    /// writes keep the turn's copy current (`session_status_written`,
    /// `invalidate_session`), so only a read after such a write, or of a
    /// session the turn never read, reaches the control plane. Usage totals
    /// in it are as of the turn's first read.
    async fn session_with<F, Fut>(&self, key: Key, fetch: F) -> Result<Option<ExecutionSession>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Option<ExecutionSession>>>,
    {
        if let Some(turn) = self.turn_after_setup() {
            if let Some(session) = turn.get(turn_sessions, &key) {
                return Ok(session);
            }
            let session = fetch().await?;
            turn.put(turn_sessions, key, session.clone());
            return Ok(session);
        }
        self.read_through(sessions, key, || self.turn_read(turn_sessions, key, fetch))
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

    /// The worker wrote the session's status. This phase's memo forgets the
    /// session; the turn's kept reads take the new status and keep the rest,
    /// since a status write changes nothing else they hold.
    pub fn session_status_written(
        &self,
        org_id: i64,
        id: Uuid,
        status: everruns_contracts::runtime::session::SessionExecutionState,
    ) {
        if let Some(turn) = &self.turn {
            turn.set_session_status(org_id, id, status);
        }
        self.with_state(|s| {
            s.sessions.remove(&(org_id, id));
            s.turn_contexts
                .retain(|&(org, session, _), _| (org, session) != (org_id, id));
        });
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

    /// A new phase of the same turn begins on this host (a worker running a
    /// whole turn on a session lease keeps one host for the turn): memoize its
    /// setup afresh and time it from now. The turn's shared reads stay.
    pub fn begin_phase(&self) {
        self.with_state(|s| {
            *s = State {
                phase_began: Some(Instant::now()),
                ..State::default()
            }
        });
    }

    /// Log the phase's setup time when its start event goes out. Once per memo.
    pub fn note_event(&self, event_type: &str) {
        if event_type != crate::core::REASON_STARTED && event_type != crate::core::ACT_STARTED {
            return;
        }
        let (fetched, saved, from_turn, first, began) = self.with_state(|s| {
            let counts = (
                s.fetched.saturating_sub(s.from_turn),
                s.saved,
                s.from_turn,
                !s.setup_done,
                s.phase_began.unwrap_or(self.started),
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
                setup_ms = began.elapsed().as_millis() as u64,
                reads_fetched = fetched,
                reads_saved = saved,
                reads_from_turn = from_turn,
                "phase setup"
            );
        }
    }

    /// The turn's slot, once the phase has started.
    fn turn_after_setup(&self) -> Option<TurnSlot> {
        let turn = self.turn.clone()?;
        self.with_state(|s| s.setup_done).then_some(turn)
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
#[path = "phase_reads_tests.rs"]
mod tests;
