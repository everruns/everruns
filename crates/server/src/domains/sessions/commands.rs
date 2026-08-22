use super::queries as q;
use super::types::{
    AddSessionParticipantRequest, CancelStatus, CancelTurnResponse, CreateSessionRequest,
    ForkSessionRequest, GetOrCreateChatSessionRequest, SessionFacetsResponse, SessionStatsResponse,
    UpdateSessionRequest,
};
use crate::domains::common::*;
use crate::services::PrincipalService;
use crate::storage::backend::MAX_SESSION_PARTICIPANT_HISTORY;
use chrono::{DateTime, Utc};
use everruns_core::events::{
    EventContext, EventData, EventRequest, InputMessageData, LLM_GENERATION, SessionIdledData,
    TurnCancelledData, deserialize_event_data,
};
use everruns_core::{Message, SessionContextReport};
use everruns_platform::ANONYMOUS_USER_ID;
use everruns_platform::capabilities::session_title_updated_event;
use everruns_platform::{
    Session, SessionActivity, SessionParticipant, SessionParticipantKind, SessionParticipantRole,
    SessionSource,
};
use everruns_provider::model_profiles::get_model_profile;
use everruns_provider::provider::DriverId;
use everruns_provider::typed_id::{AgentId, MessageId, SessionParticipantId, TurnId};
use serde::Deserialize;
use std::str::FromStr;
use utoipa::ToSchema;

fn validation_error(
    error: (
        axum::http::StatusCode,
        axum::Json<crate::api::common::ErrorResponse>,
    ),
) -> CommandError {
    let body = error.1.0;
    let message = body.detail.unwrap_or_else(|| {
        if body.title.is_empty() {
            "Request failed".to_string()
        } else {
            body.title
        }
    });
    match error.0 {
        axum::http::StatusCode::NOT_FOUND => CommandError::not_found_msg(message),
        _ => CommandError::bad_request(message),
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
        if let Some(limiter) = &ctx.org_rate_limiter
            && limiter.check_session_create(ctx.org_id()).await.is_err()
        {
            return Err(
                CommandError::rate_limited("Too many requests. Please try again later.")
                    .with_code("rate_limited")
                    .with_retry_after(60),
            );
        }

        let mut req = self.0;
        req.locale =
            crate::api::validation::normalize_locale(req.locale).map_err(limit_validation_error)?;

        // Cheap request-shape validation first, so a malformed request reports
        // 400 rather than being masked by a 409 when the org is at the cap.
        if req.harness_id.is_some() && req.harness_name.is_some() {
            return Err(CommandError::bad_request(
                "Cannot specify both harness_id and harness_name",
            ));
        }
        if req.agent_id.is_some() && req.agent_name.is_some() {
            return Err(CommandError::bad_request(
                "Cannot specify both agent_id and agent_name",
            ));
        }
        if req.seed != everruns_core::SessionSeedMode::Fresh && req.forked_from_session_id.is_none()
        {
            return Err(CommandError::bad_request(
                "seed requires forked_from_session_id",
            ));
        }

        // Enforce per-org session cap before the heavier creation work. Sessions
        // are hard-deleted, so the count reflects only live rows.
        let max = ctx.resource_limits.max_sessions_per_org;
        let count = ctx
            .db
            .count_sessions_for_org(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;
        if count >= max {
            return Err(CommandError::conflict(format!(
                "Session limit reached (max {max})"
            )));
        }

        // Resolve the agent first (by id or name) so its harness can seed the
        // session harness when no explicit harness is supplied (agent-first
        // creation). The agent's harness is a default, not an override: an
        // explicit request harness still wins (D4). Selecting an agent also
        // exposes and executes agent configuration, so SESSION_MANAGE alone is
        // not enough for deployments that split session and agent permissions.
        if req.agent_id.is_some() || req.agent_name.is_some() {
            crate::domains::agents::AGENT_VIEW
                .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
                .map_err(|e| CommandError::forbidden(e.to_string()))?;
        }

        let (agent_internal_id, agent_public_id, agent_harness_id) =
            if let Some(agent_id) = req.agent_id {
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
                let harness_id =
                    (row.harness_source != "organization_default").then_some(row.harness_id);
                (Some(row.id.uuid()), Some(public_id), harness_id)
            } else if let Some(name) = req.agent_name.as_deref() {
                let row = ctx
                    .db
                    .get_agent_by_name(ctx.org_id(), name)
                    .await
                    .map_err(classify_anyhow)?
                    .ok_or_else(|| CommandError::not_found("Agent"))?;
                let public_id: AgentId = row
                    .public_id
                    .parse()
                    .unwrap_or_else(|_| AgentId::from_uuid(row.id.uuid()));
                let harness_id =
                    (row.harness_source != "organization_default").then_some(row.harness_id);
                (Some(row.id.uuid()), Some(public_id), harness_id)
            } else {
                (None, None, None)
            };

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
                        CommandError::not_found_msg(
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
            agent_harness_id,
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
                .get_model(ctx.org_id(), model_id.uuid())
                .await
                .map_err(classify_anyhow)?
                .ok_or_else(|| CommandError::not_found("Model"))?;
        }

        if let Some(prompt) = req.system_prompt.as_ref() {
            crate::api::validation::validate_agent_system_prompt(prompt)
                .map_err(limit_validation_error)?;
        }
        if !req.initial_files.is_empty() {
            crate::api::validation::validate_initial_files(&req.initial_files)
                .map_err(limit_validation_error)?;
        }
        crate::domains::capabilities::validation::validate_feature_gated_capability_refs(
            &ctx.feature_flags,
            &req.capabilities,
        )?;

        // Attaching a session to an existing workspace grants the session (and
        // its agent) read/write access to that workspace's files, so require
        // WORKSPACE_MANAGE on the caller — SESSION_MANAGE alone must not be a
        // path to a workspace the caller cannot otherwise manage. The service
        // additionally validates the target exists, is active, and is in-org.
        if req.workspace_id.is_some() {
            crate::domains::workspaces::WORKSPACE_MANAGE
                .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
                .map_err(|e| CommandError::forbidden(e.to_string()))?;
        }

        // A chat thread is an ordinary session created here (EVE-852), so the
        // client may label itself `chat`. Everything else is server-owned, or
        // the facet rail would report whatever a caller claimed.
        let source = req.source.unwrap_or(SessionSource::Api);
        if !source.is_client_declarable() {
            return Err(CommandError::bad_request(format!(
                "source must be one of chat, api (got {source})"
            )));
        }
        // A session created under a parent is a delegated child whatever the
        // caller called it. This is the spawn path the in-process worker
        // adapter takes, so the rule lives here rather than at each call site.
        let source = if req.parent_session_id.is_some() {
            SessionSource::Subagent
        } else {
            source
        };

        q::session_service(ctx)?
            .create(
                &ctx.caller,
                harness_id.uuid(),
                agent_internal_id,
                agent_public_id,
                source,
                req,
            )
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateSession>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSessionParticipants {
    /// Session whose participant history should be returned.
    pub session_id: String,
}

impl Command for ListSessionParticipants {
    type Output = Vec<SessionParticipant>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_session_participants",
            category: "sessions",
            description: "List the participant history for a session.",
            method: "GET",
            path: "/v1/sessions/{session_id}/participants",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        ensure_session_exists(ctx, session_id).await?;

        let rows = ctx
            .db
            .list_session_participants(ctx.org_id(), session_id)
            .await
            .map_err(classify_anyhow)?;
        if rows.len() > MAX_SESSION_PARTICIPANT_HISTORY {
            return Err(CommandError::conflict(format!(
                "Session participant history exceeds the {MAX_SESSION_PARTICIPANT_HISTORY} row limit"
            )));
        }
        Ok(rows.into_iter().map(|row| row.to_core()).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListSessionParticipants>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddSessionParticipant {
    /// Session that receives the participant.
    pub session_id: String,
    /// Participant to add.
    #[serde(flatten)]
    pub req: AddSessionParticipantRequest,
}

impl Command for AddSessionParticipant {
    type Output = SessionParticipant;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "add_session_participant",
            category: "sessions",
            description: "Add a member participant to a session.",
            method: "POST",
            path: "/v1/sessions/{session_id}/participants",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        let session = ensure_session_exists(ctx, session_id).await?;
        let role = self.req.role.unwrap_or(SessionParticipantRole::Member);
        if role == SessionParticipantRole::Host {
            return Err(CommandError::bad_request(
                "Host participant is managed by session creation",
            ));
        }

        let (agent_id, agent_version_id) = match self.req.kind {
            SessionParticipantKind::Agent => {
                let agent_id = self
                    .req
                    .agent_id
                    .ok_or_else(|| CommandError::bad_request("agent_id is required"))?;
                let agent = ctx
                    .db
                    .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
                    .await
                    .map_err(classify_anyhow)?
                    .ok_or_else(|| CommandError::not_found("Agent"))?;
                (Some(agent.id), agent.default_version_id)
            }
            SessionParticipantKind::User => {
                if self.req.agent_id.is_some() {
                    return Err(CommandError::bad_request(
                        "User participants cannot include agent_id",
                    ));
                }
                (None, None)
            }
        };

        let is_user_participant = self.req.kind == SessionParticipantKind::User;
        if !is_user_participant {
            let rows = ctx
                .db
                .list_session_participants(ctx.org_id(), session_id)
                .await
                .map_err(classify_anyhow)?;
            if rows.len() > MAX_SESSION_PARTICIPANT_HISTORY {
                return Err(CommandError::conflict(format!(
                    "Session participant history exceeds the {MAX_SESSION_PARTICIPANT_HISTORY} row limit"
                )));
            }
            if rows.iter().any(|row| {
                row.kind == SessionParticipantKind::Agent.to_string()
                    && row.role == SessionParticipantRole::Member.to_string()
                    && row.agent_id == agent_id
                    && row.left_at.is_none()
            }) {
                return Err(CommandError::conflict(
                    "Session already has an active agent participant for this agent",
                ));
            }
        }

        let (principal_id, display_name) = if is_user_participant {
            match ctx.caller.user_id {
                Some(user_id) => {
                    let principal = PrincipalService::new(ctx.db.clone())
                        .ensure_user_principal(ctx.org_id(), user_id)
                        .await
                        .map_err(classify_anyhow)?;
                    let display_name = ctx
                        .db
                        .get_user(user_id)
                        .await
                        .map_err(classify_anyhow)?
                        .map(|user| user.name.trim().to_string())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| "User".to_string());
                    (principal.id, Some(display_name))
                }
                None => (session.owner_principal_id, Some("User".to_string())),
            }
        } else {
            (session.owner_principal_id, None)
        };

        let input = crate::storage::models::CreateSessionParticipantRow {
            org_id: ctx.org_id(),
            session_id,
            kind: self.req.kind,
            agent_id,
            agent_version_id,
            principal_id,
            display_name,
            role,
            joined_at: None,
        };

        let row = if is_user_participant {
            ctx.db
                .ensure_active_user_session_participant(input)
                .await
                .map_err(classify_anyhow)?
        } else {
            ctx.db
                .create_session_participant(input)
                .await
                .map_err(classify_anyhow)?
        };

        Ok(row.to_core())
    }
}

inventory::submit! { CommandDescriptor::of::<AddSessionParticipant>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct LeaveSessionParticipant {
    /// Session that owns the participant.
    pub session_id: String,
    /// Participant row to mark as left.
    pub participant_id: String,
}

impl Command for LeaveSessionParticipant {
    type Output = SessionParticipant;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "leave_session_participant",
            category: "sessions",
            description: "Mark a session member participant as having left.",
            method: "DELETE",
            path: "/v1/sessions/{session_id}/participants/{participant_id}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        ensure_session_exists(ctx, session_id).await?;
        let participant_id: SessionParticipantId = self
            .participant_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid participant ID: {e}")))?;

        let participants = ctx
            .db
            .list_session_participants(ctx.org_id(), session_id)
            .await
            .map_err(classify_anyhow)?;
        let participant = participants
            .iter()
            .find(|row| row.id == participant_id)
            .ok_or_else(|| CommandError::not_found("Participant"))?;
        if participant.role == SessionParticipantRole::Host.to_string()
            && participant.left_at.is_none()
        {
            return Err(CommandError::conflict(
                "Host participant cannot leave through this endpoint",
            ));
        }

        ctx.db
            .leave_session_participant(ctx.org_id(), session_id, participant_id)
            .await
            .map_err(classify_anyhow)?
            .map(|row| row.to_core())
            .ok_or_else(|| CommandError::not_found("Participant"))
    }
}

inventory::submit! { CommandDescriptor::of::<LeaveSessionParticipant>() }

async fn ensure_session_exists(
    ctx: &Ctx,
    session_id: everruns_provider::typed_id::SessionId,
) -> Result<crate::storage::models::SessionRow, CommandError> {
    ctx.db
        .get_session(ctx.org_id(), session_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Session"))
}

/// Fork a session into a new, independent session (knowledge/runtime-resources/forking-sessions.md).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ForkSession {
    /// Session to fork (prefixed public id).
    pub session_id: String,
    /// Optional overrides applied to the fork.
    #[serde(default)]
    pub overrides: ForkSessionRequest,
}

impl Command for ForkSession {
    type Output = Session;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "fork_session",
            category: "sessions",
            description: "Fork a session into a new, independent session that copies its conversation history and workspace files.",
            method: "POST",
            path: "/v1/sessions/{session_id}/fork",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Session, CommandError> {
        // Fork creates a new session (and deep-copies parent state), so it
        // shares the same per-org velocity bucket as CreateSession. Enforce it
        // here in the transport-independent command so MCP/inventory dispatch
        // cannot bypass the limit the REST handler previously enforced alone
        // (TM-DOS-016).
        if let Some(limiter) = &ctx.org_rate_limiter
            && limiter.check_session_create(ctx.org_id()).await.is_err()
        {
            return Err(
                CommandError::rate_limited("Too many requests. Please try again later.")
                    .with_code("rate_limited")
                    .with_retry_after(60),
            );
        }

        let parent_id = q::parse_session_id(&self.session_id)?;

        // Existence + active-status checks here so callers get precise HTTP
        // codes (404 / 409) rather than a generic 500 surfaced from the service.
        let parent = ctx
            .db
            .get_session(ctx.org_id(), parent_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))?;
        if matches!(
            parent.status.as_str(),
            "active" | "waiting_for_tool_results"
        ) {
            return Err(CommandError::conflict(
                "Cannot fork a session while it is mid-turn".to_string(),
            ));
        }

        let mut overrides = self.overrides;
        overrides.locale =
            crate::api::validation::normalize_locale(overrides.locale).map_err(|_| {
                CommandError::bad_request(crate::api::validation::VALIDATION_ERROR_MESSAGE)
            })?;

        q::session_service(ctx)?
            .fork(
                &ctx.caller,
                parent_id,
                super::ForkOverrides {
                    title: overrides.title,
                    goal: overrides.goal,
                    tags: overrides.tags,
                    model_id: overrides.model_id,
                    agent_id: overrides.agent_id,
                    locale: overrides.locale,
                    system_prompt: overrides.system_prompt,
                },
            )
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ForkSession>() }

/// Filters shared by `ListSessions` and `GetSessionFacets` so the page and the
/// counts that annotate it can never describe different populations (EVE-852).
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct SessionFilterArgs {
    /// Agent's prefixed public identifier.
    pub agent_id: Option<AgentId>,
    /// Case-insensitive title substring match.
    pub search: Option<String>,
    /// Comma-separated sources (`chat`, `api`, `slack`, `ag_ui`, `fcp`,
    /// `schedule`, `webhook`, `a2a`, `eval`, `subagent`, `unknown`).
    pub source: Option<String>,
    /// Comma-separated derived activities (`running`, `paused`, `failed`,
    /// `completed`, `idle`).
    pub status: Option<String>,
    /// Restrict to sessions owned by the calling user.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    pub mine: Option<bool>,
    /// Inclusive lower bound on `created_at` (RFC 3339).
    pub created_after: Option<String>,
    /// Exclusive upper bound on `created_at` (RFC 3339).
    pub created_before: Option<String>,
    /// Include archived sessions. Default `false`.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    pub include_archived: Option<bool>,
    /// `created_at` (default) or `last_activity`. The chat thread list is this
    /// endpoint with `source=chat`, `mine=true`, `order=last_activity`.
    pub order: Option<String>,
}

impl SessionFilterArgs {
    /// Resolve into the storage-level predicate, translating the agent's public
    /// id and rejecting unknown enum members rather than silently widening the
    /// result set.
    async fn resolve(
        self,
        ctx: &Ctx,
    ) -> Result<Option<crate::storage::SessionListFilters>, CommandError> {
        let agent_id = match self.agent_id {
            Some(agent_id) => {
                let row = ctx
                    .db
                    .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
                    .await
                    .map_err(classify_anyhow)?;
                // An unknown agent matches nothing; the caller renders an empty
                // page rather than an error.
                match row {
                    Some(row) => Some(AgentId::from_uuid(row.id.uuid())),
                    None => return Ok(None),
                }
            }
            None => None,
        };

        fn parse_csv<T>(
            raw: Option<&str>,
            parse: impl Fn(&str) -> Option<T>,
            field: &str,
        ) -> Result<Vec<T>, CommandError> {
            let Some(raw) = raw else {
                return Ok(vec![]);
            };
            raw.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(|part| {
                    parse(part).ok_or_else(|| {
                        CommandError::bad_request(format!("Unknown {field}: {part}"))
                    })
                })
                .collect()
        }

        fn parse_time(
            raw: Option<&str>,
            field: &str,
        ) -> Result<Option<DateTime<Utc>>, CommandError> {
            raw.map(|value| {
                DateTime::parse_from_rfc3339(value)
                    .map(|t| t.with_timezone(&Utc))
                    .map_err(|_| {
                        CommandError::bad_request(format!("{field} must be an RFC 3339 timestamp"))
                    })
            })
            .transpose()
        }

        let order = match self.order.as_deref() {
            None | Some("created_at") => crate::storage::SessionListOrder::CreatedAt,
            Some("last_activity") => crate::storage::SessionListOrder::LastActivity,
            Some(other) => {
                return Err(CommandError::bad_request(format!("Unknown order: {other}")));
            }
        };

        Ok(Some(crate::storage::SessionListFilters {
            include_archived: self.include_archived.unwrap_or(false),
            agent_id,
            search: self.search,
            sources: parse_csv(self.source.as_deref(), SessionSource::parse, "source")?,
            activities: parse_csv(self.status.as_deref(), SessionActivity::parse, "status")?,
            // `mine` without an authenticated user would silently list the
            // whole org, so it resolves to "nothing" instead.
            owner_user_id: match self.mine {
                Some(true) => Some(ctx.caller.user_id.unwrap_or(ANONYMOUS_USER_ID)),
                _ => None,
            },
            created_after: parse_time(self.created_after.as_deref(), "created_after")?,
            created_before: parse_time(self.created_before.as_deref(), "created_before")?,
            order,
        }))
    }
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListSessions {
    #[serde(flatten)]
    pub filters: SessionFilterArgs,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    /// Zero-based offset into the result set.
    pub offset: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    /// Maximum number of items returned in this page.
    pub limit: Option<u32>,
}

impl Command for ListSessions {
    type Output = Paginated<Session>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_sessions",
            category: "sessions",
            description: "List sessions. Filter by agent_id, source, status, owner (mine), and creation window; search by title; order by created_at or last_activity. Supports pagination (limit/offset).",
            method: "GET",
            path: "/v1/sessions",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Paginated<Session>, CommandError> {
        let pagination = pagination(self.offset, self.limit);
        let empty = Paginated {
            data: vec![],
            total: 0,
            offset: pagination.offset,
            limit: pagination.limit,
        };
        let Some(filters) = self.filters.resolve(ctx).await? else {
            return Ok(empty);
        };
        let (sessions, total) = q::session_service(ctx)?
            .list(
                &ctx.caller,
                ctx.caller.user_id,
                &filters,
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

/// Facet-rail counts and masthead metrics over the sessions list predicate.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct GetSessionFacets {
    #[serde(flatten)]
    pub filters: SessionFilterArgs,
}

impl Command for GetSessionFacets {
    type Output = SessionFacetsResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session_facets",
            category: "sessions",
            description: "Counts per status, source, and agent plus masthead metrics for the sessions list, over the same filters as list_sessions.",
            method: "GET",
            path: "/v1/sessions/facets",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionFacetsResponse, CommandError> {
        let Some(filters) = self.filters.resolve(ctx).await? else {
            return Ok(SessionFacetsResponse {
                total: 0,
                by_activity: vec![],
                by_source: vec![],
                by_agent: vec![],
                active_now: 0,
                failed_today: 0,
                p95_duration_ms: 0,
                tokens_today: 0,
            });
        };
        q::session_service(ctx)?
            .facets(&ctx.caller, &filters)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<GetSessionFacets>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetSession {
    /// Session's prefixed public identifier.
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
pub struct GetSessionContextReport {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for GetSessionContextReport {
    type Output = SessionContextReport;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session_context_report",
            category: "sessions",
            description: "Get the latest estimated context token breakdown for a session, grouped by system prompt, tools, rules, skills, MCP, subagents, and conversation.",
            method: "GET",
            path: "/v1/sessions/{session_id}/context-report",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionContextReport, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        let session = q::get_session(ctx, session_id, ctx.caller.user_id).await?;
        let rows = ctx
            .db
            .list_events(
                session_id,
                None,
                None,
                &[LLM_GENERATION.to_string()],
                &[],
                None,
                Some(1),
            )
            .await
            .map_err(classify_anyhow)?;

        let Some(row) = rows.into_iter().next() else {
            return Ok(SessionContextReport {
                session_id: session.id.to_string(),
                model: "unknown".to_string(),
                context_window_tokens: None,
                estimated_input_tokens: 0,
                sections: vec![],
                contributions: vec![],
                cumulative_usage: session.usage,
            });
        };

        let EventData::LlmGeneration(data) = deserialize_event_data(&row.event_type, row.data)
        else {
            return Err(CommandError::internal(anyhow::anyhow!(
                "latest llm.generation event could not be decoded"
            )));
        };
        let context_window_tokens = data
            .metadata
            .provider
            .as_deref()
            .and_then(parse_provider_type)
            .and_then(|provider_type| get_model_profile(&provider_type, &data.metadata.model))
            .and_then(|profile| profile.limits)
            .and_then(|limits| u32::try_from(limits.context).ok());

        Ok(everruns_core::build_session_context_report_from_generation(
            session.id.to_string(),
            &data,
            context_window_tokens,
            session.usage,
        ))
    }
}

fn parse_provider_type(provider: &str) -> Option<DriverId> {
    DriverId::from_str(&provider.to_ascii_lowercase()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::harnesses::CreateHarness;
    use crate::domains::harnesses::types::CreateHarnessRequest;
    use crate::storage::StorageBackend;
    use everruns_core::{Caller, DEFAULT_ORG_ID, DefaultPermissionResolver, OrgRole};
    use everruns_provider::typed_id::{HarnessId, SessionId};
    use std::sync::Arc;
    use uuid::Uuid;

    #[test]
    fn parse_provider_type_accepts_mixed_case_known_values() {
        assert_eq!(parse_provider_type("OpenAI"), Some(DriverId::OpenAI));
        assert_eq!(
            parse_provider_type("AZURE_OPENAI"),
            Some(DriverId::AzureOpenAI)
        );
    }

    fn test_ctx(db: Arc<StorageBackend>, max_sessions_per_org: i64) -> Ctx {
        let session_service = Arc::new(crate::domains::sessions::SessionService::new(db.clone()));
        let event_service = Arc::new(crate::services::EventService::new(
            db.clone(),
            crate::event_delivery::EventDelivery::in_memory(),
        ));
        let capability_service =
            Arc::new(crate::services::CapabilityService::new(db.clone(), None));
        // Internal caller: the owner principal resolves to the system principal,
        // avoiding a user lookup the in-memory store cannot satisfy.
        let mut ctx = Ctx::new(
            Caller::internal(DEFAULT_ORG_ID),
            db,
            capability_service,
            None,
            Arc::new(DefaultPermissionResolver),
        )
        .with_session_service(session_service)
        .with_event_service(event_service);
        ctx.resource_limits.max_sessions_per_org = max_sessions_per_org;
        ctx
    }

    fn external_test_ctx(db: Arc<StorageBackend>, user_id: Uuid) -> Ctx {
        let session_service = Arc::new(crate::domains::sessions::SessionService::new(db.clone()));
        let capability_service =
            Arc::new(crate::services::CapabilityService::new(db.clone(), None));
        Ctx::new(
            Caller {
                org_id: DEFAULT_ORG_ID,
                org_public_id: everruns_core::organization::org_public_id_from_internal(
                    DEFAULT_ORG_ID,
                ),
                user_id: Some(user_id),
                role: OrgRole::Owner,
                is_platform_user: false,
                is_internal: false,
            },
            db,
            capability_service,
            None,
            Arc::new(DefaultPermissionResolver),
        )
        .with_session_service(session_service)
    }

    fn create_request(harness_id: HarnessId) -> CreateSessionRequest {
        CreateSessionRequest {
            source: None,
            workspace_id: None,
            harness_id: Some(harness_id),
            harness_name: None,
            agent_id: None,
            agent_name: None,
            agent_identity_id: None,
            title: Some("Test Session".to_string()),
            goal: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            parent_session_id: None,
            forked_from_session_id: None,
            budget_root_session_id: None,
            seed: everruns_core::SessionSeedMode::Fresh,
        }
    }

    async fn seed_harness(ctx: &Ctx) -> HarnessId {
        CreateHarness(CreateHarnessRequest {
            name: "limit-harness".to_string(),
            display_name: None,
            description: None,
            system_prompt: Some("prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        })
        .execute(ctx)
        .await
        .expect("seed harness")
        .id
    }

    async fn chat_test_ctx(session_rpm: u32) -> (Ctx, crate::auth::rate_limit::OrgRateLimiter) {
        let db = Arc::new(StorageBackend::in_memory());
        crate::org_init::initialize_org_harnesses(&db, DEFAULT_ORG_ID)
            .await
            .expect("initialize built-in harnesses");
        let user = db
            .create_user(crate::storage::models::CreateUserRow {
                email: format!("chat-owner-{}@example.com", Uuid::now_v7()),
                name: "Chat Owner".to_string(),
                avatar_url: None,
                roles: vec!["user".to_string()],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .expect("create chat owner");
        let limiter =
            crate::auth::rate_limit::OrgRateLimiter::for_test_with_session_rpm(session_rpm);
        let ctx = external_test_ctx(db, user.id)
            .with_chat_harness_name(Some("platform-chat".to_string()))
            .with_org_rate_limiter(limiter.clone());
        (ctx, limiter)
    }

    #[tokio::test]
    async fn create_session_enforces_shared_org_rate_limit() {
        let db = Arc::new(StorageBackend::in_memory());
        let limiter = crate::auth::rate_limit::OrgRateLimiter::for_test_with_session_rpm(1);
        limiter
            .check_session_create(DEFAULT_ORG_ID)
            .await
            .expect("initial permit");
        let ctx = test_ctx(db, 100).with_org_rate_limiter(limiter);

        let err = CreateSession(create_request(HarnessId::new()))
            .execute(&ctx)
            .await
            .expect_err("exhausted per-org bucket must reject command dispatch");

        assert!(matches!(err.kind, CommandErrorKind::RateLimited(_)));
        assert_eq!(err.code.as_deref(), Some("rate_limited"));
        assert_eq!(err.retry_after_seconds, Some(60));
    }

    #[tokio::test]
    async fn fork_session_enforces_shared_org_rate_limit() {
        // Fork must consume the same per-org bucket as create so MCP dispatch
        // cannot bypass it. The limiter is checked before parent lookup, so an
        // exhausted bucket rejects the fork regardless of the parent id.
        let db = Arc::new(StorageBackend::in_memory());
        let limiter = crate::auth::rate_limit::OrgRateLimiter::for_test_with_session_rpm(1);
        limiter
            .check_session_create(DEFAULT_ORG_ID)
            .await
            .expect("initial permit");
        let ctx = test_ctx(db, 100).with_org_rate_limiter(limiter);

        let err = ForkSession {
            session_id: SessionId::new().to_string(),
            overrides: Default::default(),
        }
        .execute(&ctx)
        .await
        .expect_err("exhausted per-org bucket must reject fork dispatch");

        assert!(matches!(err.kind, CommandErrorKind::RateLimited(_)));
        assert_eq!(err.code.as_deref(), Some("rate_limited"));
        assert_eq!(err.retry_after_seconds, Some(60));
    }

    #[tokio::test]
    async fn get_or_create_chat_session_enforces_rate_limit_when_creating() {
        let (ctx, limiter) = chat_test_ctx(1).await;
        limiter
            .check_session_create(DEFAULT_ORG_ID)
            .await
            .expect("initial permit");

        let err = GetOrCreateChatSession { locale: None }
            .execute(&ctx)
            .await
            .expect_err("exhausted per-org bucket must reject chat session creation");

        assert!(
            matches!(err.kind, CommandErrorKind::RateLimited(_)),
            "unexpected error: {err:?}"
        );
        assert_eq!(err.code.as_deref(), Some("rate_limited"));
        assert_eq!(err.retry_after_seconds, Some(60));
    }

    #[tokio::test]
    async fn get_or_create_chat_session_reuse_does_not_consume_rate_limit() {
        let (ctx, limiter) = chat_test_ctx(1).await;

        let created = GetOrCreateChatSession { locale: None }
            .execute(&ctx)
            .await
            .expect("first request creates within the available permit");
        assert!(
            limiter.check_session_create(DEFAULT_ORG_ID).await.is_err(),
            "create branch must consume the shared bucket"
        );

        let reused = GetOrCreateChatSession { locale: None }
            .execute(&ctx)
            .await
            .expect("reuse must not check or consume the exhausted bucket");

        assert_eq!(reused.id, created.id);
    }

    // Minimal runner whose `cancel_run` succeeds without a real durable backend,
    // so `CancelSession` can be exercised against the in-memory store.
    struct CancelTestRunner;

    #[async_trait::async_trait]
    impl everruns_worker::AgentRunner for CancelTestRunner {
        async fn start_run(
            &self,
            _org_id: i64,
            _session_id: SessionId,
            _harness_id: HarnessId,
            _agent_id: Option<AgentId>,
            _input_message_id: MessageId,
            _request_id: Option<String>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn resume_after_tool_results(&self, _session_id: SessionId) -> anyhow::Result<()> {
            Ok(())
        }

        async fn cancel_run(&self, _session_id: SessionId) -> anyhow::Result<()> {
            Ok(())
        }

        async fn is_running(&self, _session_id: SessionId) -> bool {
            false
        }

        async fn active_count(&self) -> usize {
            0
        }
    }

    // EVE-708: cancelling an active turn must settle the session back to `idle`.
    // The durable workflow is marked cancelled and the worker short-circuits before
    // the runtime's idle transition, so `CancelSession` itself must idle the session
    // or it stays `active` forever, blocking clean follow-up turns.
    #[tokio::test]
    async fn cancel_active_session_transitions_to_idle() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = test_ctx(db.clone(), 100).with_runner(Arc::new(CancelTestRunner));
        let harness_id = seed_harness(&ctx).await;

        let session = CreateSession(create_request(harness_id))
            .execute(&ctx)
            .await
            .expect("create session");

        // Simulate an in-flight turn.
        q::session_service(&ctx)
            .unwrap()
            .update_status(&ctx.caller, session.id.uuid(), "active".to_string())
            .await
            .expect("mark active");

        let response = CancelSession {
            session_id: session.id.to_string(),
        }
        .execute(&ctx)
        .await
        .expect("cancel");
        assert!(
            matches!(response.status, CancelStatus::Cancelled),
            "expected an active cancel, got {:?}",
            response.status
        );

        let after = q::get_session(&ctx, session.id, None)
            .await
            .expect("reload session");
        assert_eq!(
            after.status,
            everruns_platform::SessionStatus::Idle,
            "cancelled session must settle to idle"
        );
    }

    #[tokio::test]
    async fn update_session_title_emits_one_semantic_event_per_change() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = test_ctx(db, 100);
        let harness_id = seed_harness(&ctx).await;
        let session = CreateSession(create_request(harness_id))
            .execute(&ctx)
            .await
            .expect("create session");

        for _ in 0..2 {
            UpdateSessionCmd {
                session_id: session.id.to_string(),
                req: serde_json::from_value(serde_json::json!({"title": "Updated title"}))
                    .expect("update request"),
            }
            .execute(&ctx)
            .await
            .expect("update title");
        }

        let events = ctx
            .event_service
            .as_ref()
            .expect("event service")
            .list(
                session.id.uuid(),
                None,
                None,
                &[everruns_core::SESSION_TITLE_UPDATED.to_string()],
                &[],
                None,
                None,
            )
            .await
            .expect("list events");
        assert_eq!(events.len(), 1);
        assert!(events[0].context.turn_id.is_none());
        match &events[0].data {
            EventData::SessionTitleUpdated(data) => {
                assert_eq!(data.previous_title.as_deref(), Some("Test Session"));
                assert_eq!(data.title, "Updated title");
            }
            data => panic!("unexpected event data: {data:?}"),
        }
    }

    #[tokio::test]
    async fn session_creation_rejected_at_limit_and_allowed_below() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = test_ctx(db, 1);
        let harness_id = seed_harness(&ctx).await;

        CreateSession(create_request(harness_id))
            .execute(&ctx)
            .await
            .expect("first session below limit");

        let err = CreateSession(create_request(harness_id))
            .execute(&ctx)
            .await
            .expect_err("second session exceeds the cap");
        assert_eq!(err.status().as_u16(), 409);
        assert!(err.message().contains("Session limit reached"));
    }

    #[tokio::test]
    async fn create_session_with_non_internal_owner_sets_parent_session_id() {
        // Trusted subagent/handoff spawns dispatch CreateSession through
        // DirectPlatformStore, whose caller runs as the session owner
        // (`is_internal == false`, see caller_resolution.rs). The internal
        // parent link must survive that path. Forgery by untrusted clients is
        // prevented at the HTTP boundary, which strips `parent_session_id`
        // before dispatch (see `strip_internal_only_fields` in api/sessions.rs),
        // so the command layer must not reject a caller-set parent link.
        let db = Arc::new(StorageBackend::in_memory());
        let internal_ctx = test_ctx(db.clone(), 10);
        let harness_id = seed_harness(&internal_ctx).await;
        let owner = db
            .create_user(crate::storage::models::CreateUserRow {
                email: format!("owner-{}@example.com", Uuid::now_v7()),
                name: "Owner".to_string(),
                avatar_url: None,
                roles: vec!["user".to_string()],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .expect("create owner user");
        let ctx = external_test_ctx(db, owner.id);
        let parent = SessionId::new();
        let mut req = create_request(harness_id);
        req.parent_session_id = Some(parent);

        let session = CreateSession(req)
            .execute(&ctx)
            .await
            .expect("owner-caller spawn with a parent link should succeed");

        assert_eq!(session.parent_session_id, Some(parent));
    }

    async fn seed_agent(ctx: &Ctx, harness_id: HarnessId, name: &str) -> AgentId {
        let public_id = format!("agent_{}", Uuid::now_v7().simple());
        let row = ctx
            .db
            .create_agent(
                ctx.org_id(),
                crate::storage::models::CreateAgentRow {
                    public_id: public_id.clone(),
                    name: name.to_string(),
                    display_name: None,
                    description: None,
                    system_prompt: "You are helpful.".to_string(),
                    default_model_id: None,
                    harness_id,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    tools: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    max_iterations: None,
                    parallel_tool_calls: None,
                    is_built_in: false,
                },
            )
            .await
            .expect("seed agent");
        row.public_id.parse().expect("agent public id")
    }

    #[tokio::test]
    async fn create_session_from_agent_inherits_agent_harness() {
        // Agent-first: a session created with only an agent runs on the agent's
        // own harness (D4), and an explicit request harness still overrides it.
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = test_ctx(db.clone(), 10);
        let agent_harness = seed_harness(&ctx).await;
        let override_harness = CreateHarness(CreateHarnessRequest {
            name: "override-harness".to_string(),
            display_name: None,
            description: None,
            system_prompt: Some("prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        })
        .execute(&ctx)
        .await
        .expect("seed override harness")
        .id;
        let agent_id = seed_agent(&ctx, agent_harness, "support").await;

        // 1. agent only, no harness → inherits the agent's harness.
        let mut req = create_request(agent_harness);
        req.harness_id = None;
        req.agent_id = Some(agent_id);
        let session = CreateSession(req)
            .execute(&ctx)
            .await
            .expect("create session from agent");
        assert_eq!(session.harness_id, agent_harness);

        // 2. explicit request harness overrides the agent's harness.
        let mut req = create_request(override_harness);
        req.agent_id = Some(agent_id);
        let session = CreateSession(req)
            .execute(&ctx)
            .await
            .expect("create session with explicit harness override");
        assert_eq!(session.harness_id, override_harness);
    }

    #[tokio::test]
    async fn create_session_rejects_both_agent_id_and_agent_name() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = test_ctx(db, 10);
        let harness_id = seed_harness(&ctx).await;
        let mut req = create_request(harness_id);
        req.agent_id = Some(AgentId::new());
        req.agent_name = Some("support".to_string());

        let err = CreateSession(req)
            .execute(&ctx)
            .await
            .expect_err("agent_id + agent_name is rejected");
        assert_eq!(err.status().as_u16(), 400);
    }

    #[tokio::test]
    async fn participant_commands_list_add_and_leave_history() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = test_ctx(db.clone(), 10);
        let harness_id = seed_harness(&ctx).await;
        let host_public_id = AgentId::new();
        let host_agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                crate::storage::models::CreateAgentRow {
                    public_id: host_public_id.to_string(),
                    name: "participant-host".to_string(),
                    display_name: None,
                    description: None,
                    system_prompt: "You are helpful.".to_string(),
                    default_model_id: None,
                    harness_id,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    tools: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    max_iterations: None,
                    parallel_tool_calls: None,
                    is_built_in: false,
                },
            )
            .await
            .expect("create host agent");
        let member_public_id = AgentId::new();
        let member_agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                crate::storage::models::CreateAgentRow {
                    public_id: member_public_id.to_string(),
                    name: "participant-member".to_string(),
                    display_name: None,
                    description: None,
                    system_prompt: "You are helpful.".to_string(),
                    default_model_id: None,
                    harness_id,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    tools: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    max_iterations: None,
                    parallel_tool_calls: None,
                    is_built_in: false,
                },
            )
            .await
            .expect("create member agent");

        let mut req = create_request(harness_id);
        req.agent_id = Some(host_public_id);
        let session = CreateSession(req)
            .execute(&ctx)
            .await
            .expect("create session with host agent");

        let initial = ListSessionParticipants {
            session_id: session.id.to_string(),
        }
        .execute(&ctx)
        .await
        .expect("list initial participants");
        assert_eq!(initial.len(), 2);
        let host = initial
            .iter()
            .find(|participant| participant.role == SessionParticipantRole::Host)
            .expect("host participant");
        assert_eq!(host.kind, SessionParticipantKind::Agent);
        assert_eq!(host.agent_id, Some(host_agent.id));

        let user_again = AddSessionParticipant {
            session_id: session.id.to_string(),
            req: AddSessionParticipantRequest {
                kind: SessionParticipantKind::User,
                agent_id: None,
                role: None,
            },
        }
        .execute(&ctx)
        .await
        .expect("user participant add is idempotent");
        assert_eq!(user_again.kind, SessionParticipantKind::User);
        assert_eq!(
            initial
                .iter()
                .filter(|p| p.kind == SessionParticipantKind::User)
                .count(),
            1
        );

        let added = AddSessionParticipant {
            session_id: session.id.to_string(),
            req: AddSessionParticipantRequest {
                kind: SessionParticipantKind::Agent,
                agent_id: Some(member_public_id),
                role: None,
            },
        }
        .execute(&ctx)
        .await
        .expect("add member participant");
        assert_eq!(added.role, SessionParticipantRole::Member);
        assert_eq!(added.agent_id, Some(member_agent.id));

        let duplicate_agent = AddSessionParticipant {
            session_id: session.id.to_string(),
            req: AddSessionParticipantRequest {
                kind: SessionParticipantKind::Agent,
                agent_id: Some(member_public_id),
                role: None,
            },
        }
        .execute(&ctx)
        .await
        .expect_err("duplicate active agent participant is rejected");
        assert_eq!(duplicate_agent.status().as_u16(), 409);

        let left = LeaveSessionParticipant {
            session_id: session.id.to_string(),
            participant_id: added.id.to_string(),
        }
        .execute(&ctx)
        .await
        .expect("leave member participant");
        assert!(left.left_at.is_some());

        let history = ListSessionParticipants {
            session_id: session.id.to_string(),
        }
        .execute(&ctx)
        .await
        .expect("list participant history");
        assert_eq!(history.len(), 3);
        assert!(
            history
                .iter()
                .find(|participant| participant.id == added.id)
                .and_then(|participant| participant.left_at)
                .is_some()
        );

        let err = LeaveSessionParticipant {
            session_id: session.id.to_string(),
            participant_id: host.id.to_string(),
        }
        .execute(&ctx)
        .await
        .expect_err("host cannot leave through member endpoint");
        assert_eq!(err.status().as_u16(), 409);
    }
}

inventory::submit! { CommandDescriptor::of::<GetSessionContextReport>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSessionCmd {
    /// Session's prefixed public identifier.
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
        let requested_title = req.title.clone();
        let title_event = if let Some(title) = requested_title {
            let previous_title = q::get_session(ctx, session_id, None).await?.title;
            session_title_updated_event(session_id, EventContext::empty(), previous_title, title)
        } else {
            None
        };
        // Resolve the emitter before mutating so a title change cannot succeed
        // through this path when its semantic event cannot be produced.
        let event_service = if title_event.is_some() {
            Some(ctx.event_service.clone().ok_or_else(|| {
                CommandError::internal(anyhow::anyhow!("Event service not configured"))
            })?)
        } else {
            None
        };

        let session = q::session_service(ctx)?
            .update(&ctx.caller, session_id.uuid(), req)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))?;

        if let (Some(event), Some(event_service)) = (title_event, event_service) {
            event_service.emit(event).await.map_err(classify_anyhow)?;
        }

        Ok(session)
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateSessionCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteSession {
    /// Session's prefixed public identifier.
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
            CommandError::not_found_msg(
                "Global chat is not configured for this platform".to_string(),
            )
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
            .get_or_create_chat_session(
                &ctx.caller,
                user_id,
                chat_harness_id.uuid(),
                title,
                locale,
                ctx.org_rate_limiter.as_ref(),
            )
            .await
            .map_err(|error| match error {
                super::service::GetOrCreateChatSessionError::RateLimited => {
                    CommandError::rate_limited("Too many requests. Please try again later.")
                        .with_code("rate_limited")
                        .with_retry_after(60)
                }
                super::service::GetOrCreateChatSessionError::Other(error) => classify_anyhow(error),
            })
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
    /// Session's prefixed public identifier.
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
            CommandError::forbidden("Authentication required to pin sessions".to_string())
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
    /// Session's prefixed public identifier.
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
            CommandError::forbidden("Authentication required to unpin sessions".to_string())
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
pub struct ArchiveSession {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for ArchiveSession {
    type Output = bool;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "archive_session",
            category: "sessions",
            description: "Archive a session so it drops out of default lists.",
            method: "PUT",
            path: "/v1/sessions/{session_id}/archive",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        // Resolve first so an unknown session is a 404 rather than a silent
        // no-op, and so org scoping is checked the same way every other
        // session command checks it.
        q::get_session(ctx, session_id, None).await?;
        q::session_service(ctx)?
            .archive(&ctx.caller, session_id.uuid())
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ArchiveSession>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UnarchiveSession {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for UnarchiveSession {
    type Output = bool;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "unarchive_session",
            category: "sessions",
            description: "Restore an archived session to default lists.",
            method: "DELETE",
            path: "/v1/sessions/{session_id}/archive",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::get_session(ctx, session_id, None).await?;
        q::session_service(ctx)?
            .unarchive(&ctx.caller, session_id.uuid())
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<UnarchiveSession>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CancelSession {
    /// Session's prefixed public identifier.
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

        if session.status != everruns_platform::SessionStatus::Active {
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

            // EVE-708: emit the `session.idled` lifecycle event for parity with the
            // normal turn-completion path. The cancelled workflow short-circuits in
            // the worker before the runtime reaches its `session.idled` emission, so
            // without this the lifecycle would silently stop at `turn.cancelled`.
            let idled_event = EventRequest::new(
                session_id,
                EventContext::turn(turn_id, input_message_id),
                SessionIdledData {
                    turn_id,
                    iterations: None,
                    usage: None,
                },
            );
            if let Err(error) = event_service.emit(idled_event).await {
                tracing::warn!(session_id = %session_id, error = %error, "Failed to emit session.idled event");
            }
        }

        // EVE-708: settle the session status. Cancelling marks the durable workflow
        // `Cancelled`, which makes the worker fail the task and return early — so the
        // runtime turn loop never reaches a terminal `TurnPlan` and never transitions
        // the session back to `idle`. Session status is a stored column that is not
        // derived from events, so we must set it here or the session stays `active`
        // indefinitely, blocking clean follow-up turns.
        if let Err(error) = q::session_service(ctx)?
            .update_status(&ctx.caller, session_id.uuid(), "idle".to_string())
            .await
        {
            tracing::warn!(session_id = %session_id, error = %error, "Failed to set session idle after cancel");
        }

        Ok(CancelTurnResponse {
            status: CancelStatus::Cancelled,
            message: "Turn cancelled successfully".to_string(),
        })
    }
}

inventory::submit! { CommandDescriptor::of::<CancelSession>() }
