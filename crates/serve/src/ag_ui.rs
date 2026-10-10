//! The AG-UI channel: `POST /v1/channels/{agent}/ag-ui` serves an agent to AG-UI
//! clients (CopilotKit, `@ag-ui/client`) as AG-UI 1.0 server-sent events.
//! Requires the `ag-ui` feature.
//!
//! Decisions:
//! - Built in, not a [`Channel`](crate::Channel): that trait answers a
//!   webhook and delivers the reply later, and AG-UI streams the reply in
//!   the response. The run is `everruns::Session::ag_ui_with`, the same
//!   projection the facade and the everruns server use; serve adds routing,
//!   the thread map, and its interrupts.
//! - The path has the shape of the everruns server's channel route
//!   (`/v1/channels/{id}/ag-ui`), so a front end moves between serve and Everruns
//!   by base URL and id alone. Here the id is the agent's name: every
//!   top-level agent is an endpoint, and none is declared separately.
//! - One session per (agent, AG-UI `threadId`), kept in the same thread map
//!   channels use (channel key `ag-ui:{agent}`), so a thread survives a
//!   restart like a Slack thread does. serve keeps its own map rather than
//!   `everruns::ag_ui::AgUiThreads` because its sessions live in its own
//!   catalog; it shares the seeding: a thread's first run records the
//!   input's earlier messages as the new session's history.
//! - Interrupts are serve's own pending approvals and questions, read and
//!   answered through `everruns::ag_ui::InterruptSource`. One responder
//!   serves both APIs: a request an AG-UI run interrupted on can be answered
//!   by `/question-answers` or `/approvals` too, and the reverse.
//! - The trusted projection policy (reasoning, usage and runtime errors
//!   visible): the developer owns both ends of a serve app. A public
//!   deployment puts it behind its own auth, as with every serve route.
//! - SSE framing is the facade's `sse_response`, which matches the server's
//!   AG-UI route: unnamed `data:` events and a `keepalive` comment every 15
//!   seconds. The facade's `AgUiHandler` is not used: serve resolves threads
//!   and authorizes requests its own way.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use everruns::SessionId;
use everruns::ag_ui::{
    AgUiError, AgUiOptions, Interrupt, InterruptSource, ResumeEntry, ResumeOutcome, RunAgentInput,
    approval_decision, approval_interrupt, question_interrupt, question_outcome, sse_response,
};
use everruns::approval::ApprovalDecision;
use everruns::ask_user::Outcome;
use tokio::sync::broadcast;

use crate::host::{ApiError, Host};

/// The route of `agent`'s AG-UI endpoint.
pub(crate) fn route(agent: &str) -> String {
    format!("/v1/channels/{agent}/ag-ui")
}

/// `POST /v1/channels/{agent}/ag-ui`: one AG-UI run. A malformed body or input the
/// run cannot use is a 400 problem; an unknown agent a 404.
pub(crate) async fn run(
    State(host): State<Arc<Host>>,
    Path(agent): Path<String>,
    body: Bytes,
) -> Response {
    respond(&host, &agent, &body).await
}

/// One AG-UI run for `agent` from a raw `RunAgentInput` body: the stream, or
/// the problem the route would answer. Shared with [`crate::Server::ag_ui`].
pub(crate) async fn respond(host: &Arc<Host>, agent: &str, body: &[u8]) -> Response {
    match start(host, agent, body).await {
        Ok(response) => response,
        Err(err) => crate::server::Failure::from(err).into_response(),
    }
}

async fn start(host: &Arc<Host>, agent: &str, body: &[u8]) -> crate::Result<Response> {
    let input: RunAgentInput = serde_json::from_slice(body)
        .map_err(|err| ApiError::BadRequest(format!("invalid RunAgentInput: {err}")))?;
    if input.thread_id.trim().is_empty() {
        return Err(ApiError::BadRequest("threadId is required".into()).into());
    }
    let (session, created) = thread_session(host, agent, &input.thread_id).await?;
    // A thread's first run carries a client's earlier messages into the new
    // session, as the server's endpoint does.
    let options = AgUiOptions::new()
        .interrupts(HostInterrupts { host: host.clone() })
        .seed_history(created);
    let run = host.ag_ui(&session, input, options).await?;
    Ok(sse_response(run))
}

/// The session behind `agent`'s AG-UI thread, created on its first run, and
/// whether this call created it.
async fn thread_session(host: &Host, agent: &str, thread: &str) -> crate::Result<(String, bool)> {
    if host.app.agent(agent).is_none_or(|entry| entry.sub) {
        return Err(ApiError::NotFound(format!("agent {agent}")).into());
    }
    let conversation =
        crate::channels::conversation(host, "ag-ui", agent, thread, "AG-UI thread").await?;
    Ok((conversation.session_id, conversation.created))
}

/// serve's pending approvals and questions, as AG-UI interrupts.
struct HostInterrupts {
    host: Arc<Host>,
}

/// One open request, with what answers it.
enum Open {
    Approval(Interrupt),
    Question(Interrupt, Vec<everruns::ask_user::Question>),
}

impl Open {
    fn interrupt(&self) -> &Interrupt {
        match self {
            Open::Approval(interrupt) | Open::Question(interrupt, _) => interrupt,
        }
    }
}

enum Answer {
    Approval(ApprovalDecision),
    Question(Outcome),
}

impl HostInterrupts {
    /// Approvals first, then questions, each by tool call id: the order the
    /// session view lists them in.
    fn open(&self, session_id: SessionId) -> Vec<Open> {
        let id = session_id.to_string();
        let approvals = self.host.pending_approvals(&id).into_iter().map(|view| {
            Open::Approval(approval_interrupt(&everruns::ToolCall {
                id: view.tool_call_id,
                name: view.tool_name,
                arguments: view.arguments,
            }))
        });
        let questions = self.host.pending_questions(&id).into_iter().map(|view| {
            Open::Question(
                question_interrupt(&view.tool_call_id, &view.questions),
                view.questions,
            )
        });
        approvals.chain(questions).collect()
    }
}

impl InterruptSource for HostInterrupts {
    fn interrupts(&self, session_id: SessionId) -> Vec<Interrupt> {
        self.open(session_id)
            .iter()
            .map(|open| open.interrupt().clone())
            .collect()
    }

    fn subscribe(&self) -> broadcast::Receiver<SessionId> {
        self.host.parked_on.subscribe()
    }

    fn resume(
        &self,
        session_id: SessionId,
        entries: &[ResumeEntry],
    ) -> Result<ResumeOutcome, AgUiError> {
        let open = self.open(session_id);
        if open.is_empty() {
            return Ok(ResumeOutcome::NothingOpen);
        }
        let entry_for = |id: &str| entries.iter().find(|entry| entry.interrupt_id == id);
        if open
            .iter()
            .any(|open| entry_for(&open.interrupt().id).is_none())
        {
            return Ok(ResumeOutcome::StillOpen(
                open.iter().map(|open| open.interrupt().clone()).collect(),
            ));
        }
        // Validate every entry before applying any.
        let mut answers = Vec::with_capacity(open.len());
        for open in &open {
            let id = &open.interrupt().id;
            let Some(entry) = entry_for(id) else {
                continue;
            };
            let answer = match open {
                Open::Approval(_) => Answer::Approval(approval_decision(entry)?),
                Open::Question(_, questions) => {
                    Answer::Question(question_outcome(entry, id, questions)?)
                }
            };
            answers.push((id.clone(), answer));
        }
        let id = session_id.to_string();
        for (tool_call_id, answer) in answers {
            // Fails only when the request went away meanwhile (cancel, or an
            // answer through the `/v1` API); the turn has moved on then.
            let _ = match answer {
                Answer::Approval(decision) => {
                    self.host.decide_approval(&id, &tool_call_id, decision)
                }
                Answer::Question(outcome) => self.host.answer_questions(
                    &id,
                    Some(&tool_call_id),
                    outcome.status,
                    outcome.answers,
                ),
            };
        }
        Ok(ResumeOutcome::Resumed)
    }
}
