//! Who does the work, and how the factory watches it.
//!
//! Two backends, one evidence channel. An Everruns session is observed through
//! its own canonical event stream — typed, with tool calls and model steps
//! already separated. An external coding agent is a child process observed
//! through stdout and stderr, which is all any harness gets from a CLI.
//!
//! The supervisor cannot tell them apart, and that is the point: what it reads
//! is a bounded [`Observation`](crate::observation::Observation), and the
//! strongest evidence in one — the repository's own diff — is gathered by the
//! host either way. Swapping the worker does not change the supervision.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use everruns::{Agent, Engine, SessionEventKind, TurnHandle, TurnStopReason};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Notify, mpsc};

use crate::factory::{Signal, Watcher, lock, note, with};
use crate::observation::{WorkerKind, WorkerRecord, WorkerStatus};

/// The `{repo}` placeholder in an external agent's command template.
const REPO: &str = "{repo}";
/// The `{mission}` placeholder in an external agent's command template.
const MISSION: &str = "{mission}";

/// Who is on the floor.
pub enum Crew {
    /// Everruns sessions on the Bashkit shell.
    Sessions(Box<Sessions>),
    /// An external coding-agent CLI, one child process per worker.
    External(ExternalAgent),
}

/// Two agents over one workspace, and the engine that runs their sessions.
///
/// The verifier is a second agent over the same directory under the default
/// read-only policy, so "independent check" is a property of the mount rather
/// than a request in a prompt a model may decline to honor.
pub struct Sessions {
    /// Edits the repository.
    pub worker: Agent,
    /// Reads it.
    pub verifier: Agent,
    /// Owns the sessions.
    pub engine: Engine,
}

impl Crew {
    /// A crew of Everruns sessions.
    pub fn sessions(worker: Agent, verifier: Agent) -> Self {
        Self::Sessions(Box::new(Sessions {
            worker,
            verifier,
            engine: Engine::new(),
        }))
    }

    /// What to call this crew in the run's header.
    pub fn label(&self) -> String {
        match self {
            Self::Sessions(_) => format!("{} on the bashkit shell", crate::agent::WORKER_MODEL),
            Self::External(agent) => format!("{} (external CLI)", agent.label),
        }
    }
}

/// An external coding agent, as the command line that starts one.
///
/// Templates rather than hard-coded argv, because the tools move: a preset that
/// stops matching its CLI is one `--worker-command` away from working again,
/// and the example does not have to pretend it tracks every release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalAgent {
    /// `codex`, `yolop`, or whatever the operator named.
    pub label: String,
    /// Argv for a coding pass. `{repo}` and `{mission}` are substituted whole.
    pub coding: Vec<String>,
    /// Argv for a verification pass.
    pub verifying: Vec<String>,
}

/// Why an external agent could not be described.
#[derive(Debug)]
pub enum TemplateError {
    /// The template had no words in it.
    Empty,
    /// The template never says where the mission goes.
    NoMission,
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "worker command is empty"),
            Self::NoMission => write!(f, "worker command has no {MISSION} placeholder"),
        }
    }
}

impl std::error::Error for TemplateError {}

impl ExternalAgent {
    /// The Codex CLI, on the stable non-interactive `exec` interface.
    ///
    /// This is the line Foreman itself runs, with one change: the verification
    /// pass asks for `read-only`, so an independent check cannot quietly become
    /// another coding pass.
    pub fn codex() -> Self {
        let base = ["codex", "exec", "--cd", REPO, "--color", "never", "--json"];
        Self {
            label: "codex".to_owned(),
            coding: argv(&base, "workspace-write"),
            verifying: argv(&base, "read-only"),
        }
    }

    /// yolop, on its one-shot `--print` interface.
    ///
    /// yolop publishes no read-only mode, so both passes run the same way and
    /// the verifier's independence rests on its mission alone. That is weaker
    /// than the session backend, and worth knowing before trusting a result.
    pub fn yolop() -> Self {
        let argv: Vec<String> = ["yolop", "-C", REPO, "-p", MISSION]
            .iter()
            .map(|word| (*word).to_owned())
            .collect();
        Self {
            label: "yolop".to_owned(),
            coding: argv.clone(),
            verifying: argv,
        }
    }

    /// An agent described by the operator, as a whitespace-separated template.
    pub fn from_template(template: &str) -> Result<Self, TemplateError> {
        let words: Vec<String> = template.split_whitespace().map(str::to_owned).collect();
        let label = words.first().cloned().ok_or(TemplateError::Empty)?;
        if !words.iter().any(|word| word == MISSION) {
            return Err(TemplateError::NoMission);
        }
        Ok(Self {
            label,
            coding: words.clone(),
            verifying: words,
        })
    }

    /// The command line for one worker, with the placeholders filled in.
    ///
    /// Substitution is whole-word, never string interpolation: a mission is
    /// one argv entry however many quotes, newlines, or semicolons it holds,
    /// and there is no shell between here and the process.
    pub fn command(&self, kind: WorkerKind, repo: &Path, mission: &str) -> Vec<String> {
        let template = match kind {
            WorkerKind::Coding => &self.coding,
            WorkerKind::Verifier => &self.verifying,
        };
        template
            .iter()
            .map(|word| match word.as_str() {
                REPO => repo.display().to_string(),
                MISSION => mission.to_owned(),
                other => other.to_owned(),
            })
            .collect()
    }
}

fn argv(base: &[&str], sandbox: &str) -> Vec<String> {
    let mut argv: Vec<String> = base.iter().map(|word| (*word).to_owned()).collect();
    argv.push("--sandbox".to_owned());
    argv.push(sandbox.to_owned());
    argv.push(MISSION.to_owned());
    argv
}

/// How the factory asks a worker to stop.
///
/// Cooperative for a session: the turn stops at its next boundary and resolves
/// as cancelled, leaving the session's own history consistent. Blunt for a
/// process, because a CLI offers nothing better.
pub enum Stop {
    /// Cancel a session's turn.
    Turn(TurnHandle),
    /// Ask a child process to die.
    Process(Arc<Notify>),
}

impl Stop {
    /// Ask the worker to stop. Returning does not mean it has.
    pub async fn request(&self) {
        match self {
            Self::Turn(handle) => {
                let _ = handle.cancel().await;
            }
            // `notify_one` leaves a permit, so a pump that is not yet waiting
            // still sees the request when it gets there.
            Self::Process(notify) => notify.notify_one(),
        }
    }
}

/// Everything a running worker writes its evidence into.
pub struct Feed {
    /// `worker-1`, `worker-2`, …
    pub id: String,
    /// Coding or verification.
    pub kind: WorkerKind,
    /// Every worker so far; this one is the last.
    pub workers: Arc<Mutex<Vec<WorkerRecord>>>,
    /// The run's bounded event window.
    pub events: Arc<Mutex<VecDeque<String>>>,
    /// Presentation.
    pub watcher: Arc<dyn Watcher>,
    /// Wakes the supervisory loop.
    pub signals: mpsc::UnboundedSender<Signal>,
}

impl Feed {
    fn finish(&self, status: WorkerStatus, success: bool, summary: String, reason: Option<String>) {
        with(&self.workers, &self.id, |worker| {
            worker.status = status;
            worker.finished = Some(std::time::Instant::now());
            if worker.stop_reason.is_none() {
                worker.stop_reason = reason;
            }
        });
        note(&self.events, format!("{} turn {}", self.id, status.label()));
        let _ = self.signals.send(Signal::Finished {
            worker_id: self.id.clone(),
            kind: self.kind,
            success,
            summary,
        });
    }
}

/// Drain one session's canonical events into the evidence the supervisor reads.
///
/// This is the whole coupling between the two loops: the worker never waits for
/// it, and it never speaks back into the session.
pub async fn pump_session(
    feed: Feed,
    session: everruns::Session,
    mut stream: everruns::EventStream,
    pending: everruns::SentMessage,
) {
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
        if event.kind.is_terminal() {
            // Deliberately silent: the run's view of a finished worker is not
            // complete until its turn has resolved and any verification result
            // has been recorded. `Signal::Finished` is that boundary.
            break;
        }
        match &event.kind {
            SessionEventKind::TextDelta { delta } => {
                with(&feed.workers, &feed.id, |worker| worker.output.push(delta));
                feed.watcher.worker_text(&feed.id, delta);
            }
            SessionEventKind::ReasonStarted => {
                with(&feed.workers, &feed.id, |worker| worker.iterations += 1);
            }
            SessionEventKind::ToolStarted { tool_name, .. } => {
                // The reviewed surface names the tool; the script it was called
                // with lives in the canonical payload, and that is what says
                // whether a worker is repeating itself.
                let script = script_of(&event);
                with(&feed.workers, &feed.id, |worker| {
                    worker.tool_calls += 1;
                    worker.last_tool = Some(format!("{tool_name}: {}", first_line(&script)));
                    // What the worker ran belongs in the tail beside what came
                    // back: a diff shows the file it wrote, this shows the one
                    // it only meant to.
                    worker.output.push(&format!("\n$ {script}\n"));
                });
                feed.watcher.worker_tool(&feed.id, tool_name, &script);
                note(
                    &feed.events,
                    format!("{} tool {tool_name} {}", feed.id, first_line(&script)),
                );
            }
            SessionEventKind::ToolOutputDelta { delta, .. } => {
                with(&feed.workers, &feed.id, |worker| worker.output.push(delta));
            }
            SessionEventKind::ToolCompleted {
                tool_name, success, ..
            } => note(
                &feed.events,
                format!(
                    "{} tool {tool_name} {}",
                    feed.id,
                    if *success { "ok" } else { "failed" }
                ),
            ),
            _ => {}
        }
        let _ = feed.signals.send(Signal::Activity);
    }

    let (status, success, summary, reason) = match pending.wait().await {
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
    with(&feed.workers, &feed.id, |worker| {
        worker.output.push(&summary)
    });
    feed.finish(status, success, summary, reason);
    // The session is dropped here, after its turn has resolved.
    drop(session);
}

/// Run an external coding agent and stream what it prints.
///
/// A CLI gives no typed events, so every line is evidence of the same two
/// things: that the worker is alive, and what it last said. The repository's
/// own diff carries the rest, which is why the supervisor reads it directly
/// rather than asking the worker what it did.
pub async fn pump_process(feed: Feed, argv: Vec<String>, cwd: PathBuf, stop: Arc<Notify>) {
    let Some((program, arguments)) = argv.split_first() else {
        feed.finish(
            WorkerStatus::Failed,
            false,
            String::new(),
            Some("worker command is empty".to_owned()),
        );
        return;
    };
    note(&feed.events, format!("{} exec {program}", feed.id));

    let mut child = match tokio::process::Command::new(program)
        .args(arguments)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            // The common case is the binary not being installed, and saying so
            // beats a supervisor inferring "stuck" from silence.
            feed.finish(
                WorkerStatus::Failed,
                false,
                String::new(),
                Some(format!("could not start {program}: {error}")),
            );
            return;
        }
    };

    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(tokio::spawn(read_lines(
            BufReader::new(stdout).lines(),
            feed_handle(&feed),
            true,
        )));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(tokio::spawn(read_lines(
            BufReader::new(stderr).lines(),
            feed_handle(&feed),
            false,
        )));
    }

    let stopped;
    let exit = tokio::select! {
        status = child.wait() => {
            stopped = false;
            status
        }
        _ = stop.notified() => {
            stopped = true;
            let _ = child.start_kill();
            child.wait().await
        }
    };
    for reader in readers {
        let _ = reader.await;
    }

    let summary = lock(&feed.workers)
        .iter()
        .find(|worker| worker.id == feed.id)
        .map(|worker| worker.output.as_str().to_owned())
        .unwrap_or_default();
    let (status, success, reason) = match exit {
        _ if stopped => (WorkerStatus::Stopped, false, "cancelled".to_owned()),
        Ok(status) if status.success() => (WorkerStatus::Completed, true, "exit 0".to_owned()),
        Ok(status) => (
            WorkerStatus::Failed,
            false,
            match status.code() {
                Some(code) => format!("exit {code}"),
                None => "killed by signal".to_owned(),
            },
        ),
        Err(error) => (WorkerStatus::Failed, false, error.to_string()),
    };
    feed.finish(status, success, summary, Some(reason));
}

fn feed_handle(feed: &Feed) -> Feed {
    Feed {
        id: feed.id.clone(),
        kind: feed.kind,
        workers: Arc::clone(&feed.workers),
        events: Arc::clone(&feed.events),
        watcher: Arc::clone(&feed.watcher),
        signals: feed.signals.clone(),
    }
}

async fn read_lines<R>(mut lines: tokio::io::Lines<BufReader<R>>, feed: Feed, stdout: bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        with(&feed.workers, &feed.id, |worker| {
            worker.output.push(&format!("{line}\n"));
            if stdout {
                // A CLI has no tool-call event, so a printed step is the
                // closest honest equivalent. Codex's `--json` lines name their
                // own type; anything else counts as one step and no more.
                worker.iterations += 1;
                if let Some(kind) = json_type(&line) {
                    worker.tool_calls += 1;
                    worker.last_tool = Some(kind);
                }
            }
        });
        feed.watcher.worker_text(&feed.id, &format!("{line}\n"));
        let _ = feed.signals.send(Signal::Activity);
    }
}

/// The `type` of a JSONL event line, when the worker emits them.
fn json_type(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    value
        .get("type")
        .and_then(|kind| kind.as_str())
        .map(str::to_owned)
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
pub fn first_line(script: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_runs_foremans_own_command_line() {
        let argv = ExternalAgent::codex().command(
            WorkerKind::Coding,
            Path::new("/tmp/repo"),
            "Add rate limiting.",
        );
        assert_eq!(
            argv,
            vec![
                "codex",
                "exec",
                "--cd",
                "/tmp/repo",
                "--color",
                "never",
                "--json",
                "--sandbox",
                "workspace-write",
                "Add rate limiting.",
            ]
        );
    }

    #[test]
    fn a_codex_verifier_is_given_a_read_only_sandbox() {
        let argv =
            ExternalAgent::codex().command(WorkerKind::Verifier, Path::new("/tmp/repo"), "Check.");
        let sandbox = argv.iter().position(|word| word == "--sandbox").unwrap();
        assert_eq!(argv[sandbox + 1], "read-only");
    }

    #[test]
    fn yolop_runs_its_one_shot_print_interface() {
        let argv =
            ExternalAgent::yolop().command(WorkerKind::Coding, Path::new("/tmp/repo"), "Fix it.");
        assert_eq!(argv, vec!["yolop", "-C", "/tmp/repo", "-p", "Fix it."]);
    }

    #[test]
    fn a_mission_stays_one_argument_however_it_is_written() {
        // No shell sits between the template and the process, so a mission
        // carrying quotes, newlines, or a semicolon is still one argv entry.
        let mission = "Fix \"it\"; then\nrun the tests";
        let argv =
            ExternalAgent::yolop().command(WorkerKind::Coding, Path::new("/tmp/repo"), mission);
        assert_eq!(argv.len(), 5);
        assert_eq!(argv[4], mission);
    }

    #[test]
    fn an_operator_template_needs_somewhere_to_put_the_mission() {
        let agent = ExternalAgent::from_template("mycli --repo {repo} --task {mission}").unwrap();
        assert_eq!(agent.label, "mycli");
        assert_eq!(
            agent.command(WorkerKind::Coding, Path::new("/repo"), "go"),
            vec!["mycli", "--repo", "/repo", "--task", "go"]
        );
        assert!(matches!(
            ExternalAgent::from_template("mycli --repo {repo}"),
            Err(TemplateError::NoMission)
        ));
        assert!(matches!(
            ExternalAgent::from_template("   "),
            Err(TemplateError::Empty)
        ));
    }

    #[test]
    fn a_jsonl_line_names_its_own_step() {
        assert_eq!(
            json_type(r#"{"type":"tool_use","name":"shell"}"#).as_deref(),
            Some("tool_use")
        );
        assert_eq!(json_type("not json at all"), None);
        assert_eq!(json_type(r#"{"no":"type"}"#), None);
    }

    #[test]
    fn a_tool_call_is_summarized_by_its_first_real_line() {
        assert_eq!(
            first_line("\n\n  cat src/rates.py\nls tests\n"),
            "cat src/rates.py"
        );
        assert_eq!(first_line(""), "");
        let clipped = first_line(&"x".repeat(200));
        assert_eq!(clipped.chars().count(), 80);
        assert!(clipped.ends_with('…'));
    }
}
