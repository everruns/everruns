use super::queries as q;
use super::types::{
    CancelStatus, CancelTurnResponse, CreateSessionRequest, GetOrCreateChatSessionRequest,
    SessionStatsResponse, UpdateSessionRequest,
};
use crate::domains::common::*;
use everruns_core::events::{EventContext, EventRequest, InputMessageData, TurnCancelledData};
use everruns_core::typed_id::{AgentId, MessageId, TurnId};
use everruns_core::{ANONYMOUS_USER_ID, Message, Session};
use serde::Deserialize;
use utoipa::ToSchema;

fn validation_error(
    error: (
        axum::http::StatusCode,
        axum::Json<crate::api::common::ErrorResponse>,
    ),
) -> CommandError {
    match error.0 {
        axum::http::StatusCode::NOT_FOUND => CommandError::NotFound(error.1.0.error),
        _ => CommandError::bad_request(error.1.0.error),
    }
}

fn limit_validation_error(_: crate::api::validation::ValidationError) -> CommandError {
    CommandError::bad_request(crate::api::validation::VALIDATION_ERROR_MESSAGE)
}

#[derive(Debug, Deserialize)]
pub struct CreateSession(pub CreateSessionRequest);

impl CommandSchema for CreateSession {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateSessionRequest>()
    }
}

impl Command for CreateSession {
    type Output = Session;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_session",
            category: "sessions",
            description: "Create a new session. Optionally assign an agent and harness.",
            method: "POST",
            path: "/v1/sessions",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Session, CommandError> {
        let mut req = self.0;
        req.locale =
            crate::api::validation::normalize_locale(req.locale).map_err(limit_validation_error)?;

        if req.harness_id.is_some() && req.harness_name.is_some() {
            return Err(CommandError::bad_request(
                "Cannot specify both harness_id and harness_name",
            ));
        }

        if let Some(name) = req.harness_name.clone() {
            crate::api::validation::validate_harness_name(&name).map_err(validation_error)?;
            if name == "default" {
                let settings = ctx
                    .db
                    .get_organization_settings(ctx.org_id())
                    .await
                    .map_err(classify_anyhow)?;
                req.harness_id = Some(settings.and_then(|row| row.default_harness_id).ok_or_else(
                    || {
                        CommandError::NotFound(
                            "Default harness not configured for this organization".to_string(),
                        )
                    },
                )?);
            } else {
                let row = ctx
                    .db
                    .get_harness_by_name(ctx.org_id(), &name)
                    .await
                    .map_err(classify_anyhow)?
                    .ok_or_else(|| CommandError::not_found("Harness"))?;
                req.harness_id = Some(row.id);
            }
        }

        let harness_id = q::resolve_session_harness_id(
            &ctx.db,
            ctx.org_id(),
            req.harness_id,
            ctx.fallback_harness_name.as_deref(),
        )
        .await
        .map_err(classify_anyhow)?;
        req.harness_id = Some(harness_id);

        ctx.db
            .get_harness(ctx.org_id(), harness_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Harness"))?;

        if let Some(model_id) = req.model_id {
            ctx.db
                .get_llm_model(ctx.org_id(), model_id.uuid())
                .await
                .map_err(classify_anyhow)?
                .ok_or_else(|| CommandError::not_found("Model"))?;
        }

        let (agent_internal_id, agent_public_id) = if let Some(agent_id) = req.agent_id {
            let row = ctx
                .db
                .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
                .await
                .map_err(classify_anyhow)?
                .ok_or_else(|| CommandError::not_found("Agent"))?;
            let public_id: AgentId = row
                .public_id
                .parse()
                .unwrap_or_else(|_| AgentId::from_uuid(row.id.uuid()));
            (Some(row.id.uuid()), Some(public_id))
        } else {
            (None, None)
        };

        if let Some(prompt) = req.system_prompt.as_ref() {
            crate::api::validation::validate_agent_system_prompt(prompt)
                .map_err(limit_validation_error)?;
        }
        if !req.initial_files.is_empty() {
            crate::api::validation::validate_initial_files(&req.initial_files)
                .map_err(limit_validation_error)?;
        }

        q::session_service(ctx)?
            .create(
                &ctx.caller,
                harness_id.uuid(),
                agent_internal_id,
                agent_public_id,
                req,
            )
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateSession>() }

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListSessions {
    pub agent_id: Option<AgentId>,
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub offset: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub limit: Option<u32>,
}

impl Command for ListSessions {
    type Output = Paginated<Session>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_sessions",
            category: "sessions",
            description: "List sessions. Filter by agent_id, search by title. Supports pagination (limit/offset).",
            method: "GET",
            path: "/v1/sessions",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Paginated<Session>, CommandError> {
        let pagination = pagination(self.offset, self.limit);
        let agent_internal_id = if let Some(agent_id) = self.agent_id {
            let row = ctx
                .db
                .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
                .await
                .map_err(classify_anyhow)?;
            match row {
                Some(row) => Some(row.id.uuid()),
                None => {
                    return Ok(Paginated {
                        data: vec![],
                        total: 0,
                        offset: pagination.offset,
                        limit: pagination.limit,
                    });
                }
            }
        } else {
            None
        };
        let (sessions, total) = q::session_service(ctx)?
            .list(
                &ctx.caller,
                agent_internal_id,
                ctx.caller.user_id,
                self.search.as_deref(),
                crate::api::common::Pagination::new(pagination.offset, pagination.limit),
            )
            .await
            .map_err(classify_anyhow)?;

        Ok(Paginated {
            data: sessions,
            total,
            offset: pagination.offset,
            limit: pagination.limit,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<ListSessions>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetSession {
    pub session_id: String,
}

impl Command for GetSession {
    type Output = Session;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session",
            category: "sessions",
            description: "Get session details including status, agent, harness, and model.",
            method: "GET",
            path: "/v1/sessions/{session_id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Session, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::get_session(ctx, session_id, ctx.caller.user_id).await
    }
}

inventory::submit! { CommandDescriptor::of::<GetSession>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSessionCmd {
    pub session_id: String,
    #[serde(flatten)]
    pub req: UpdateSessionRequest,
}

impl Command for UpdateSessionCmd {
    type Output = Session;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_session",
            category: "sessions",
            description: "Update session title, tags, or locale.",
            method: "PATCH",
            path: "/v1/sessions/{session_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Session, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        let mut req = self.req;
        req.locale =
            crate::api::validation::normalize_locale(req.locale).map_err(limit_validation_error)?;
        q::session_service(ctx)?
            .update(&ctx.caller, session_id.uuid(), req)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateSessionCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteSession {
    pub session_id: String,
}

impl Command for DeleteSession {
    type Output = bool;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_session",
            category: "sessions",
            description: "Delete a session.",
            method: "DELETE",
            path: "/v1/sessions/{session_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::session_service(ctx)?
            .delete(&ctx.caller, session_id.uuid())
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteSession>() }

#[derive(Debug, Deserialize)]
pub struct GetOrCreateChatSession {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

impl CommandSchema for GetOrCreateChatSession {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<GetOrCreateChatSessionRequest>()
    }
}

impl Command for GetOrCreateChatSession {
    type Output = Session;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_or_create_chat_session",
            category: "sessions",
            description: "Get or create the user's global chat session.",
            method: "POST",
            path: "/v1/sessions/chat",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Session, CommandError> {
        let locale = crate::api::validation::normalize_locale(self.locale)
            .map_err(limit_validation_error)?;
        let user_id = ctx.caller.user_id.unwrap_or(ANONYMOUS_USER_ID);
        let chat_harness_name = ctx.chat_harness_name.clone().ok_or_else(|| {
            CommandError::NotFound("Global chat is not configured for this platform".to_string())
        })?;
        let chat_harness_id =
            q::resolve_named_built_in_harness_id(&ctx.db, ctx.org_id(), &chat_harness_name)
                .await
                .map_err(classify_anyhow)?;
        let title = ctx
            .chat_session_title
            .as_deref()
            .unwrap_or(chat_harness_name.as_str());

        q::session_service(ctx)?
            .get_or_create_chat_session(&ctx.caller, user_id, chat_harness_id.uuid(), title, locale)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<GetOrCreateChatSession>() }

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct GetSessionStats;

impl Command for GetSessionStats {
    type Output = SessionStatsResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session_stats",
            category: "sessions",
            description: "Get session counts by status.",
            method: "GET",
            path: "/v1/sessions/stats",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionStatsResponse, CommandError> {
        let stats = q::session_service(ctx)?
            .stats(&ctx.caller)
            .await
            .map_err(classify_anyhow)?;
        Ok(SessionStatsResponse {
            total: stats.total,
            active: stats.active,
            idle: stats.idle,
            started: stats.started,
            waiting_for_tool_results: stats.waiting_for_tool_results,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<GetSessionStats>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct PinSession {
    pub session_id: String,
}

impl Command for PinSession {
    type Output = bool;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "pin_session",
            category: "sessions",
            description: "Pin a session for the current user.",
            method: "PUT",
            path: "/v1/sessions/{session_id}/pin",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let user_id = ctx.caller.user_id.ok_or_else(|| {
            CommandError::Forbidden("Authentication required to pin sessions".to_string())
        })?;
        let session_id = q::parse_session_id(&self.session_id)?;
        q::session_service(ctx)?
            .pin(&ctx.caller, user_id, session_id.uuid())
            .await
            .map_err(classify_anyhow)?;
        Ok(true)
    }
}

inventory::submit! { CommandDescriptor::of::<PinSession>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UnpinSession {
    pub session_id: String,
}

impl Command for UnpinSession {
    type Output = bool;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "unpin_session",
            category: "sessions",
            description: "Unpin a session for the current user.",
            method: "DELETE",
            path: "/v1/sessions/{session_id}/pin",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let user_id = ctx.caller.user_id.ok_or_else(|| {
            CommandError::Forbidden("Authentication required to unpin sessions".to_string())
        })?;
        let session_id = q::parse_session_id(&self.session_id)?;
        q::session_service(ctx)?
            .unpin(&ctx.caller, user_id, session_id.uuid())
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<UnpinSession>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CancelSession {
    pub session_id: String,
}

impl Command for CancelSession {
    type Output = CancelTurnResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "cancel_session",
            category: "sessions",
            description: "Cancel the currently executing turn in a session.",
            method: "POST",
            path: "/v1/sessions/{session_id}/cancel",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<CancelTurnResponse, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        let session = q::get_session(ctx, session_id, None).await?;

        if session.status != everruns_core::SessionStatus::Active {
            return Ok(CancelTurnResponse {
                status: CancelStatus::NoOp,
                message: "No turn currently running".to_string(),
            });
        }

        if let Err(error) = q::runner(ctx)?.cancel_run(session_id).await {
            tracing::error!(session_id = %session_id, error = %error, "Failed to cancel workflow");
        }

        let turn_id = TurnId::from_uuid(session_id.uuid());
        let input_message_id = MessageId::new();

        if let Some(event_service) = &ctx.event_service {
            let cancelled_event = EventRequest::new(
                session_id,
                EventContext::turn(turn_id, input_message_id),
                TurnCancelledData {
                    turn_id,
                    reason: Some("User requested cancellation".to_string()),
                    usage: None,
                },
            );
            if let Err(error) = event_service.emit(cancelled_event).await {
                tracing::warn!(session_id = %session_id, error = %error, "Failed to emit turn.cancelled event");
            }

            let user_message_event = EventRequest::new(
                session_id,
                EventContext::turn(turn_id, input_message_id),
                InputMessageData::new(Message::user("User requested to cancel the work.")),
            );
            if let Err(error) = event_service.emit(user_message_event).await {
                tracing::warn!(session_id = %session_id, error = %error, "Failed to emit user cancellation message");
            }
        }

        Ok(CancelTurnResponse {
            status: CancelStatus::Cancelled,
            message: "Turn cancelled successfully".to_string(),
        })
    }
}

inventory::submit! { CommandDescriptor::of::<CancelSession>() }
