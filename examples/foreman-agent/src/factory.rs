//! Concurrent observation, assessment, and intervention. Workers retain their own loops.

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
use crate::worker::{CodingWorker, Crew, Feed, Stop};

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
    pub live_readings: usize,
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
    fn assessed(&self, iteration: usize, assessment: &Assessment, active: Option<&WorkerRecord>) {}
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

/// A crew supervised by a decision service.
pub struct Factory {
    config: Config,
    foreman: Foreman,
    crew: Crew,
    workspace: PathBuf,
    job: String,
    tests: Option<String>,
    test_image: String,
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
            test_image: "rust:1.96-bookworm".to_owned(),
            run_id: run_id(),
            watcher: Arc::new(Silent),
        }
    }

    /// Check the work by running `command` in the repository.
    pub fn testing(mut self, command: Option<String>) -> Self {
        self.tests = command;
        self
    }

    pub fn test_image(mut self, image: String) -> Self {
        self.test_image = image;
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

    pub fn verifier_model(&self) -> &str {
        &self.crew.model
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
        if state
            .tests
            .as_ref()
            .is_some_and(|(run, _)| run.exit_code.is_none() || run.exit_code == Some(125))
        {
            state.status = Status::Escalated;
            state.failures.push(
                "test runner unavailable; check Docker, source mounts, and --test-image".to_owned(),
            );
        }

        // Dispatch through the same Rust policy, before there is semantic evidence.
        if state.status == Status::Running {
            let dispatch = policy::decide(&state.floor(), &Assessment::default(), &self.config);
            self.apply(&mut state, dispatch, &signals).await;
        }

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

        // Do not report a terminal outcome while a cancelled worker is still live.
        self.stop_active(&mut state).await;
        let settled = tokio::time::timeout(Duration::from_secs(3), async {
            while active_id(&lock(&state.workers)).is_some() {
                if let Some(signal) = inbox.recv().await {
                    self.absorb(&mut state, signal);
                }
            }
        })
        .await;
        if settled.is_err() {
            state
                .failures
                .push("worker cancellation did not settle in 3s".to_owned());
        }

        Outcome {
            status: state.status,
            iterations: state.iteration,
            live_readings: state.live_readings,
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
            state.verification_completed = success;
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
    async fn check_tests(&self, state: &mut State) {
        let Some(command) = self.tests.as_deref() else {
            return;
        };
        if active_id(&lock(&state.workers)).is_some() {
            return;
        }
        let run =
            observation::run_tests(&self.workspace, command, &self.config, &self.test_image).await;
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

        let active = workers.iter().find(|worker| worker.is_active());
        state.live_readings += usize::from(active.is_some());
        self.watcher.assessed(state.iteration, &assessment, active);
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
        if kind == WorkerKind::Coding {
            state.verification_started = false;
            state.verification_completed = false;
        }
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

        let stop = match (&self.crew.coding, kind) {
            (CodingWorker::External(external), WorkerKind::Coding) => {
                let argv = external.command(&self.workspace, &mission);
                let notify = Arc::new(Notify::new());
                tokio::spawn(crate::worker::pump_process(
                    feed,
                    argv,
                    self.workspace.clone(),
                    external.credential_environment(),
                    Arc::clone(&notify),
                ));
                Stop::Process(notify)
            }
            (coding, _) => {
                let agent = match (coding, kind) {
                    (CodingWorker::Session(agent), WorkerKind::Coding) => (**agent).clone(),
                    _ => self.crew.verifier.clone(),
                };
                let session = self.crew.engine.create(agent);
                // Subscribe first so a fast worker cannot emit unseen evidence.
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
    live_readings: usize,
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
            live_readings: 0,
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
            tests_passed: self.tests.as_ref().map(|(run, _)| run.passed),
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
#[path = "../tests/unit/factory.rs"]
mod tests;
