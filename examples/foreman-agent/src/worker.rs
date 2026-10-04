//! Coding backends and event pumps. Every backend shares a read-only Framework verifier.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use everruns::{Agent, Engine, SessionEventKind, TurnHandle, TurnStopReason};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Notify, mpsc};

use crate::factory::{Signal, Watcher, lock, note, with};
use crate::observation::{WorkerKind, WorkerRecord, WorkerStatus};

/// Non-secret host settings needed to locate and run an installed CLI.
const OPERATING_ENVIRONMENT: &[&str] = &[
    "HOME",
    "LANG",
    "LC_ALL",
    "LOGNAME",
    "PATH",
    "SHELL",
    "TERM",
    "TMPDIR",
    "USER",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
];
const CODEX_CREDENTIALS: &[&str] = &["OPENAI_API_KEY"];
const YOLOP_CREDENTIALS: &[&str] = &["ANTHROPIC_API_KEY"];

/// The `{repo}` placeholder in an external agent's command template.
const REPO: &str = "{repo}";
/// The `{mission}` placeholder in an external agent's command template.
const MISSION: &str = "{mission}";

/// One coding backend and an independent Framework verifier.
pub struct Crew {
    pub model: String,
    pub coding: CodingWorker,
    pub verifier: Agent,
    pub engine: Engine,
}

pub enum CodingWorker {
    Session(Box<Agent>),
    External(ExternalAgent),
}

impl Crew {
    pub fn sessions(model: impl Into<String>, worker: Agent, verifier: Agent) -> Self {
        Self {
            model: model.into(),
            coding: CodingWorker::Session(Box::new(worker)),
            verifier,
            engine: Engine::new(),
        }
    }

    pub fn external(
        worker: ExternalAgent,
        verifier_model: impl Into<String>,
        verifier: Agent,
    ) -> Self {
        Self {
            model: verifier_model.into(),
            coding: CodingWorker::External(worker),
            verifier,
            engine: Engine::new(),
        }
    }

    pub fn label(&self) -> String {
        match &self.coding {
            CodingWorker::Session(_) => format!("{} on Bashkit", self.model),
            CodingWorker::External(agent) => format!("{} (external CLI)", agent.label),
        }
    }
}

/// An external coding agent, as the command line that starts one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalAgent {
    /// `codex`, `yolop`, or whatever the operator named.
    pub label: String,
    /// Coding argv; `{repo}` and `{mission}` are substituted whole.
    pub coding: Vec<String>,
    /// Credential variables this known worker is permitted to inherit.
    pub(crate) credential_environment: &'static [&'static str],
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
    pub fn codex() -> Self {
        let base = ["codex", "exec", "--cd", REPO, "--color", "never", "--json"];
        Self {
            label: "codex".to_owned(),
            coding: argv(&base, "workspace-write"),
            credential_environment: CODEX_CREDENTIALS,
        }
    }

    /// yolop, on its one-shot `--print` interface.
    pub fn yolop() -> Self {
        let argv: Vec<String> = ["yolop", "-C", REPO, "-p", MISSION]
            .iter()
            .map(|word| (*word).to_owned())
            .collect();
        Self {
            label: "yolop".to_owned(),
            coding: argv.clone(),
            credential_environment: YOLOP_CREDENTIALS,
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
            // An arbitrary command must opt into credentials through another
            // mechanism; Foreman cannot safely infer which secrets it needs.
            credential_environment: &[],
        })
    }

    /// The command line for one worker, with the placeholders filled in.
    pub fn command(&self, repo: &Path, mission: &str) -> Vec<String> {
        self.coding
            .iter()
            .map(|word| match word.as_str() {
                REPO => repo.display().to_string(),
                MISSION => mission.to_owned(),
                other => other.to_owned(),
            })
            .collect()
    }

    /// The narrow credential boundary for this worker preset.
    pub fn credential_environment(&self) -> &'static [&'static str] {
        self.credential_environment
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
            } => {
                // Streaming stdout omits exit status and structured failure details.
                // Keep the final tool result in the same bounded evidence tail.
                let payload = event.canonical_json();
                let data = &payload["data"];
                let result = data.get("result").or_else(|| data.get("error"));
                with(&feed.workers, &feed.id, |worker| {
                    worker.output.push(&format!(
                        "\n{tool_name} completed (success={success}): {}\n",
                        result.map_or_else(String::new, serde_json::Value::to_string)
                    ));
                });
                note(
                    &feed.events,
                    format!(
                        "{} tool {tool_name} {}",
                        feed.id,
                        if *success { "ok" } else { "failed" }
                    ),
                );
            }
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
pub async fn pump_process(
    feed: Feed,
    argv: Vec<String>,
    cwd: PathBuf,
    credential_environment: &'static [&'static str],
    stop: Arc<Notify>,
) {
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

    let mut command = sanitized_command(program, arguments, &cwd, credential_environment);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = match command
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

    let mut group = ProcessGroup(child.id());
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
            group.stop();
            let _ = child.start_kill();
            child.wait().await
        }
    };
    // Descendants can hold stdout open even after the parent exits.
    group.stop();
    for mut reader in readers {
        if tokio::time::timeout(std::time::Duration::from_secs(1), &mut reader)
            .await
            .is_err()
        {
            reader.abort();
        }
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

/// Kill the process tree on cancellation, normal exit, or pump shutdown.
struct ProcessGroup(Option<u32>);

impl ProcessGroup {
    fn stop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0.take() {
            // This child was spawned as its own process group; never signal the host's group.
            unsafe {
                libc::killpg(pid as i32, libc::SIGKILL);
            }
        }
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Build a child process without crossing Foreman's credential boundary.
fn sanitized_command(
    program: &str,
    arguments: &[String],
    cwd: &Path,
    credential_environment: &[&str],
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    // THREAT[TM-LLM-042]: workers must never inherit Foreman's supervisor key
    // or ambient host credentials.
    command.args(arguments).current_dir(cwd).env_clear();
    for name in OPERATING_ENVIRONMENT.iter().chain(credential_environment) {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
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
#[path = "../tests/unit/worker.rs"]
mod tests;
