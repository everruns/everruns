//! The runtime: two loops that do not block each other.
//!
//! The worker's loop is its own — an Everruns session, or an external CLI in a
//! child process. The supervisory loop runs beside it on whatever that worker
//! emits, so evidence reaches the classifier while the work is still happening.
//! Nothing has to stop for the factory to think.
//!
//! Between readings the loop debounces: activity marks the run dirty and waits
//! for the floor, and only a worker finishing bypasses it. Foreman's forcing
//! events are worker lifecycle boundaries, which happen a handful of times per
//! run — forcing on something that happens constantly, like a tool call, spends
//! the whole supervisory budget before the worker does.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, mpsc};

use crate::agent;
use crate::foreman::{Assessment, Foreman};
use crate::observation::{
    self, Evidence, TestRun, VerificationResult, WorkerKind, WorkerRecord, WorkerStatus,
};
use crate::policy::{self, Action, Config, Floor, Intervention};
use crate::worker::{Crew, Feed, Stop};

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
    /// A worker emitted output.
    fn worker_text(&self, worker_id: &str, delta: &str) {}
    /// A worker started a tool call, with the script it passed.
    fn worker_tool(&self, worker_id: &str, tool: &str, script: &str) {}
    /// A worker finished.
    fn worker_finished(&self, worker: &WorkerRecord) {}
    /// The supervisor took a reading.
    fn assessed(&self, iteration: usize, assessment: &Assessment) {}
    /// The supervisor could not take a reading.
    fn assessment_failed(&self, iteration: usize, error: &str) {}
    /// The policy decided.
    fn intervened(&self, intervention: &Intervention) {}
    /// The supervisor ran the repository's tests.
    fn tested(&self, run: &TestRun) {}
}

/// A watcher that renders nothing.
pub struct Silent;
impl Watcher for Silent {}

/// What a running worker tells the supervisory loop.
pub enum Signal {
    /// Something happened. Worth a reading once the debounce floor allows one.
    Activity,
    /// A worker finished. The one boundary that bypasses the floor, because the
    /// floor is exactly where an intervention stops being useful.
    Finished {
        /// Which worker.
        worker_id: String,
        /// Coding or verification.
        kind: WorkerKind,
        /// Whether it ended successfully.
        success: bool,
        /// What it said, unbounded here and bounded on the way into evidence.
        summary: String,
    },
}

/// A crew supervised by a classifier.
pub struct Factory {
    config: Config,
    foreman: Foreman,
    crew: Crew,
    workspace: PathBuf,
    job: String,
    tests: Option<String>,
    run_id: String,
    watcher: Arc<dyn Watcher>,
}

impl Factory {
    /// Assemble a factory over `workspace`.
    pub fn new(
        job: impl Into<String>,
        workspace: impl Into<PathBuf>,
        crew: Crew,
        foreman: Foreman,
        config: Config,
    ) -> Self {
        Self {
            config,
            foreman,
            crew,
            workspace: workspace.into(),
            job: job.into(),
            tests: None,
            run_id: run_id(),
            watcher: Arc::new(Silent),
        }
    }

    /// Check the work by running `command` in the repository.
    ///
    /// The supervisor runs it itself, the way it runs `git` itself: a worker
    /// reporting its own green suite is a claim, and this is the fact. Without
    /// it a run still works — `tests_sufficient` simply rests on someone
    /// reading the tests rather than on one having passed.
    pub fn testing(mut self, command: Option<String>) -> Self {
        self.tests = command;
        self
    }

    /// Render the run through `watcher`.
    pub fn watched_by(mut self, watcher: Arc<dyn Watcher>) -> Self {
        self.watcher = watcher;
        self
    }

    /// Who is doing the work, for display.
    pub fn crew_label(&self) -> String {
        self.crew.label()
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

        // A baseline before anyone touches the repository, so a suite that was
        // already red is not read as the worker having broken it.
        self.check_tests(&mut state).await;

        // The first worker starts unconditionally: there is nothing to assess
        // about a floor where no work has begun.
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
            let periodic = remaining(self.config.periodic_assessment, since);
            let floor = remaining(self.config.min_assessment_interval, since);
            let wait = if dirty { periodic.min(floor) } else { periodic };

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

            // Re-read the clock: `since` was measured before the wait, and
            // deciding on a stale reading costs a whole extra trip around the
            // loop before the floor is seen to have passed.
            let floor_passed = last_assessment
                .is_none_or(|at| at.elapsed() >= self.config.min_assessment_interval);
            if !(force || (dirty && floor_passed)) {
                continue;
            }

            let intervention = self.assess(state).await;
            last_assessment = Some(Instant::now());
            dirty = false;
            force = false;
            self.apply(state, intervention, signals).await;
        }
    }

    /// Fold one signal into the run's state, reporting whether it should bypass
    /// the debounce floor.
    fn absorb(&self, state: &mut State, signal: Signal) -> bool {
        let Signal::Finished {
            worker_id,
            kind,
            success,
            summary,
        } = signal
        else {
            return false;
        };

        if kind == WorkerKind::Verifier {
            state.verification_completed = true;
            state.verification.push(VerificationResult {
                worker_id: worker_id.clone(),
                passed: success,
                summary: observation::tail(&summary, self.config.output_limit),
            });
        }
        if !success {
            // Name what went wrong: "did not succeed" is not evidence a
            // supervisor, or a person reading the run, can act on.
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

    /// Run the repository's tests, when a command is configured and the floor
    /// is quiet enough for the answer to mean anything.
    ///
    /// A suite read mid-edit is a torn read, so it runs when no worker is
    /// active; between times the last result is carried, labelled with its age.
    async fn check_tests(&self, state: &mut State) {
        let Some(command) = self.tests.as_deref() else {
            return;
        };
        if active_id(&lock(&state.workers)).is_some() {
            return;
        }
        let run = observation::run_tests(&self.workspace, command, &self.config).await;
        self.watcher.tested(&run);
        note(
            &state.events,
            format!("tests {}", if run.passed { "passed" } else { "failed" }),
        );
        state.tests = Some((run, Instant::now()));
    }

    /// One supervisory iteration: observe, assess, decide.
    async fn assess(&self, state: &mut State) -> Intervention {
        state.iteration += 1;
        self.check_tests(state).await;
        let tests = state.tests.as_ref().map(|(run, at)| TestRun {
            ran_seconds_ago: (at.elapsed().as_secs_f64() * 10.0).round() / 10.0,
            ..run.clone()
        });
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
            tests,
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

    /// Carry out a decision. The only place the supervisor touches a worker,
    /// and the vocabulary is closed.
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

    /// Put a worker on the floor and start watching it.
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
        note(&state.events, format!("{id} started ({})", kind.label()));

        let mission = match kind {
            WorkerKind::Coding => agent::coding_mission(&self.job),
            WorkerKind::Verifier => agent::verification_mission(&self.job),
        };
        let feed = Feed {
            id: id.clone(),
            kind,
            workers: Arc::clone(&state.workers),
            events: Arc::clone(&state.events),
            watcher: Arc::clone(&self.watcher),
            signals: signals.clone(),
        };

        let stop = match &self.crew {
            Crew::Sessions(sessions) => {
                let agent = match kind {
                    WorkerKind::Coding => sessions.worker.clone(),
                    WorkerKind::Verifier => sessions.verifier.clone(),
                };
                let session = sessions.engine.create(agent);
                // Subscribe before sending: an event emitted between the two
                // would otherwise be evidence the supervisor never sees.
                let stream = session.events();
                match session.send(mission.as_str()).await {
                    Ok(pending) => {
                        let stop = Stop::Turn(pending.turn());
                        tokio::spawn(crate::worker::pump_session(feed, session, stream, pending));
                        stop
                    }
                    Err(error) => {
                        state
                            .failures
                            .push(format!("{id}: could not start: {error}"));
                        finish_unstarted(&state.workers, &id, signals, kind);
                        return;
                    }
                }
            }
            Crew::External(external) => {
                let argv = external.command(kind, &self.workspace, &mission);
                let notify = Arc::new(Notify::new());
                tokio::spawn(crate::worker::pump_process(
                    feed,
                    argv,
                    self.workspace.clone(),
                    Arc::clone(&notify),
                ));
                Stop::Process(notify)
            }
        };
        state.handles.insert(id, stop);
    }

    async fn stop(&self, state: &mut State, worker_id: &str, reason: &str) {
        if let Some(stop) = state.handles.get(worker_id) {
            stop.request().await;
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
    tests: Option<(TestRun, Instant)>,
    workers: Arc<Mutex<Vec<WorkerRecord>>>,
    events: Arc<Mutex<VecDeque<String>>>,
    handles: HashMap<String, Stop>,
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
            tests: None,
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

/// Settle a worker whose process or session never started.
fn finish_unstarted(
    workers: &Mutex<Vec<WorkerRecord>>,
    id: &str,
    signals: &mpsc::UnboundedSender<Signal>,
    kind: WorkerKind,
) {
    with(workers, id, |worker| {
        worker.status = WorkerStatus::Failed;
        worker.finished = Some(Instant::now());
    });
    let _ = signals.send(Signal::Finished {
        worker_id: id.to_owned(),
        kind,
        success: false,
        summary: String::new(),
    });
}

/// Edit one worker's record, if it is still there.
pub(crate) fn with(
    workers: &Mutex<Vec<WorkerRecord>>,
    id: &str,
    edit: impl FnOnce(&mut WorkerRecord),
) {
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

/// Append to the run's bounded event window.
pub(crate) fn note(events: &Mutex<VecDeque<String>>, line: String) {
    let mut events = lock(events);
    // Bounded here rather than at read time, so a long run does not accumulate
    // a timeline nobody will ever look at.
    if events.len() >= 200 {
        events.pop_front();
    }
    events.push_back(line);
}

/// Lock, treating a poisoned mutex as ordinary data.
///
/// The only writers are the worker pumps and the supervisor, and neither holds
/// the lock across an await: a poisoned lock means a panic elsewhere, not
/// evidence worth discarding.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
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

    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use everruns::{
        AgentLoopError, ClassificationAnswer, ClassificationOutcome, ClassificationRequest,
        Classifier, ClassifierService, LlmSimConfig, Model,
    };
    use serde_json::Value;

    use crate::foreman::DIMENSIONS;
    use crate::worker::ExternalAgent;

    /// A classifier service that answers from a closure over the observation.
    ///
    /// The stub receives the state as JSON, exactly as a vendor's service does,
    /// so a test drives the run through the same request and parsing a live
    /// reading uses.
    /// How a test answers one reading.
    type Reply = Box<dyn Fn(&Value) -> Result<Assessment, AgentLoopError> + Send + Sync>;

    struct Answering(Reply);

    #[async_trait::async_trait]
    impl ClassifierService for Answering {
        fn is_configured(&self) -> bool {
            true
        }

        async fn evaluate(
            &self,
            request: ClassificationRequest,
        ) -> Result<ClassificationOutcome, AgentLoopError> {
            let assessment = (self.0)(&request.state)?;
            Ok(ClassificationOutcome {
                model: "test".to_owned(),
                answers: DIMENSIONS
                    .iter()
                    .map(|dimension| {
                        (
                            dimension.id.to_owned(),
                            ClassificationAnswer::Noul {
                                probability: assessment.value(dimension.id),
                            },
                        )
                    })
                    .collect::<BTreeMap<_, _>>(),
                ..ClassificationOutcome::default()
            })
        }
    }

    fn answering(
        answer: impl Fn(&Value) -> Result<Assessment, AgentLoopError> + Send + Sync + 'static,
    ) -> Foreman {
        Foreman::new(
            Classifier::new("test", Answering(Box::new(answer))),
            Duration::from_secs(5),
        )
    }

    /// Whether a worker was still on the floor when this reading was taken.
    fn working(state: &Value) -> bool {
        state
            .get("active_workers")
            .and_then(Value::as_array)
            .is_some_and(|workers| !workers.is_empty())
    }

    /// A worker that takes long enough that "during" is unambiguous.
    fn slow_worker() -> Model {
        Model::simulated_with_config(
            LlmSimConfig::fixed("Finished.").with_response_delay(Duration::from_secs(2)),
        )
    }

    fn sessions(root: &std::path::Path, model: impl Fn() -> Model) -> Crew {
        Crew::sessions(
            crate::agent::worker(model(), root).unwrap(),
            crate::agent::verifier(model(), root).unwrap(),
        )
    }

    /// Quick clocks, one worker: the run ends as soon as that worker does.
    fn brisk() -> Config {
        Config {
            min_assessment_interval: Duration::from_millis(100),
            periodic_assessment: Duration::from_millis(200),
            overall_timeout: Duration::from_secs(30),
            max_workers: 1,
            ..Config::default()
        }
    }

    fn healthy() -> Assessment {
        Assessment {
            meaningful_progress: 0.9,
            ..Assessment::default()
        }
    }

    #[tokio::test]
    async fn readings_land_while_the_worker_is_still_working() {
        // The whole architectural claim: supervision runs *during* the work,
        // not after it. A reading that sees an active worker is a reading taken
        // before that worker's turn resolved.
        let workspace = tempfile::tempdir().unwrap();
        let live = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&live);
        let foreman = answering(move |state| {
            if working(state) {
                counter.fetch_add(1, Ordering::SeqCst);
            }
            Ok(healthy())
        });

        let outcome = Factory::new(
            "Take your time.",
            workspace.path(),
            sessions(workspace.path(), slow_worker),
            foreman,
            brisk(),
        )
        .run()
        .await;

        let live = live.load(Ordering::SeqCst);
        assert!(
            live >= 2,
            "expected several readings during the turn, got {live}"
        );
        // And the worker really did keep working through them.
        assert!(outcome.workers[0].elapsed() >= Duration::from_secs(2));
        assert_eq!(outcome.workers[0].status, WorkerStatus::Completed);
    }

    #[tokio::test]
    async fn a_stuck_worker_is_stopped_retried_once_and_then_escalated() {
        let workspace = tempfile::tempdir().unwrap();
        // One reading, held for the whole run: the worker is stuck. The policy
        // still has to walk stop → retry → escalate rather than repeat itself.
        let foreman = answering(|_| {
            Ok(Assessment {
                worker_stuck: 0.95,
                meaningful_progress: 0.05,
                ..Assessment::default()
            })
        });
        let config = Config {
            min_assessment_interval: Duration::from_millis(50),
            periodic_assessment: Duration::from_millis(150),
            overall_timeout: Duration::from_secs(20),
            max_retries: 1,
            ..Config::default()
        };
        let endless = || {
            Model::simulated_with_config(
                LlmSimConfig::fixed("Still working on it.")
                    .with_response_delay(Duration::from_secs(30)),
            )
        };

        let outcome = Factory::new(
            "Keep going forever.",
            workspace.path(),
            sessions(workspace.path(), endless),
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
            sessions(workspace.path(), slow_worker),
            answering(|_| Err(AgentLoopError::llm("classifier service is down"))),
            brisk(),
        )
        .run()
        .await;

        assert_eq!(outcome.status, Status::Escalated);
        assert!(!outcome.failures.is_empty());
    }

    /// A stand-in for Codex or yolop: a real child process that streams.
    fn stand_in(script: &str) -> ExternalAgent {
        let argv: Vec<String> = ["bash", "-c", script, "foreman-worker", "{mission}"]
            .iter()
            .map(|word| (*word).to_owned())
            .collect();
        ExternalAgent {
            label: "stand-in".to_owned(),
            coding: argv.clone(),
            verifying: argv,
        }
    }

    #[tokio::test]
    async fn an_external_worker_is_watched_while_it_runs() {
        // Neither Codex nor yolop is installed in CI, and neither needs to be:
        // what this proves is the path they travel — a child process whose
        // output reaches the supervisor before the process exits.
        let workspace = tempfile::tempdir().unwrap();
        let live = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&live);
        let foreman = answering(move |state| {
            if working(state) {
                counter.fetch_add(1, Ordering::SeqCst);
            }
            Ok(healthy())
        });

        let outcome = Factory::new(
            "Do the thing.",
            workspace.path(),
            Crew::External(stand_in(
                r#"printf '{"type":"read_file"}\n'; sleep 2; printf 'done\n'"#,
            )),
            foreman,
            brisk(),
        )
        .run()
        .await;

        assert!(
            live.load(Ordering::SeqCst) >= 2,
            "an external worker must be observable while it runs"
        );
        let worker = &outcome.workers[0];
        assert_eq!(worker.status, WorkerStatus::Completed);
        assert!(worker.output.as_str().contains("done"));
        // A JSONL line names its own step, so it counts as one.
        assert_eq!(worker.last_tool.as_deref(), Some("read_file"));
        // The mission reached the process as one argument.
        assert!(worker.output.as_str().contains("Do the thing.") || worker.tool_calls == 1);
    }

    #[tokio::test]
    async fn a_missing_external_agent_fails_by_name_rather_than_hanging() {
        let workspace = tempfile::tempdir().unwrap();
        let mut agent = stand_in("true");
        "no-such-coding-agent-on-this-machine".clone_into(&mut agent.coding[0]);
        agent.verifying = agent.coding.clone();

        let outcome = Factory::new(
            "Do the thing.",
            workspace.path(),
            Crew::External(agent),
            answering(|_| Ok(healthy())),
            brisk(),
        )
        .run()
        .await;

        assert_eq!(outcome.workers[0].status, WorkerStatus::Failed);
        let failure = outcome.failures.join(" ");
        assert!(
            failure.contains("no-such-coding-agent-on-this-machine"),
            "the failure should name the missing binary: {failure}"
        );
    }

    #[tokio::test]
    async fn an_external_worker_is_killed_when_the_policy_stops_it() {
        let workspace = tempfile::tempdir().unwrap();
        let outcome = Factory::new(
            "Loop forever.",
            workspace.path(),
            // Nothing about this process ends on its own.
            Crew::External(stand_in("while true; do sleep 1; done")),
            answering(|_| {
                Ok(Assessment {
                    worker_stuck: 0.95,
                    ..Assessment::default()
                })
            }),
            Config {
                min_assessment_interval: Duration::from_millis(50),
                periodic_assessment: Duration::from_millis(150),
                overall_timeout: Duration::from_secs(20),
                max_retries: 0,
                ..Config::default()
            },
        )
        .run()
        .await;

        assert_eq!(outcome.status, Status::Escalated);
        assert_eq!(outcome.workers[0].status, WorkerStatus::Stopped);
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
    fn run_ids_are_short_and_distinct_enough_to_name_a_run() {
        let first = run_id();
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(first.len(), 12);
        assert_ne!(first, run_id());
    }
}
