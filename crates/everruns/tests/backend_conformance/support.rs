//! Running a scenario on every backend and comparing what each observed.

use std::future::Future;
use std::time::Duration;

use everruns::{Agent, Engine, Session, Turn, TurnStopReason, durable};

/// A backend the suite runs every scenario on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    InProcess,
    DurableMemory,
}

pub const BACKENDS: [BackendKind; 2] = [BackendKind::InProcess, BackendKind::DurableMemory];

/// How long one scenario may take on one backend.
const SCENARIO_TIMEOUT: Duration = Duration::from_secs(20);

pub fn engine(kind: BackendKind) -> Engine {
    match kind {
        BackendKind::InProcess => Engine::new(),
        BackendKind::DurableMemory => Engine::builder()
            .backend(durable::Backend::memory().workers(2))
            .build(),
    }
}

/// The comparable shape of a finished turn; its id differs per run.
#[derive(Debug, PartialEq)]
pub struct TurnShape {
    pub response: String,
    pub success: bool,
    pub stop_reason: TurnStopReason,
    pub iterations: usize,
    pub tool_calls: usize,
}

impl From<Turn> for TurnShape {
    fn from(turn: Turn) -> Self {
        Self {
            response: turn.response,
            success: turn.success,
            stop_reason: turn.stop_reason,
            iterations: turn.iterations,
            tool_calls: turn.tool_calls,
        }
    }
}

/// What a scenario reports from one backend.
#[derive(Debug, Default)]
pub struct Observed {
    pub turns: Vec<Turn>,
    /// Anything else the scenario compares, in the order it saw it.
    pub notes: Vec<String>,
}

impl Observed {
    pub fn turns(turns: Vec<Turn>) -> Self {
        Self {
            turns,
            notes: Vec::new(),
        }
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

/// What a scenario observed on one backend, compared across backends.
#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub turns: Vec<TurnShape>,
    pub notes: Vec<String>,
    /// The session's persisted event types, in order.
    pub event_types: Vec<String>,
}

/// Run `scenario` on a fresh session on each backend and require the same
/// [`Outcome`] from all of them. `setup` builds the agent, plus whatever the
/// scenario uses to observe it, once per backend.
pub async fn run_on<P, S, F, Fut>(setup: S, scenario: F) -> Outcome
where
    S: Fn() -> (Agent, P),
    F: Fn(Engine, Session, P) -> Fut,
    Fut: Future<Output = Observed>,
{
    let mut outcomes = Vec::new();
    for kind in BACKENDS {
        let engine = engine(kind);
        let (agent, probe) = setup();
        let session = engine.create(agent);
        let session_id = session.session_id();
        let observed =
            tokio::time::timeout(SCENARIO_TIMEOUT, scenario(engine.clone(), session, probe))
                .await
                .unwrap_or_else(|_| panic!("{kind:?}: the scenario finishes"));
        // The scenario may have let its session go; reopen it to read history.
        let session = engine
            .resume(session_id)
            .await
            .unwrap_or_else(|error| panic!("{kind:?}: the session reopens: {error}"));
        let event_types = session
            .events_after(0)
            .await
            .expect("history reads")
            .iter()
            .map(|event| event.event_type().to_string())
            .collect();
        outcomes.push((
            kind,
            Outcome {
                turns: observed.turns.into_iter().map(TurnShape::from).collect(),
                notes: observed.notes,
                event_types,
            },
        ));
    }
    let (_, in_process) = outcomes.remove(0);
    for (kind, outcome) in outcomes {
        assert_eq!(outcome, in_process, "{kind:?} diverges from in process");
    }
    in_process
}

pub fn agent_with(model: everruns::Model) -> Agent {
    Agent::builder()
        .instructions("You are concise.")
        .model(model)
        .build()
        .expect("valid agent")
}

pub fn has_event(outcome: &Outcome, event_type: &str) -> bool {
    outcome.event_types.iter().any(|kind| kind == event_type)
}
