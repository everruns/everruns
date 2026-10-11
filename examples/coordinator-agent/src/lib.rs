//! A coordinator agent on the Everruns Framework.
//!
//! The person talks to one session. It hands each piece of work to a thread,
//! each thread keeps a checklist and finishes with a summary, and every
//! finished thread wakes the coordinator with an automatic update.
//!
//! The example runs on a real model (`gpt-6.1-sol`). The tests drive
//! [`coordinator`] with a scripted stand-in that lives only in `tests/`.

use std::time::Duration;

use everruns::coordination::{self, Coordination, Thread, ThreadStatus};
use everruns::providers::openai::{OpenAI, OpenAIError};
use everruns::{Agent, AgentLoopError, BuildError, LocalConfig, Model, Session, WorkspacePolicy};

/// The model the example runs on.
pub const MODEL: &str = "gpt-6.1-sol";

pub const INSTRUCTIONS: &str = "\
You coordinate a small launch team. Do not do focused work yourself: start one \
thread per piece of work with start_thread, giving it a short title and a brief \
that says what done looks like. When an automatic update says a thread finished, \
tell the person in one line what it produced. When the person asks you to \
resolve a thread, call resolve_thread with its id (list_threads shows ids). \
Inside a thread, keep your checklist current with update_checklist and finish \
with complete_assignment.";

/// The person's first request in the demo conversation.
pub const LAUNCH_REQUEST: &str = "Plan the launch: write the announcement and draft a pricing FAQ.";
/// The person's follow-up once both threads are ready for review.
pub const RESOLVE_REQUEST: &str = "The announcement looks good. Resolve that thread.";

/// Build the coordinator: an agent with the coordination capability whose
/// threads share `workspace`. Session state lives in `workspace/.everruns`.
pub fn coordinator(model: Model, workspace: &std::path::Path) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("launch-coordinator")
        .instructions(INSTRUCTIONS)
        .model(model)
        .capability(Coordination::new().max_active_threads(4))
        .max_iterations(8)
        // Threads write their drafts; the default policy is read-only.
        .workspace_policy(WorkspacePolicy::read_write())
        .local(LocalConfig::new(workspace.join(".everruns")).workspace(workspace))
        .build()
}

/// OpenAI's `gpt-6.1-sol`, keyed by `OPENAI_API_KEY`.
pub fn openai_model() -> Result<Model, OpenAIError> {
    Ok(Model::new(MODEL, OpenAI::from_env()?))
}

/// Wait until `done` holds for the coordinator's threads, or `timeout` passes.
pub async fn wait_for_threads(
    session: &Session,
    timeout: Duration,
    done: impl Fn(&[Thread]) -> bool,
) -> Result<Vec<Thread>, AgentLoopError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let threads = coordination::threads(session).await?;
        if done(&threads) || tokio::time::Instant::now() >= deadline {
            return Ok(threads);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The coordinator's replies so far.
pub async fn replies(session: &Session) -> Result<Vec<String>, everruns::HistoryError> {
    Ok(session
        .history()
        .page()
        .await?
        .iter()
        .filter(|message| message.role == everruns::MessageRole::Agent)
        .map(|message| message.text())
        .filter(|text| !text.is_empty())
        .collect())
}

/// Wait for the coordinator to reply beyond its first `seen` replies, without
/// the person saying anything, or for `timeout` to pass.
pub async fn wait_for_new_reply(
    session: &Session,
    seen: usize,
    timeout: Duration,
) -> Result<Option<String>, everruns::HistoryError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let replies = replies(session).await?;
        if replies.len() > seen {
            return Ok(replies.last().cloned());
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// True once every thread finished its work.
pub fn all_ready(threads: &[Thread]) -> bool {
    !threads.is_empty()
        && threads
            .iter()
            .all(|thread| thread.status == ThreadStatus::ReadyForReview)
}

/// The board the person sees: one line per thread.
pub fn render_board(threads: &[Thread]) -> String {
    threads
        .iter()
        .map(|thread| {
            let done = thread
                .checklist
                .iter()
                .filter(|(_, status)| status == "done" || status == "skipped")
                .count();
            let mut line = format!(
                "  {:<16} {:<14} {done}/{} steps",
                format!("{:?}", thread.status),
                thread.title,
                thread.checklist.len()
            );
            if let Some(summary) = thread.summary.as_deref().and_then(|s| s.lines().next()) {
                line.push_str(&format!("  {summary}"));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}
