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

use everruns_contracts::error::Result;
use everruns_core::ExecutionSession;
use everruns_platform::{Agent, Harness};
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::worker_adapters::{TurnContext, WorkerAdapters};

/// How long a memoized read stays usable. Covers setup (under a second even
/// with a remote database).
const FRESH_FOR: Duration = Duration::from_secs(2);

type Key = (i64, Uuid);
/// Org, session, and input message.
type TurnKey = (i64, Uuid, Uuid);

struct Entry<T> {
    at: Instant,
    cell: Arc<OnceCell<T>>,
}

#[derive(Default)]
struct State {
    agents: HashMap<Key, Entry<Option<Agent>>>,
    harnesses: HashMap<Key, Entry<Option<Harness>>>,
    sessions: HashMap<Key, Entry<Option<ExecutionSession>>>,
    turn_contexts: HashMap<TurnKey, Entry<TurnContext>>,
    fetched: u32,
    saved: u32,
    /// The phase started: stop memoizing.
    setup_done: bool,
}

type Map<K, T> = fn(&mut State) -> &mut HashMap<K, Entry<T>>;

fn agents(s: &mut State) -> &mut HashMap<Key, Entry<Option<Agent>>> {
    &mut s.agents
}

fn harnesses(s: &mut State) -> &mut HashMap<Key, Entry<Option<Harness>>> {
    &mut s.harnesses
}

fn sessions(s: &mut State) -> &mut HashMap<Key, Entry<Option<ExecutionSession>>> {
    &mut s.sessions
}

fn turn_contexts(s: &mut State) -> &mut HashMap<TurnKey, Entry<TurnContext>> {
    &mut s.turn_contexts
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
    pub fn reason(input: &everruns_engine::ReasonInput) -> Option<Self> {
        let message_id = input.context.input_message_id.uuid();
        Some(Self {
            org_id: input.org_id,
            session_id: input.context.session_id.uuid(),
            harness_id: input.harness_id.uuid(),
            agent_id: input.agent_id.map(|id| id.uuid()),
            input_message_id: (!message_id.is_nil()).then_some(message_id),
        })
    }

    pub fn act(input: &everruns_engine::ActInput) -> Option<Self> {
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
    ) -> Result<Option<Agent>> {
        self.read_through(agents, (org_id, id), || adapters.get_agent(org_id, id))
            .await
    }

    pub async fn harness<A: WorkerAdapters>(
        &self,
        adapters: &A,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<Harness>> {
        self.read_through(harnesses, (org_id, id), || adapters.get_harness(org_id, id))
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
            || adapters.load_turn_context_for_execution(org_id, session_id, input_message_id),
        )
        .await
    }

    /// Keep records another read already returned (the batched turn context
    /// carries the session and agent), so later lookups reuse them.
    pub fn seed(&self, org_id: i64, session: &ExecutionSession, agent: Option<&Agent>) {
        let session_id = session.id.uuid();
        self.store(sessions, (org_id, session_id), Some(session.clone()));
        if let Some(agent) = agent {
            let key = (org_id, agent.public_id.uuid());
            self.store(agents, key, Some(agent.clone()));
        }
    }

    /// Forget the session after a write, so the next read sees it. The turn
    /// context carries the session too.
    pub fn invalidate_session(&self, org_id: i64, id: Uuid) {
        self.with_state(|s| {
            s.sessions.remove(&(org_id, id));
            s.turn_contexts
                .retain(|&(org, session, _), _| (org, session) != (org_id, id));
        });
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
}
