use super::queries as q;
use crate::domains::common::*;
use everruns_core::SessionTask;
use everruns_core::session_task::{
    NewTaskMessage, SessionTaskFilter, SessionTaskRegistry, SessionTaskState, TaskMessage,
    TaskMessagePart,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

fn registry_err(e: everruns_core::AgentLoopError) -> CommandError {
    CommandError::internal(anyhow::anyhow!(e))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSessionTasks {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Optional state filter (queued, running, awaiting_input, succeeded, failed, canceled).
    #[serde(default)]
    pub state: Option<String>,
    /// Optional kind filter (subagent, external_agent, background_tool, ...).
    #[serde(default)]
    pub kind: Option<String>,
}

impl Command for ListSessionTasks {
    type Output = Vec<SessionTask>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_session_tasks",
            category: "session_tasks",
            description: "List background tasks owned by a session.",
            method: "GET",
            path: "/v1/sessions/{session_id}/tasks",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SessionTask>, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        if !q::session_in_org(&ctx.db, ctx.org_id(), session_id)
            .await
            .map_err(classify_anyhow)?
        {
            return Err(CommandError::not_found("Session"));
        }
        let state = match self.state.as_deref().filter(|s| !s.is_empty()) {
            Some(raw) => Some(SessionTaskState::parse(raw).ok_or_else(|| {
                CommandError::bad_request(format!(
                    "Unknown state filter \"{raw}\". Valid states: queued, running, \
                     awaiting_input, succeeded, failed, canceled."
                ))
            })?),
            None => None,
        };
        let filter = SessionTaskFilter {
            kind: self.kind,
            state,
        };
        q::registry_for_ctx(ctx)
            .list(session_id, Some(&filter))
            .await
            .map_err(registry_err)
    }
}

inventory::submit! { CommandDescriptor::of::<ListSessionTasks>() }

/// Task snapshot plus the recent message thread.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionTaskDetail {
    pub task: SessionTask,
    pub messages: Vec<TaskMessage>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetSessionTask {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
}

impl Command for GetSessionTask {
    type Output = SessionTaskDetail;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session_task",
            category: "session_tasks",
            description: "Get one session task with its recent message thread.",
            method: "GET",
            path: "/v1/sessions/{session_id}/tasks/{task_id}",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionTaskDetail, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        let task = q::get_task_in_org(ctx, ctx.org_id(), session_id, &self.task_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session task"))?;
        let messages = q::registry_for_ctx(ctx)
            .list_messages(session_id, &self.task_id, Some(50))
            .await
            .map_err(registry_err)?;
        Ok(SessionTaskDetail { task, messages })
    }
}

inventory::submit! { CommandDescriptor::of::<GetSessionTask>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct PostSessionTaskMessage {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
    /// Plain-text message (alternative to `content`).
    #[serde(default)]
    pub text: Option<String>,
    /// Structured message parts (alternative to `text`).
    #[serde(default)]
    pub content: Option<Vec<TaskMessagePart>>,
    /// Input request ID this message answers, when applicable.
    #[serde(default)]
    pub in_reply_to: Option<String>,
}

impl Command for PostSessionTaskMessage {
    type Output = TaskMessage;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "post_session_task_message",
            category: "session_tasks",
            description: "Send an inbound message to a session task.",
            method: "POST",
            path: "/v1/sessions/{session_id}/tasks/{task_id}/messages",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<TaskMessage, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::get_task_in_org(ctx, ctx.org_id(), session_id, &self.task_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session task"))?;

        let content = match (self.content, self.text) {
            (Some(content), _) if !content.is_empty() => content,
            (_, Some(text)) if !text.trim().is_empty() => vec![TaskMessagePart::text(text)],
            _ => {
                return Err(CommandError::bad_request(
                    "Message requires non-empty `content` parts or `text`",
                ));
            }
        };
        let mut message = NewTaskMessage {
            direction: everruns_core::session_task::TaskMessageDirection::Inbound,
            content,
            in_reply_to: self.in_reply_to,
        };
        message.in_reply_to = message.in_reply_to.filter(|s| !s.is_empty());

        q::registry_for_ctx(ctx)
            .record_message(session_id, &self.task_id, message)
            .await
            .map_err(registry_err)
    }
}

inventory::submit! { CommandDescriptor::of::<PostSessionTaskMessage>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CancelSessionTask {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Task's prefixed public identifier.
    pub task_id: String,
}

impl Command for CancelSessionTask {
    type Output = SessionTask;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "cancel_session_task",
            category: "session_tasks",
            description: "Request cooperative cancellation of a session task.",
            method: "POST",
            path: "/v1/sessions/{session_id}/tasks/{task_id}/cancel",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionTask, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        if !q::session_in_org(&ctx.db, ctx.org_id(), session_id)
            .await
            .map_err(classify_anyhow)?
        {
            return Err(CommandError::not_found("Session"));
        }
        q::registry_for_ctx(ctx)
            .request_cancel(session_id, &self.task_id)
            .await
            .map_err(registry_err)?
            .ok_or_else(|| CommandError::not_found("Session task"))
    }
}

inventory::submit! { CommandDescriptor::of::<CancelSessionTask>() }
