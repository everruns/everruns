//! A coordinator agent on the Everruns Framework.
//!
//! The person talks to one session. It hands each piece of work to a thread,
//! each thread keeps a checklist and finishes with a summary, and every
//! finished thread wakes the coordinator with an automatic update.
//!
//! [`TeamSim`] is the offline model: a deterministic stand-in that plays both
//! the coordinator and its threads by reading the conversation it is given, so
//! the example and its test run with no key and no network.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use everruns::coordination::{self, Coordination, Thread, ThreadStatus};
use everruns::llm::{Message, MessageRole};
use everruns::{
    Agent, AgentLoopError, BuildError, ChatDriver, LlmCallConfig, LlmResponseStream, LocalConfig,
    Model, Provider, ProviderEndpoint, Session, ToolCall, WorkspacePolicy,
};
use everruns_llmsim::{LlmSimConfig, LlmSimDriver};
use serde_json::{Value, json};

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

/// The offline model for the demo conversation.
pub fn simulated_model() -> Model {
    Model::new("team-sim", Provider::new("team-sim", TeamSim))
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

// =============================================================================
// Offline model
// =============================================================================

/// Plays the coordinator and both threads. Each call reads the newest user
/// message and the tools already called since, so the coordinator and its
/// threads can run concurrently in any order and still get the same script.
pub struct TeamSim;

const BRIEF_MARKER: &str = "New assignment from the coordinator: ";

#[async_trait]
impl ChatDriver for TeamSim {
    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        let (text, calls) = next_move(&messages);
        let mut script = LlmSimConfig::fixed(text);
        if !calls.is_empty() {
            script = script.with_tool_calls(calls);
        }
        LlmSimDriver::new(script)
            .chat_completion_stream(endpoint, messages, config)
            .await
    }
}

fn next_move(messages: &[Message]) -> (String, Vec<ToolCall>) {
    let Some(position) = messages
        .iter()
        .rposition(|message| message.role == MessageRole::User)
    else {
        return ("Hello.".into(), Vec::new());
    };
    let request = messages[position].content_as_text();
    let called: Vec<String> = messages[position + 1..]
        .iter()
        .filter_map(|message| message.tool_calls.as_ref())
        .flatten()
        .map(|call| call.name.clone())
        .collect();

    if let Some(rest) = request.split(BRIEF_MARKER).nth(1) {
        let title = rest.lines().next().unwrap_or("Work").trim().to_string();
        return thread_move(&title, called.len());
    }
    if finished_task(&request).is_some() {
        // Updates that arrive together are answered together.
        let mut finished: Vec<&str> = messages[..=position]
            .iter()
            .rev()
            .take_while(|message| message.role == MessageRole::User)
            .filter_map(|message| finished_task_owned(&message.content_as_text()))
            .collect();
        finished.reverse();
        let verb = if finished.len() == 1 { "is" } else { "are" };
        return (
            format!("{} {verb} ready for review.", finished.join(" and ")),
            Vec::new(),
        );
    }
    if request.contains("Resolve") {
        return match thread_id(messages, "Announcement") {
            Some(id) if called.is_empty() => (
                "Resolving the announcement thread.".into(),
                vec![call("resolve_thread", json!({ "thread_id": id }))],
            ),
            _ => ("Resolved the announcement thread.".into(), Vec::new()),
        };
    }
    if called.is_empty() {
        return (
            "Starting two threads.".into(),
            vec![
                call(
                    "start_thread",
                    json!({
                        "title": "Announcement",
                        "brief": "Write a three-line launch announcement. Done means it names the product, the date, and where to sign up.",
                    }),
                ),
                call(
                    "start_thread",
                    json!({
                        "title": "Pricing FAQ",
                        "brief": "Draft three pricing FAQ entries. Done means each has a question and a one-sentence answer.",
                    }),
                ),
            ],
        );
    }
    (
        "I started two threads, Announcement and Pricing FAQ. I will tell you when each is ready."
            .into(),
        Vec::new(),
    )
}

/// A thread's steps: write its draft, record the checklist, finish, reply.
fn thread_move(title: &str, step: usize) -> (String, Vec<ToolCall>) {
    let (file, draft, steps, summary) = if title == "Announcement" {
        (
            "announcement.md",
            "Everruns Cloud is live on 14 October.\nRun durable agents without running servers.\nSign up at everruns.com/cloud.\n",
            ["Draft three lines", "Check name, date and sign-up link"],
            "Three-line announcement drafted in announcement.md with the launch date and sign-up link.",
        )
    } else {
        (
            "pricing-faq.md",
            "Is there a free tier? Yes, $5 of credits on sign-up.\nHow am I billed? Prepaid credits, charged per run.\nCan I set a cap? Runs stop when credits reach zero.\n",
            ["Draft three questions", "Answer each in one sentence"],
            "Three pricing FAQ entries drafted in pricing-faq.md, one sentence each.",
        )
    };
    match step {
        0 => (
            "Writing the draft.".into(),
            vec![call(
                "write_file",
                json!({ "path": file, "content": draft }),
            )],
        ),
        1 => (
            "Updating the checklist.".into(),
            vec![call(
                "update_checklist",
                json!({ "steps": steps.map(|title| json!({ "title": title, "status": "done" })) }),
            )],
        ),
        2 => (
            "Finishing up.".into(),
            vec![call(
                "complete_assignment",
                json!({
                    "summary": summary,
                    "validation": "Checked against the brief.",
                    "artifacts": [{ "name": file, "type": "file", "path": file }],
                }),
            )],
        ),
        _ => (format!("Done: {summary}"), Vec::new()),
    }
}

/// The task title in an automatic update about finished work.
fn finished_task(text: &str) -> Option<&str> {
    if !text.contains(") finished: succeeded") {
        return None;
    }
    let start = text.find("Task \"")? + "Task \"".len();
    let end = text[start..].find('"')? + start;
    Some(&text[start..end])
}

fn finished_task_owned(text: &str) -> Option<&'static str> {
    // Titles come from this script, so map them back to static strings.
    let title = finished_task(text)?;
    ["Announcement", "Pricing FAQ"]
        .into_iter()
        .find(|known| *known == title)
}

/// A thread id from an earlier `start_thread` result with this title.
fn thread_id(messages: &[Message], title: &str) -> Option<String> {
    messages
        .iter()
        .filter(|message| message.role == MessageRole::Tool)
        .filter_map(|message| serde_json::from_str::<Value>(&message.content_as_text()).ok())
        .find(|result| result["title"] == title)
        .and_then(|result| result["thread_id"].as_str().map(str::to_string))
}

fn call(name: &str, arguments: Value) -> ToolCall {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    ToolCall {
        id: format!("call_{name}_{}", NEXT.fetch_add(1, Ordering::Relaxed)),
        name: name.into(),
        arguments,
    }
}
