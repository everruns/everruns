//! The runtime: two loops that do not block each other.
//!
//! The worker's loop is the Framework's — reason, act, observe — running inside
//! a session. The supervisory loop runs beside it on the session's canonical
//! event stream, so evidence reaches the classifier while the worker is still
//! working. Nothing has to stop for the factory to think.
//!
//! Between assessments the loop debounces: routine output respects a floor, and
//! lifecycle boundaries — a tool finishing, a turn ending — bypass it, because
//! those are the moments where an early intervention is still worth something.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use everruns::{Agent, Engine, Session, SessionEventKind, TurnHandle, TurnStopReason};
use tokio::sync::mpsc;

use crate::agent;
use crate::foreman::{Assessment, Foreman};
use crate::observation::{
    self, Evidence, VerificationResult, WorkerKind, WorkerRecord, WorkerStatus,
};
use crate::policy::{self, Action, Config, Floor, Intervention};

/// Where a run ended up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Still going.
    Running,
    /// The policy declared the job complete.
    Finished,
    /// The policy handed the job to a person.
    Escalated,
    /// The run exceeded its overall budget.
    TimedOut,
}

impl Status {
    /// The status as it is written in the timeline.
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Finished => "finished",
            Self::Escalated => "escalated",
            Self::TimedOut => "timed out",
        }
    }
}

/// What a run leaves behind.
pub struct Outcome {
    /// Where it ended up.
    pub status: Status,
    /// Supervisory iterations spent.
    pub iterations: usize,
    /// Every worker, oldest first.
    pub workers: Vec<WorkerRecord>,
    /// What verification reported.
    pub verification: Vec<VerificationResult>,
    /// The last assessment made.
    pub last_assessment: Option<Assessment>,
    /// The decision that ended the run.
    pub last_intervention: Option<Intervention>,
    /// Failures recorded along the way.
    pub failures: Vec<String>,
    /// Wall-clock time.
    pub elapsed: Duration,
}

/// Everything the factory tells someone watching it.
///
/// Presentation only: a watcher never influences a decision, so a silent run
/// and a rendered one take exactly the same path.
#[allow(unused_variables)]
pub trait Watcher: Send + Sync {
    /// A worker was started on a mission.
    fn worker_started(&self, worker: &WorkerRecord) {}
    /// A worker emitted assistant text.
    fn worker_text(&self, worker_id: &str, delta: &str) {}
    /// A worker started a tool call, with the script it passed.
    fn worker_tool(&self, worker_id: &str, tool: &str, script: &str) {}
    /// A worker's turn ended.
    fn worker_finished(&self, worker: &WorkerRecord) {}
    /// The supervisor took a reading.
    fn assessed(&self, iteration: usize, assessment: &Assessment) {}
    /// The supervisor could not take a reading.
    fn assessment_failed(&self, iteration: usize, error: &str) {}
    /// The policy decided.
    fn intervened(&self, intervention: &Intervention) {}
}

/// A watcher that renders nothing.
pub struct Silent;
impl Watcher for Silent {}

enum Signal {
    /// Something happened on the floor that may be worth assessing.
    Activity {
        /// Lifecycle boundaries bypass the debounce floor; output does not.
        important: bool,
    },
    /// A worker's turn ended.
    Finished {
        worker_id: String,
        kind: WorkerKind,
        success: bool,
        summary: String,
    },
}

/// A coding worker supervised by a classifier.
pub struct Factory {
    config: Config,
    foreman: Foreman,
    worker_agent: Agent,
    verifier_agent: Agent,
    engine: Engine,
    workspace: PathBuf,
    job: String,
    run_id: String,
    watcher: Arc<dyn Watcher>,
}

impl Factory {
    /// Assemble a factory over `workspace`.
    pub fn new(
        job: impl Into<String>,
        workspace: impl Into<PathBuf>,
        worker_agent: Agent,
        verifier_agent: Agent,
        foreman: Foreman,
        config: Config,
    ) -> Self {
        Self {
            config,
            foreman,
            worker_agent,
            verifier_agent,
            engine: Engine::new(),
            workspace: workspace.into(),
            job: job.into(),
            run_id: run_id(),
            watcher: Arc::new(Silent),
        }
    }

    /// Render the run through `watcher`.
    pub fn watched_by(mut self, watcher: Arc<dyn Watcher>) -> Self {
        self.watcher = watcher;
        self
    }

    /// The model doing the supervising, for display.
    pub fn foreman_model(&self) -> &str {
        self.foreman.model()
    }

    /// The run's id.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Start one coding worker and supervise until the policy stops.
    pub async fn run(&self) -> Outcome {
        let started = Instant::now();
        let mut state = State::new();
        let (signals, mut inbox) = mpsc::unbounded_channel();

        // The first worker is started unconditionally: there is nothing to
        // assess about a floor where no work has begun.
        self.start_worker(&mut state, WorkerKind::Coding, &signals)
            .await;

        let supervision = self.supervise(&mut state, &mut inbox, &signals);
        if tokio::time::timeout(self.config.overall_timeout, supervision)
            .await
            .is_err()
        {
            state.status = Status::TimedOut;
            state
                .failures
                .push("overall run budget exceeded".to_owned());
            self.stop_active(&mut state).await;
        }

        Outcome {
            status: state.status,
            iterations: state.iteration,
            workers: lock(&state.workers).clone(),
            verification: state.verification.clone(),
            last_assessment: state.last_assessment,
            last_intervention: state.last_intervention.clone(),
            failures: state.failures.clone(),
            elapsed: started.elapsed(),
        }
    }

    /// The supervisory loop. Ported from Foreman's watch loop, debounce and all.
    async fn supervise(
        &self,
        state: &mut State,
        inbox: &mut mpsc::UnboundedReceiver<Signal>,
        signals: &mpsc::UnboundedSender<Signal>,
    ) {
        let mut last_assessment: Option<Instant> = None;
        let mut dirty = false;
        // The floor has just been started, which is itself worth a reading.
        let mut force = true;

        while state.status == Status::Running {
            let since = last_assessment.map(|at| at.elapsed());
            let periodic_remaining = remaining(self.config.periodic_assessment, since);
            let floor_remaining = remaining(self.config.min_assessment_interval, since);
            let wait = if dirty {
                periodic_remaining.min(floor_remaining)
            } else {
                periodic_remaining
            };

            match tokio::time::timeout(wait.max(Duration::from_millis(1)), inbox.recv()).await {
                Ok(Some(signal)) => {
                    force |= self.absorb(state, signal);
                    dirty = true;
                    // Collapse a burst of streaming output into one reading
                    // rather than one reading per chunk.
                    while let Ok(signal) = inbox.try_recv() {
                        force |= self.absorb(state, signal);
                    }
                }
                // Quiet work still gets looked at.
                _ => dirty = true,
            }

            // Re-read the clock: `since` was measured before the wait above, and
            // deciding eligibility on a stale reading costs a whole extra trip
            // around the loop before the floor is seen to have passed.
            let interval_elapsed = last_assessment
                .is_none_or(|at| at.elapsed() >= self.config.min_assessment_interval);
            if !(force || (dirty && interval_elapsed)) {
                continue;
            }

            let intervention = self.assess(state).await;
            last_assessment = Some(Instant::now());
            dirty = false;
            force = false;
            self.apply(state, intervention, signals).await;
        }
    }

    /// Fold one signal into the run's state, reporting whether it should
    /// bypass the debounce floor.
    fn absorb(&self, state: &mut State, signal: Signal) -> bool {
        match signal {
            Signal::Activity { important } => important,
            Signal::Finished {
                worker_id,
                kind,
                success,
                summary,
            } => {
                if kind == WorkerKind::Verifier {
                    state.verification_completed = true;
                    state.verification.push(VerificationResult {
                        worker_id: worker_id.clone(),
                        passed: success,
                        summary: observation::tail(&summary, self.config.output_limit),
                    });
                }
                if !success {
                    // Name what went wrong: "did not succeed" is not evidence
                    // a supervisor, or a person reading the run, can act on.
                    let detail = find(&mut lock(&state.workers), &worker_id)
                        .and_then(|worker| worker.stop_reason.clone())
                        .unwrap_or_else(|| "turn did not succeed".to_owned());
                    state.failures.push(format!("{worker_id}: {detail}"));
                }
                if let Some(worker) = find(&mut lock(&state.workers), &worker_id) {
                    self.watcher.worker_finished(worker);
                }
                state.handles.remove(&worker_id);
                true
            }
        }
    }

    /// One supervisory iteration: observe, assess, decide.
    async fn assess(&self, state: &mut State) -> Intervention {
        state.iteration += 1;
        let git = observation::git_evidence(&self.workspace, &self.config).await;
        let workers = lock(&state.workers).clone();
        let events = lock(&state.events).clone();
        let observation = observation::build(Evidence {
            job: &self.job,
            run_id: &self.run_id,
            status: state.status.label(),
            iteration: state.iteration,
            workers: &workers,
            verification: &state.verification,
            events: &events,
            previous_assessment: state.last_assessment,
            previous_intervention: state.last_intervention.clone(),
            failures: &state.failures,
            elapsed: state.started.elapsed(),
            git,
            config: &self.config,
        });

        let assessment = match self.foreman.assess(&observation).await {
            Ok(assessment) => assessment,
            Err(error) => {
                // A supervisor that cannot see is not allowed to keep deciding.
                let error = error.to_string();
                self.watcher.assessment_failed(state.iteration, &error);
                state.failures.push(error.clone());
                return Intervention {
                    action: Action::Escalate,
                    reason: format!("semantic assessment unavailable: {error}"),
                    iteration: state.iteration,
                    worker: None,
                };
            }
        };

        self.watcher.assessed(state.iteration, &assessment);
        state.last_assessment = Some(assessment);
        policy::decide(&state.floor(), &assessment, &self.config)
    }

    /// Carry out a decision. This is the only place the supervisor touches a
    /// session, and the vocabulary is closed.
    async fn apply(
        &self,
        state: &mut State,
        intervention: Intervention,
        signals: &mpsc::UnboundedSender<Signal>,
    ) {
        self.watcher.intervened(&intervention);
        let action = intervention.action;
        state.last_intervention = Some(intervention.clone());

        match action {
            Action::Continue => {}
            Action::StartWorker => self.start_worker(state, WorkerKind::Coding, signals).await,
            Action::StartVerifier => {
                state.verification_started = true;
                self.start_worker(state, WorkerKind::Verifier, signals)
                    .await;
            }
            Action::StopWorker => {
                let worker = intervention
                    .worker
                    .or_else(|| active_id(&lock(&state.workers)));
                if let Some(worker) = worker {
                    self.stop(state, &worker, &intervention.reason).await;
                }
            }
            Action::RetryWorker => {
                state.retries += 1;
                self.start_worker(state, WorkerKind::Coding, signals).await;
            }
            Action::Finish => state.status = Status::Finished,
            Action::Escalate => {
                self.stop_active(state).await;
                state.status = Status::Escalated;
            }
        }
    }

    /// Start a session on the workspace and let it run while we watch it.
    async fn start_worker(
        &self,
        state: &mut State,
        kind: WorkerKind,
        signals: &mpsc::UnboundedSender<Signal>,
    ) {
        let number = lock(&state.workers).len() + 1;
        let id = format!("worker-{number}");
        let record = WorkerRecord::new(&id, kind, state.retries + 1, self.config.output_limit);
        self.watcher.worker_started(&record);
        lock(&state.workers).push(record);

        let (agent, mission) = match kind {
            WorkerKind::Coding => (self.worker_agent.clone(), agent::coding_mission(&self.job)),
            WorkerKind::Verifier => (
                self.verifier_agent.clone(),
                agent::verification_mission(&self.job),
            ),
        };

        let session = self.engine.create(agent);
        // Subscribe before sending: an event emitted between the two would
        // otherwise be evidence the supervisor never sees.
        let stream = session.events();
        let pending = match session.send(mission.as_str()).await {
            Ok(pending) => pending,
            Err(error) => {
                let message = format!("{id}: could not start worker: {error}");
                state.failures.push(message);
                finish_record(&mut lock(&state.workers), &id, WorkerStatus::Failed, None);
                let _ = signals.send(Signal::Finished {
                    worker_id: id,
                    kind,
                    success: false,
                    summary: String::new(),
                });
                return;
            }
        };

        state.handles.insert(id.clone(), pending.turn());
        note(&state.events, format!("{id} started ({kind:?})"));
        tokio::spawn(pump(
            Pump {
                session,
                id,
                kind,
                workers: Arc::clone(&state.workers),
                events: Arc::clone(&state.events),
                watcher: Arc::clone(&self.watcher),
                signals: signals.clone(),
            },
            stream,
            pending,
        ));
    }

    async fn stop(&self, state: &mut State, worker_id: &str, reason: &str) {
        if let Some(handle) = state.handles.get(worker_id) {
            // Cooperative: the turn stops at its next boundary and resolves as
            // cancelled, so the worker's own session stays consistent.
            let _ = handle.cancel().await;
        }
        if let Some(worker) = find(&mut lock(&state.workers), worker_id) {
            worker.stop_reason = Some(reason.to_owned());
        }
        note(
            &state.events,
            format!("{worker_id} stop requested: {reason}"),
        );
    }

    async fn stop_active(&self, state: &mut State) {
        let active = active_ids(&lock(&state.workers));
        for worker_id in active {
            self.stop(state, &worker_id, "factory stopped").await;
        }
    }
}

struct State {
    started: Instant,
    status: Status,
    iteration: usize,
    retries: usize,
    verification_started: bool,
    verification_completed: bool,
    verification: Vec<VerificationResult>,
    failures: Vec<String>,
    last_assessment: Option<Assessment>,
    last_intervention: Option<Intervention>,
    workers: Arc<Mutex<Vec<WorkerRecord>>>,
    events: Arc<Mutex<VecDeque<String>>>,
    handles: HashMap<String, TurnHandle>,
}

impl State {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            status: Status::Running,
            iteration: 0,
            retries: 0,
            verification_started: false,
            verification_completed: false,
            verification: Vec::new(),
            failures: Vec::new(),
            last_assessment: None,
            last_intervention: None,
            workers: Arc::new(Mutex::new(Vec::new())),
            events: Arc::new(Mutex::new(VecDeque::new())),
            handles: HashMap::new(),
        }
    }

    /// The lifecycle facts the policy is allowed to see.
    fn floor(&self) -> Floor {
        let workers = lock(&self.workers);
        Floor {
            iteration: self.iteration,
            active_worker: active_id(&workers),
            workers_started: workers.len(),
            retries: self.retries,
            verification_started: self.verification_started,
            verification_completed: self.verification_completed,
            last_action: self.last_intervention.as_ref().map(|i| i.action),
        }
    }
}

struct Pump {
    session: Session,
    id: String,
    kind: WorkerKind,
    workers: Arc<Mutex<Vec<WorkerRecord>>>,
    events: Arc<Mutex<VecDeque<String>>>,
    watcher: Arc<dyn Watcher>,
    signals: mpsc::UnboundedSender<Signal>,
}

/// Drain one worker's canonical events into the evidence the supervisor reads.
///
/// This is the whole coupling between the two loops: the worker never waits for
/// it, and it never speaks back into the session.
async fn pump(pump: Pump, mut stream: everruns::EventStream, pending: everruns::SentMessage) {
    let turn_id = pending.turn_id.clone();
    loop {
        let event = match stream.recv().await {
            Ok(Some(event)) => event,
            // A lagging observer misses events; it does not stop observing.
            Err(_) => continue,
            Ok(None) => break,
        };
        if event.turn_id.as_deref().is_some_and(|id| id != turn_id) {
            continue;
        }
        let terminal = event.kind.is_terminal();
        let important = match &event.kind {
            SessionEventKind::TextDelta { delta } => {
                with(&pump.workers, &pump.id, |worker| worker.output.push(delta));
                pump.watcher.worker_text(&pump.id, delta);
                false
            }
            SessionEventKind::ReasonStarted => {
                with(&pump.workers, &pump.id, |worker| worker.iterations += 1);
                false
            }
            SessionEventKind::ToolStarted { tool_name, .. } => {
                // The reviewed surface names the tool; the script it was
                // called with lives in the canonical payload, and that is what
                // says whether a worker is repeating itself.
                let script = script_of(&event);
                with(&pump.workers, &pump.id, |worker| {
                    worker.tool_calls += 1;
                    worker.last_tool = Some(format!("{tool_name}: {}", first_line(&script)));
                    // What the worker ran belongs in the output tail beside what
                    // came back: a diff shows the file a worker wrote, and this
                    // shows the one it only meant to.
                    worker.output.push(&format!("\n$ {script}\n"));
                });
                pump.watcher.worker_tool(&pump.id, tool_name, &script);
                note(
                    &pump.events,
                    format!("{} tool {tool_name} {}", pump.id, first_line(&script)),
                );
                false
            }
            SessionEventKind::ToolOutputDelta { delta, .. } => {
                with(&pump.workers, &pump.id, |worker| worker.output.push(delta));
                false
            }
            // A finished tool call changes the repository, so it makes the run
            // dirty — but it does not bypass the debounce floor. Foreman's
            // forcing events are worker lifecycle boundaries, which happen a
            // handful of times per run; a tool call happens constantly, and
            // forcing on one spends the whole supervisory iteration budget on a
            // chatty worker long before it finishes.
            SessionEventKind::ToolCompleted {
                tool_name, success, ..
            } => {
                note(
                    &pump.events,
                    format!(
                        "{} tool {tool_name} {}",
                        pump.id,
                        if *success { "ok" } else { "failed" }
                    ),
                );
                false
            }
            _ => false,
        };
        if terminal {
            // Deliberately silent: the run's view of a finished worker is not
            // complete until its turn has resolved and any verification result
            // has been recorded. `Signal::Finished`, below, is that boundary.
            break;
        }
        let _ = pump.signals.send(Signal::Activity { important });
    }

    let turn = pending.wait().await;
    let (status, success, summary, stop_reason) = match turn {
        Ok(turn) => {
            let status = match turn.stop_reason {
                TurnStopReason::Cancelled => WorkerStatus::Stopped,
                _ if turn.success => WorkerStatus::Completed,
                _ => WorkerStatus::Failed,
            };
            let reason = turn
                .error
                .clone()
                .unwrap_or_else(|| format!("{:?}", turn.stop_reason));
            (status, turn.success, turn.response, Some(reason))
        }
        Err(error) => (
            WorkerStatus::Failed,
            false,
            String::new(),
            Some(error.to_string()),
        ),
    };
    with(&pump.workers, &pump.id, |worker| {
        worker.output.push(&summary);
    });
    finish_record(&mut lock(&pump.workers), &pump.id, status, stop_reason);
    note(
        &pump.events,
        format!("{} turn {}", pump.id, status_label(status)),
    );
    let _ = pump.signals.send(Signal::Finished {
        worker_id: pump.id.clone(),
        kind: pump.kind,
        success,
        summary,
    });
    // The session is dropped here, after its turn has resolved.
    drop(pump.session);
}

/// The script a shell tool call was made with, when there is one.
fn script_of(event: &everruns::SessionEvent) -> String {
    let arguments = &event.canonical_json()["data"]["tool_call"]["arguments"];
    arguments["commands"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| arguments.to_string())
}

/// The first line worth showing, clipped.
fn first_line(script: &str) -> String {
    let line = script
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if line.chars().count() <= 80 {
        return line.to_owned();
    }
    format!("{}…", line.chars().take(79).collect::<String>())
}

fn status_label(status: WorkerStatus) -> &'static str {
    match status {
        WorkerStatus::Running => "running",
        WorkerStatus::Completed => "completed",
        WorkerStatus::Failed => "failed",
        WorkerStatus::Stopped => "stopped",
    }
}

fn finish_record(
    workers: &mut [WorkerRecord],
    id: &str,
    status: WorkerStatus,
    stop_reason: Option<String>,
) {
    if let Some(worker) = find(workers, id) {
        worker.status = status;
        worker.finished = Some(Instant::now());
        if worker.stop_reason.is_none() {
            worker.stop_reason = stop_reason;
        }
    }
}

fn with(workers: &Mutex<Vec<WorkerRecord>>, id: &str, edit: impl FnOnce(&mut WorkerRecord)) {
    if let Some(worker) = find(&mut lock(workers), id) {
        edit(worker);
    }
}

fn find<'a>(workers: &'a mut [WorkerRecord], id: &str) -> Option<&'a mut WorkerRecord> {
    workers.iter_mut().find(|worker| worker.id == id)
}

fn active_id(workers: &[WorkerRecord]) -> Option<String> {
    workers
        .iter()
        .find(|worker| worker.is_active())
        .map(|worker| worker.id.clone())
}

fn active_ids(workers: &[WorkerRecord]) -> Vec<String> {
    workers
        .iter()
        .filter(|worker| worker.is_active())
        .map(|worker| worker.id.clone())
        .collect()
}

fn note(events: &Mutex<VecDeque<String>>, line: String) {
    let mut events = lock(events);
    // The window is bounded here rather than at read time, so a long run does
    // not accumulate a timeline nobody will ever look at.
    if events.len() >= 200 {
        events.pop_front();
    }
    events.push_back(line);
}

/// Lock, treating a poisoned mutex as ordinary data.
///
/// The only writers are the pumps and the supervisor, and neither holds the
/// lock across an await: a poisoned lock means a panic elsewhere, not evidence
/// worth discarding.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn remaining(budget: Duration, since: Option<Duration>) -> Duration {
    match since {
        Some(since) => budget.saturating_sub(since),
        None => Duration::ZERO,
    }
}

/// A short, sortable run id from the wall clock.
fn run_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!("{:012x}", nanos as u64 & 0xffff_ffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    use everruns::{LlmSimConfig, Model};

    use crate::foreman::Assessment;

    /// A worker that keeps talking until somebody stops it.
    fn endless_worker() -> Model {
        Model::simulated_with_config(
            LlmSimConfig::fixed("Still working on it.")
                .with_response_delay(Duration::from_secs(30)),
        )
    }

    #[tokio::test]
    async fn a_stuck_worker_is_stopped_retried_once_and_then_escalated() {
        let workspace = tempfile::tempdir().unwrap();
        // One reading, held for the whole run: the worker is stuck. The policy
        // still has to walk stop → retry → escalate rather than repeat itself.
        let foreman = Foreman::scripted([Assessment {
            worker_stuck: 0.95,
            meaningful_progress: 0.05,
            ..Assessment::default()
        }]);
        let config = Config {
            min_assessment_interval: Duration::from_millis(50),
            periodic_assessment: Duration::from_millis(150),
            overall_timeout: Duration::from_secs(20),
            max_retries: 1,
            ..Config::default()
        };

        let outcome = Factory::new(
            "Keep going forever.",
            workspace.path(),
            crate::agent::worker(endless_worker(), workspace.path()).unwrap(),
            crate::agent::verifier(endless_worker(), workspace.path()).unwrap(),
            foreman,
            config,
        )
        .run()
        .await;

        assert_eq!(outcome.status, Status::Escalated);
        // The original worker, and exactly one fresh attempt after it.
        assert_eq!(outcome.workers.len(), 2);
        assert!(outcome.workers.iter().all(|worker| !worker.is_active()));
        assert_eq!(
            outcome.last_intervention.map(|i| i.action),
            Some(Action::Escalate)
        );
    }

    #[tokio::test]
    async fn a_supervisor_that_cannot_see_escalates_rather_than_guessing() {
        let workspace = tempfile::tempdir().unwrap();
        let outcome = Factory::new(
            "Anything.",
            workspace.path(),
            crate::agent::worker(endless_worker(), workspace.path()).unwrap(),
            crate::agent::verifier(endless_worker(), workspace.path()).unwrap(),
            // An empty script cannot answer, which is the failure mode a real
            // classifier has when its service is down.
            Foreman::scripted([]),
            Config {
                min_assessment_interval: Duration::from_millis(20),
                periodic_assessment: Duration::from_millis(50),
                overall_timeout: Duration::from_secs(10),
                ..Config::default()
            },
        )
        .run()
        .await;

        assert_eq!(outcome.status, Status::Escalated);
        assert!(!outcome.failures.is_empty());
    }

    #[test]
    fn a_tool_call_is_summarized_by_its_first_real_line() {
        assert_eq!(
            first_line("\n\n  cat src/rates.py\nls tests\n"),
            "cat src/rates.py"
        );
        assert_eq!(first_line(""), "");
        let long = "x".repeat(200);
        let clipped = first_line(&long);
        assert_eq!(clipped.chars().count(), 80);
        assert!(clipped.ends_with('…'));
    }

    #[test]
    fn remaining_time_shrinks_and_never_goes_negative() {
        let budget = Duration::from_secs(30);
        assert_eq!(remaining(budget, None), Duration::ZERO);
        assert_eq!(
            remaining(budget, Some(Duration::from_secs(10))),
            Duration::from_secs(20)
        );
        assert_eq!(
            remaining(budget, Some(Duration::from_secs(90))),
            Duration::ZERO
        );
    }

    #[test]
    fn the_active_worker_is_the_one_still_running() {
        let mut workers = vec![
            WorkerRecord::new("worker-1", WorkerKind::Coding, 1, 100),
            WorkerRecord::new("worker-2", WorkerKind::Coding, 2, 100),
        ];
        workers[0].status = WorkerStatus::Completed;
        assert_eq!(active_id(&workers).as_deref(), Some("worker-2"));

        workers[1].status = WorkerStatus::Stopped;
        assert_eq!(active_id(&workers), None);
        assert!(active_ids(&workers).is_empty());
    }

    #[test]
    fn the_event_window_is_bounded() {
        let events = Mutex::new(VecDeque::new());
        for n in 0..500 {
            note(&events, format!("event-{n}"));
        }
        let events = lock(&events);
        assert_eq!(events.len(), 200);
        assert_eq!(events.back().map(String::as_str), Some("event-499"));
    }

    #[test]
    fn finishing_a_record_keeps_the_reason_the_supervisor_gave() {
        let mut workers = vec![WorkerRecord::new("worker-1", WorkerKind::Coding, 1, 100)];
        workers[0].stop_reason = Some("active worker appears stuck".into());
        finish_record(
            &mut workers,
            "worker-1",
            WorkerStatus::Stopped,
            Some("Cancelled".into()),
        );
        assert_eq!(workers[0].status, WorkerStatus::Stopped);
        assert!(workers[0].finished.is_some());
        assert_eq!(
            workers[0].stop_reason.as_deref(),
            Some("active worker appears stuck")
        );
    }

    #[test]
    fn run_ids_are_short_and_distinct_enough_to_name_a_run() {
        let first = run_id();
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(first.len(), 12);
        assert_ne!(first, run_id());
    }
}
