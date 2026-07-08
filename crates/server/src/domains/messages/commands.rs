use super::queries as q;
use crate::api::messages::Message;
use crate::domains::common::*;
use crate::domains::messages::CreateMessageContext;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMessage {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub message: crate::api::messages::InputMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<everruns_core::Controls>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Free-form metadata attached to this resource.
    pub metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Free-form tags attached to this resource.
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_actor: Option<everruns_core::ExternalActor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl Command for CreateMessage {
    type Output = Message;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_message",
            category: "messages",
            description: "Create a user message in a session and start the next run.",
            method: "POST",
            path: "/v1/sessions/{session_id}/messages",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Message, CommandError> {
        let mut req = crate::api::messages::CreateMessageRequest {
            message: self.message,
            controls: self.controls,
            metadata: self.metadata,
            tags: self.tags,
            external_actor: self.external_actor,
        };
        req.controls = crate::api::validation::normalize_controls_locale(req.controls)
            .map_err(|_| CommandError::bad_request("Invalid message controls"))?;

        let session_id = q::parse_session_id(&self.session_id)?;
        let session = q::session_service(ctx)?
            .get(&ctx.caller, session_id.uuid(), None)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))?;

        q::message_service(ctx)?
            .create(
                CreateMessageContext {
                    org_id: ctx.org_id(),
                    user_id: ctx.caller.user_id,
                    harness_id: session.harness_id.uuid(),
                    agent_id: session.agent_id.map(|id| id.uuid()),
                    session_id: session_id.uuid(),
                    event_metadata: None,
                    request_id: self.request_id,
                },
                req,
            )
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateMessage>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListMessages {
    /// Session's prefixed public identifier.
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Maximum number of items returned in this page.
    pub limit: Option<i32>,
}

impl Command for ListMessages {
    type Output = Vec<Message>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_messages",
            category: "messages",
            description: "List materialized messages in a session, optionally limited to the most recent N.",
            method: "GET",
            path: "/v1/sessions/{session_id}/messages",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Message>, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::session_service(ctx)?
            .get(&ctx.caller, session_id.uuid(), None)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))?;

        q::message_service(ctx)?
            .list_limited(session_id.uuid(), self.limit)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ListMessages>() }

#[derive(Debug, Serialize)]
pub struct ExportSessionJsonl {
    pub body: String,
}

/// Output format for session export.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionExportFormat {
    /// One materialized message per line (`application/x-ndjson`).
    #[default]
    Jsonl,
    /// A single ATIF trajectory document folded from the session's event log
    /// (`application/json`). See `specs/atif-adoption.md`.
    Atif,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExportSessionMessages {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Output format (defaults to `jsonl`).
    #[serde(default)]
    pub format: SessionExportFormat,
}

impl Command for ExportSessionMessages {
    type Output = ExportSessionJsonl;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "export_session_messages",
            category: "messages",
            description: "Export session messages as JSONL.",
            method: "GET",
            path: "/v1/sessions/{session_id}/export",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<ExportSessionJsonl, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::session_service(ctx)?
            .get(&ctx.caller, session_id.uuid(), None)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))?;

        if self.format == SessionExportFormat::Atif {
            // Fold the full event log into one ATIF trajectory document.
            // Secret scrubbing is always applied by the ATIF builder.
            let event_service = crate::services::EventService::new(
                ctx.db.clone(),
                crate::event_delivery::EventDelivery::in_memory(),
            );
            let events = event_service
                .list(session_id.uuid(), None, None, &[], &[], None, None)
                .await
                .map_err(classify_anyhow)?;
            let trajectory = crate::atif::build_trajectory(
                Some(&session_id.to_string()),
                &events,
                serde_json::Map::new(),
                crate::atif::AtifOptions::default(),
            );
            let body =
                serde_json::to_string(&trajectory).map_err(|e| CommandError::internal(e.into()))?;
            return Ok(ExportSessionJsonl { body });
        }

        let messages = q::message_service(ctx)?
            .list(session_id.uuid())
            .await
            .map_err(classify_anyhow)?;

        let mut body = String::new();
        for message in &messages {
            let line =
                serde_json::to_string(message).map_err(|e| CommandError::internal(e.into()))?;
            body.push_str(&line);
            body.push('\n');
        }

        Ok(ExportSessionJsonl { body })
    }
}

inventory::submit! { CommandDescriptor::of::<ExportSessionMessages>() }
