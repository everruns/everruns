//! Running a scenario and collecting what it observed.

use std::future::Future;
use std::time::Duration;

use everruns::{Agent, Engine, Session, Turn, TurnStopReason};

/// A turn backend the entry-point scenarios run on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    /// `InProcessBackend`: the reference, no lease.
    InProcess,
    /// `ActorRunner`: the same turns under the session's lease, what an
    /// engine runs its sessions on.
    Actor,
}

/// The backends the entry-point scenarios compare, the reference first.
pub fn backends() -> Vec<BackendKind> {
    vec![BackendKind::InProcess, BackendKind::Actor]
}

/// How long one scenario may take.
const SCENARIO_TIMEOUT: Duration = Duration::from_secs(20);

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

/// What a scenario reports.
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

/// What a scenario observed.
#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub turns: Vec<TurnShape>,
    pub notes: Vec<String>,
    /// The session's persisted event types, in order.
    pub event_types: Vec<String>,
}

/// Run `scenario` on a fresh session of an engine and report what it
/// observed. `setup` builds the agent, plus whatever the scenario uses to
/// observe it.
pub async fn run_on<P, S, F, Fut>(setup: S, scenario: F) -> Outcome
where
    S: Fn() -> (Agent, P),
    F: Fn(Engine, Session, P) -> Fut,
    Fut: Future<Output = Observed>,
{
    let engine = Engine::new();
    let (agent, probe) = setup();
    let session = engine.create(agent);
    let session_id = session.session_id();
    let observed = tokio::time::timeout(SCENARIO_TIMEOUT, scenario(engine.clone(), session, probe))
        .await
        .expect("the scenario finishes");
    // The scenario may have let its session go; reopen it to read history.
    let session = engine
        .resume(session_id)
        .await
        .unwrap_or_else(|error| panic!("the session reopens: {error}"));
    let event_types = session
        .events_after(0)
        .await
        .expect("history reads")
        .iter()
        .map(|event| event.event_type().to_string())
        .collect();
    Outcome {
        turns: observed.turns.into_iter().map(TurnShape::from).collect(),
        notes: observed.notes,
        event_types,
    }
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
