//! A coordinator agent on the Everruns Framework.
//!
//! The person talks to one session. It hands each piece of work to a thread,
//! each thread keeps a checklist and finishes with a summary, and every
//! finished thread wakes the coordinator with an automatic update.
//!
//! Threads ([`Threads`]) give the agent `start_thread`, `message_thread`,
//! `list_threads`, `get_thread` and `close_thread`; [`Coordinator`] makes it
//! route work instead of doing it. A thread runs the coordinator itself, or
//! the `copywriter` agent added with
//! [`AgentBuilder::threads_with`](everruns::AgentBuilder::threads_with).
//!
//! The example runs on a real model (`gpt-6.2-sol`). The tests drive
//! [`coordinator`] with a scripted stand-in that lives only in `tests/`.

use std::time::Duration;

use everruns::providers::openai::{OpenAI, OpenAIError};
use everruns::threads::{self, Coordinator, Thread, ThreadStatus, Threads};
use everruns::{Agent, AgentLoopError, BuildError, LocalConfig, Model, Session, WorkspacePolicy};

/// The model the example runs on.
pub const MODEL: &str = "gpt-6.2-sol";

pub const INSTRUCTIONS: &str = "\
You coordinate a small launch team. Start one thread per piece of work. Public \
copy such as announcements goes to the copywriter agent; run everything else \
yourself. When an automatic update says a thread finished, tell the person in \
one line what it produced. When the person accepts a thread's work, close it.";

/// The copywriter's instructions. It runs announcement threads.
pub const COPYWRITER_INSTRUCTIONS: &str = "\
You write short, concrete launch copy. Write the draft to the file the brief \
names, keep your checklist current, and finish with a one-line summary.";

/// The person's first request in the demo conversation.
pub const LAUNCH_REQUEST: &str = "Plan the launch: write the announcement and draft a pricing FAQ.";
/// The person's follow-up once both threads are ready for review.
pub const CLOSE_REQUEST: &str = "The announcement looks good. Close that thread.";

/// Build the coordinator: Threads plus Coordinator, with a copywriter its
/// threads can run. Threads share `workspace`; session state lives in
/// `workspace/.everruns`.
#[allow(deprecated)] // Legacy facade lane until EVE-1298 moves this onto Engine hosting.
pub fn coordinator(model: Model, workspace: &std::path::Path) -> Result<Agent, BuildError> {
    let copywriter = Agent::builder()
        .name("copywriter")
        .description("Writes announcements and other public launch copy.")
        .instructions(COPYWRITER_INSTRUCTIONS)
        .model(model.clone())
        .workspace_policy(WorkspacePolicy::read_write())
        .build()?;
    Agent::builder()
        .name("launch-coordinator")
        .instructions(INSTRUCTIONS)
        .model(model)
        .capability(Threads::new().max_open(4))
        .capability(Coordinator)
        .threads_with(copywriter)
        .max_iterations(8)
        // Threads write their drafts; the default policy is read-only.
        .workspace_policy(WorkspacePolicy::read_write())
        .local(LocalConfig::new(workspace.join(".everruns")).workspace(workspace))
        .build()
}

/// OpenAI's `gpt-6.2-sol`, keyed by `OPENAI_API_KEY`.
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
        let threads = threads::list(session).await?;
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
