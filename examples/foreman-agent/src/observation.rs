//! What the supervisor is allowed to see.
//!
//! Never the repository. An observation is a compact, bounded snapshot: worker
//! lifecycle, output tails, the Git diff, recent session events, and the
//! previous decision. Bounds are not politeness — an unbounded observation
//! makes assessment as slow as the work it is supposed to be watching.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::foreman::Assessment;
use crate::policy::{Config, Intervention};

/// Which mission a worker was started on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerKind {
    /// Does the software engineering.
    Coding,
    /// Independently checks what the coding workers left behind.
    Verifier,
}

impl WorkerKind {
    /// The kind as it is written in the timeline.
    pub fn label(self) -> &'static str {
        match self {
            Self::Coding => "coding",
            Self::Verifier => "verifier",
        }
    }
}

/// Where a worker's turn ended up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    /// Still working.
    Running,
    /// Finished its turn successfully.
    Completed,
    /// Its turn failed.
    Failed,
    /// Cancelled by the supervisor.
    Stopped,
}

impl WorkerStatus {
    /// The status as it is written in the timeline.
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// One worker, as the factory has watched it so far.
///
/// Written by the event pump while the worker's turn is live, read by the
/// supervisor between assessments. Nothing here is the model's summary of
/// itself: every field is derived from the session's own canonical events.
#[derive(Clone, Debug)]
pub struct WorkerRecord {
    /// `worker-1`, `worker-2`, …
    pub id: String,
    /// Coding or verification.
    pub kind: WorkerKind,
    /// Which attempt this worker is.
    pub attempt: usize,
    /// Where its turn stands.
    pub status: WorkerStatus,
    /// When the turn was sent.
    pub started: Instant,
    /// When the turn ended.
    pub finished: Option<Instant>,
    /// Model inference steps observed.
    pub iterations: usize,
    /// Tool calls observed.
    pub tool_calls: usize,
    /// The tool the worker is running, or ran last.
    pub last_tool: Option<String>,
    /// Interleaved assistant text and tool output, kept to a bounded tail.
    pub output: BoundedTail,
    /// Why the turn ended, when it ended for a nameable reason.
    pub stop_reason: Option<String>,
}

impl WorkerRecord {
    /// Start a record for a worker whose turn is about to be sent.
    pub fn new(id: impl Into<String>, kind: WorkerKind, attempt: usize, limit: usize) -> Self {
        Self {
            id: id.into(),
            kind,
            attempt,
            status: WorkerStatus::Running,
            started: Instant::now(),
            finished: None,
            iterations: 0,
            tool_calls: 0,
            last_tool: None,
            output: BoundedTail::new(limit),
            stop_reason: None,
        }
    }

    /// How long the worker has been running, or ran.
    pub fn elapsed(&self) -> Duration {
        self.finished
            .unwrap_or_else(Instant::now)
            .saturating_duration_since(self.started)
    }

    /// Whether the worker's turn is still live.
    pub fn is_active(&self) -> bool {
        self.status == WorkerStatus::Running
    }

    fn view(&self, limit: usize) -> WorkerView {
        WorkerView {
            worker_id: self.id.clone(),
            worker_kind: self.kind,
            status: self.status,
            attempt: self.attempt,
            elapsed_seconds: round(self.elapsed().as_secs_f64()),
            iterations: self.iterations,
            tool_calls: self.tool_calls,
            last_tool: self.last_tool.clone(),
            output_tail: tail(self.output.as_str(), limit),
            stop_reason: self.stop_reason.clone(),
        }
    }
}

/// A string that keeps its most recent `limit` characters.
///
/// The tail is the part worth having: a worker that is looping says so in what
/// it is doing now, not in what it did first.
#[derive(Clone, Debug)]
pub struct BoundedTail {
    text: String,
    limit: usize,
}

impl BoundedTail {
    /// A tail bounded to roughly `limit` characters.
    pub fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            limit: limit.max(1),
        }
    }

    /// Append, dropping from the front once the buffer has grown past twice the
    /// bound. Trimming in blocks keeps this out of the per-delta hot path.
    pub fn push(&mut self, chunk: &str) {
        self.text.push_str(chunk);
        if self.text.chars().count() > self.limit * 2 {
            let keep: String = self
                .text
                .chars()
                .skip(self.text.chars().count() - self.limit)
                .collect();
            self.text = keep;
        }
    }

    /// The text held right now.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

/// Keep the last `limit` characters, saying how much was dropped.
pub fn tail(value: &str, limit: usize) -> String {
    let total = value.chars().count();
    if total <= limit {
        return value.to_owned();
    }
    let kept: String = value.chars().skip(total - limit).collect();
    format!(
        "[... {} earlier characters omitted ...]\n{kept}",
        total - limit
    )
}

/// What an independent verification pass reported.
#[derive(Clone, Debug, Serialize)]
pub struct VerificationResult {
    /// The worker that ran it.
    pub worker_id: String,
    /// Whether its turn succeeded. Not a proof of correctness — it is one more
    /// piece of evidence, assessed like any other.
    pub passed: bool,
    /// What it said, bounded.
    pub summary: String,
}

/// What the repository's own test command reported, run by the host.
///
/// Foreman declares this field and never fills it. It is the one piece of
/// evidence that settles `tests_sufficient`, and the only honest way to get it
/// is the way the supervisor already gets a diff: run the command yourself
/// rather than ask the worker how it went.
#[derive(Clone, Debug, Serialize)]
pub struct TestRun {
    /// The command, as configured.
    pub command: String,
    /// Its exit status, or `None` when it could not be started.
    pub exit_code: Option<i32>,
    /// Whether it exited zero.
    pub passed: bool,
    /// What it printed, bounded.
    pub output_tail: String,
    /// How long ago it ran, in seconds.
    ///
    /// A suite is slower than a diff, so it is run on a quiet floor and its
    /// result carried while a worker works. Saying how stale it is keeps that
    /// from reading as fresh.
    pub ran_seconds_ago: f64,
}

/// Run `command` in `repository` and report what happened.
///
/// The child is spawned asynchronously and killed when the budget runs out —
/// a suite that hangs must not outlive the reading that asked for it, or take
/// the runtime down with it at shutdown.
pub async fn run_tests(repository: &Path, command: &str, config: &Config) -> TestRun {
    let spawned = tokio::process::Command::new("bash")
        .arg("-c")
        .arg(command)
        .current_dir(repository)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn();

    let budget = config.test_timeout;
    let (exit_code, passed, text) = match spawned {
        Ok(child) => match tokio::time::timeout(budget, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                (output.status.code(), output.status.success(), text)
            }
            Ok(Err(error)) => (None, false, format!("could not run the tests: {error}")),
            // Dropping the future drops the child, and `kill_on_drop` ends it.
            Err(_) => (
                None,
                false,
                format!("tests exceeded {:.0}s", budget.as_secs_f64()),
            ),
        },
        Err(error) => (None, false, format!("could not run the tests: {error}")),
    };

    TestRun {
        command: command.to_owned(),
        exit_code,
        passed,
        output_tail: tail(&text, config.output_limit),
        ran_seconds_ago: 0.0,
    }
}

/// Git evidence, gathered by the host rather than asked of the worker.
#[derive(Clone, Debug, Default)]
pub struct GitEvidence {
    /// `git status --short`.
    pub status: String,
    /// `git diff`, bounded.
    pub diff: String,
    /// Paths from `git diff --name-only`.
    pub changed_files: Vec<String>,
}

/// Read the repository's own account of what changed.
///
/// The worker is not asked what it did; the diff is taken from the working copy
/// it has been editing. Missing Git is not an error — the supervisor simply has
/// less to look at, and the other evidence still stands.
pub async fn git_evidence(repository: &Path, config: &Config) -> GitEvidence {
    let (status, diff, names, untracked) = tokio::join!(
        git(repository, &["status", "--short"], config.output_limit),
        git(repository, &["diff", "--no-ext-diff"], config.diff_limit),
        git(repository, &["diff", "--name-only"], config.output_limit),
        git(
            repository,
            &["ls-files", "--others", "--exclude-standard"],
            config.output_limit
        ),
    );
    // A new file is invisible to `git diff` until it is staged, and a new test
    // file is exactly the evidence a "are the tests sufficient?" question turns
    // on. Listing the untracked paths costs nothing and closes that blind spot
    // without touching the index.
    let changed_files = names
        .lines()
        .chain(untracked.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(config.history_limit * 10)
        .map(str::to_owned)
        .collect();
    GitEvidence {
        status,
        diff,
        changed_files,
    }
}

async fn git(repository: &Path, args: &[&str], limit: usize) -> String {
    let repository: PathBuf = repository.to_path_buf();
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repository)
            .args(&args)
            .output()
    })
    .await;
    match output {
        Ok(Ok(output)) if output.status.success() => {
            tail(&String::from_utf8_lossy(&output.stdout), limit)
        }
        _ => String::new(),
    }
}

/// One worker as the classifier sees it.
#[derive(Clone, Debug, Serialize)]
pub struct WorkerView {
    worker_id: String,
    worker_kind: WorkerKind,
    status: WorkerStatus,
    attempt: usize,
    elapsed_seconds: f64,
    iterations: usize,
    tool_calls: usize,
    last_tool: Option<String>,
    output_tail: String,
    stop_reason: Option<String>,
}

/// The whole bounded snapshot, and the state the nine questions are asked about.
#[derive(Clone, Debug, Serialize)]
pub struct Observation {
    /// The free-form job, verbatim.
    pub original_job: String,
    /// This run.
    pub run_id: String,
    /// Where the factory stands.
    pub factory_status: String,
    /// Supervisory iteration.
    pub iteration: usize,
    /// Workers running right now.
    pub active_workers: Vec<WorkerView>,
    /// Recent workers, oldest first.
    pub worker_history: Vec<WorkerView>,
    /// The most recent worker output, bounded.
    pub latest_worker_output: String,
    /// `git status --short`.
    pub git_status: String,
    /// `git diff`, bounded.
    pub git_diff: String,
    /// Paths changed in the working copy.
    pub changed_files: Vec<String>,
    /// What the repository's own tests reported, run by the host.
    ///
    /// Empty when no test command is configured, which is honest: the
    /// dimension then rests on someone reading the tests rather than on one
    /// having passed.
    pub test_results: Vec<TestRun>,
    /// What verification passes reported.
    pub verification_results: Vec<VerificationResult>,
    /// Recent session events, as one line each.
    pub recent_events: Vec<String>,
    /// The previous assessment, so drift is visible to the model.
    pub previous_assessment: Option<Assessment>,
    /// The previous decision.
    pub previous_intervention: Option<Intervention>,
    /// Workers started so far.
    pub attempts: usize,
    /// Failures recorded so far.
    pub failures: Vec<String>,
    /// Wall-clock time since the factory started.
    pub elapsed_factory_seconds: f64,
}

/// Everything an observation is built from.
pub struct Evidence<'a> {
    /// The free-form job.
    pub job: &'a str,
    /// This run.
    pub run_id: &'a str,
    /// Where the factory stands.
    pub status: &'a str,
    /// Supervisory iteration.
    pub iteration: usize,
    /// Every worker so far, oldest first.
    pub workers: &'a [WorkerRecord],
    /// What verification reported.
    pub verification: &'a [VerificationResult],
    /// Recent session events, oldest first.
    pub events: &'a VecDeque<String>,
    /// The previous assessment.
    pub previous_assessment: Option<Assessment>,
    /// The previous decision.
    pub previous_intervention: Option<Intervention>,
    /// Failures recorded so far.
    pub failures: &'a [String],
    /// Time since the factory started.
    pub elapsed: Duration,
    /// Git evidence.
    pub git: GitEvidence,
    /// The most recent test run, with its age already set.
    pub tests: Option<TestRun>,
    /// The bounds to apply.
    pub config: &'a Config,
}

/// Assemble one bounded observation.
pub fn build(evidence: Evidence<'_>) -> Observation {
    let config = evidence.config;
    let history_start = evidence.workers.len().saturating_sub(config.history_limit);
    let history = &evidence.workers[history_start..];
    let latest = history.last();

    Observation {
        original_job: tail(evidence.job, config.output_limit),
        run_id: evidence.run_id.to_owned(),
        factory_status: evidence.status.to_owned(),
        iteration: evidence.iteration,
        active_workers: history
            .iter()
            .filter(|worker| worker.is_active())
            .map(|worker| worker.view(config.output_limit))
            .collect(),
        worker_history: history
            .iter()
            .map(|worker| worker.view(config.output_limit))
            .collect(),
        latest_worker_output: latest
            .map(|worker| tail(worker.output.as_str(), config.output_limit))
            .unwrap_or_default(),
        git_status: evidence.git.status,
        git_diff: evidence.git.diff,
        changed_files: evidence.git.changed_files,
        test_results: evidence.tests.into_iter().collect(),
        verification_results: evidence.verification.to_vec(),
        recent_events: evidence
            .events
            .iter()
            .rev()
            .take(config.event_limit)
            .rev()
            .cloned()
            .collect(),
        previous_assessment: evidence.previous_assessment,
        previous_intervention: evidence.previous_intervention,
        attempts: evidence.workers.len(),
        failures: evidence
            .failures
            .iter()
            .rev()
            .take(config.history_limit)
            .rev()
            .cloned()
            .collect(),
        elapsed_factory_seconds: round(evidence.elapsed.as_secs_f64()),
    }
}

fn round(seconds: f64) -> f64 {
    (seconds * 10.0).round() / 10.0
}

/// Test fixtures, compiled only for tests and for the `testing` feature the
/// demo crate turns on for its own.
#[cfg(any(test, feature = "testing"))]
impl Observation {
    /// A minimal observation, for tests that need one but do not read it.
    pub fn sample() -> Self {
        let workers = [WorkerRecord::new("worker-1", WorkerKind::Coding, 1, 100)];
        build(Evidence {
            job: "Add tiered shipping rates.",
            run_id: "test-run",
            status: "running",
            iteration: 1,
            workers: &workers,
            verification: &[],
            events: &VecDeque::new(),
            previous_assessment: None,
            previous_intervention: None,
            failures: &[],
            elapsed: Duration::from_secs(3),
            git: GitEvidence::default(),
            tests: None,
            config: &Config::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tail_keeps_the_end_and_says_what_it_dropped() {
        let kept = tail("abcdefghij", 4);
        assert!(kept.ends_with("ghij"), "{kept}");
        assert!(kept.contains("6 earlier characters omitted"), "{kept}");
        assert_eq!(tail("abc", 10), "abc");
    }

    #[test]
    fn a_bounded_tail_stops_growing() {
        let mut output = BoundedTail::new(50);
        for _ in 0..200 {
            output.push("0123456789");
        }
        assert!(output.as_str().chars().count() <= 100);
        assert!(output.as_str().ends_with("0123456789"));
    }

    #[test]
    fn history_and_events_are_bounded_to_their_configured_limits() {
        let config = Config {
            history_limit: 2,
            event_limit: 3,
            ..Config::default()
        };
        let workers: Vec<WorkerRecord> = (1..=5)
            .map(|n| WorkerRecord::new(format!("worker-{n}"), WorkerKind::Coding, n, 100))
            .collect();
        let events: VecDeque<String> = (1..=10).map(|n| format!("event-{n}")).collect();

        let observation = build(Evidence {
            job: "job",
            run_id: "run",
            status: "running",
            iteration: 4,
            workers: &workers,
            verification: &[],
            events: &events,
            previous_assessment: None,
            previous_intervention: None,
            failures: &[],
            elapsed: Duration::from_secs(1),
            git: GitEvidence::default(),
            tests: None,
            config: &config,
        });

        assert_eq!(observation.worker_history.len(), 2);
        assert_eq!(observation.worker_history[1].worker_id, "worker-5");
        // The most recent events, in the order they happened.
        assert_eq!(
            observation.recent_events,
            vec!["event-8", "event-9", "event-10"]
        );
        // Every worker is still counted, even when only some are described.
        assert_eq!(observation.attempts, 5);
    }

    #[test]
    fn only_running_workers_are_active() {
        let mut workers = vec![
            WorkerRecord::new("worker-1", WorkerKind::Coding, 1, 100),
            WorkerRecord::new("worker-2", WorkerKind::Verifier, 2, 100),
        ];
        workers[0].status = WorkerStatus::Completed;
        workers[0].finished = Some(Instant::now());

        let observation = build(Evidence {
            job: "job",
            run_id: "run",
            status: "running",
            iteration: 2,
            workers: &workers,
            verification: &[],
            events: &VecDeque::new(),
            previous_assessment: None,
            previous_intervention: None,
            failures: &[],
            elapsed: Duration::from_secs(1),
            git: GitEvidence::default(),
            tests: None,
            config: &Config::default(),
        });

        assert_eq!(observation.active_workers.len(), 1);
        assert_eq!(observation.active_workers[0].worker_id, "worker-2");
        assert_eq!(observation.worker_history.len(), 2);
    }

    #[tokio::test]
    async fn a_test_run_reports_what_the_command_did() {
        let root = tempfile::tempdir().unwrap();
        let config = Config::default();

        let green = run_tests(root.path(), "echo '2 passed, 0 failed'", &config).await;
        assert!(green.passed);
        assert_eq!(green.exit_code, Some(0));
        assert!(green.output_tail.contains("2 passed"));

        let red = run_tests(root.path(), "echo boom >&2; exit 3", &config).await;
        assert!(!red.passed);
        assert_eq!(red.exit_code, Some(3));
        // Both streams are evidence; a failure usually explains itself on stderr.
        assert!(red.output_tail.contains("boom"));
    }

    #[tokio::test]
    async fn a_test_command_that_never_returns_is_bounded() {
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            test_timeout: Duration::from_millis(200),
            ..Config::default()
        };
        let run = run_tests(root.path(), "sleep 30", &config).await;
        assert!(!run.passed);
        assert_eq!(run.exit_code, None);
        assert!(run.output_tail.contains("exceeded"), "{}", run.output_tail);
    }

    #[test]
    fn an_observation_serializes_to_classifier_state() {
        let state = serde_json::to_value(Observation::sample()).unwrap();
        assert!(state.get("original_job").is_some());
        assert!(state.get("git_diff").is_some());
        assert_eq!(state["active_workers"][0]["worker_kind"], "coding");
    }
}
