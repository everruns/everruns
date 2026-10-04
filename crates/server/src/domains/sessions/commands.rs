use super::types::{
    AddSessionParticipantRequest, CancelStatus, CancelTurnResponse, CreateSessionRequest,
    ForkSessionRequest, SessionFacetsResponse, SessionStatsResponse, UpdateSessionRequest,
};
use super::validation::{limit_validation_error, validation_error};
use super::{platform_chat_starter as starter, queries as q};
use crate::domains::common::*;
use crate::records::ANONYMOUS_USER_ID;
use crate::records::{
    Session, SessionActivity, SessionParticipant, SessionParticipantKind, SessionParticipantRole,
    SessionSource,
};
use crate::services::PrincipalService;
use crate::storage::backend::MAX_SESSION_PARTICIPANT_HISTORY;
use chrono::{DateTime, Utc};
use everruns_capabilities::capabilities::session_title_updated_event;
use everruns_contracts::model_profiles::get_model_profile;
use everruns_contracts::provider::DriverId;
use everruns_contracts::typed_id::{AgentId, MessageId, SessionParticipantId, TurnId};
use everruns_core::events::{
    EventContext, EventData, EventRequest, InputMessageData, LLM_GENERATION, SessionIdledData,
    TurnCancelledData, deserialize_event_data,
};
use everruns_core::{RuntimeMessage, SessionContextReport};
use serde::Deserialize;
use std::str::FromStr;
use utoipa::ToSchema;

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
            description: "Create a new session. Optionally assign an agent and harness. Assign the agent with --agent_id or --agent_name (the everruns CLI spelling of this flag is --agent).",
            method: "POST",
            path: "/v1/sessions",
        }
    }

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions"], "create")
            .with_args(&[
                CliArg::new("agent_id").short('a').long("agent"),
                CliArg::new("harness_name").short('H').long("harness"),
                CliArg::new("tag").short('t'),
            ])
            .with_examples(&[CliExample::new(
                "Start a session for an agent",
                "everruns sessions create --agent agt_01h9",
            )]);
        Some(ROUTE)
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
        // Older browsers may still submit the retired direct-harness chat shape.
        if req.source == Some(SessionSource::Chat)
            && req.agent_id.is_none()
            && req.agent_name.is_none()
            && req.harness_name.as_deref() == Some("platform-chat")
        {
            req.agent_name = Some(crate::platform_chat_agent::NAME.into());
            req.harness_name = None;
        }
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

        let assigns_harness =
            req.harness_id.is_some() || (agent_internal_id.is_none() && agent_harness_id.is_none());
        let harness_id = q::resolve_session_harness_id(
            &ctx.db,
            ctx.org_id(),
            req.harness_id,
            agent_harness_id,
            ctx.fallback_harness_name.as_deref(),
        )
        .await
        .map_err(classify_anyhow)?;
        if assigns_harness {
            crate::domains::agents::commands::check_harness_assignment(ctx, harness_id).await?;
        }
        req.harness_id = Some(harness_id);

        let harness = ctx
            .db
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
        super::playground::bind_creation(ctx, &mut req, &harness, source).await?;
        // A session created under a parent is a delegated child whatever the
        // caller called it. This is the spawn path the in-process worker
        // adapter takes, so the rule lives here rather than at each call site.
        let source = if req.parent_session_id.is_some() {
            SessionSource::Subagent
        } else {
            source
        };
        let is_platform_chat = crate::platform_chat_agent::is_platform_chat(
            &ctx.db,
            ctx.org_id(),
            agent_internal_id.map(AgentId::from_uuid),
        )
        .await
        .map_err(classify_anyhow)?;
        if source == SessionSource::Chat && !is_platform_chat {
            return Err(CommandError::bad_request(
                "Chat requires the managed Platform Chat Agent; use Playground to test agents",
            ));
        }
        if is_platform_chat && (source == SessionSource::Playground || harness.name != "generic") {
            return Err(CommandError::bad_request(
                "Platform Chat requires Generic and cannot run in Playground",
            ));
        }
        if is_platform_chat
            && (req.system_prompt.is_some()
                || !req.capabilities.is_empty()
                || !req.tools.is_empty()
                || !req.mcp_servers.is_empty()
                || req.virtual_user_id.is_some()
                || req.environment.is_some())
        {
            return Err(CommandError::bad_request(
                "Platform Chat uses its managed Agent configuration, fixed runtime, and the current user's identity",
            ));
        }
        starter::mark_platform_chat_starter(&mut req, &harness.name, is_platform_chat)?;
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
            .map_err(starter::classify_create_session_error)
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions", "participants"], "list")
            .with_args(&[CliArg::new("session_id").long("session")])
            .with_examples(&[CliExample::new(
                "See who is attached to a session",
                "everruns sessions participants list --session ses_01h9",
            )]);
        Some(ROUTE)
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions", "participants"], "add")
            .with_args(&[CliArg::new("session_id").long("session")])
            .with_examples(&[CliExample::new(
                "Bring a user into a running session",
                "everruns sessions participants add --session ses_01h9 --kind user",
            )]);
        Some(ROUTE)
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
                        .ensure_default_virtual_user_principal(ctx.org_id(), user_id)
                        .await
                        .map_err(classify_anyhow)?;
                    let display_name = principal
                        .metadata
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(|name| name.trim().to_string())
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions", "participants"], "leave")
            .with_args(&[CliArg::new("session_id").long("session")])
            .with_examples(&[CliExample::new(
                "Remove one participant from a session",
                "everruns sessions participants leave --session ses_01h9 --participant-id par_01h9",
            )]);
        Some(ROUTE)
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
    session_id: everruns_contracts::typed_id::SessionId,
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions"], "fork")
            .with_args(&[CliArg::new("session_id").at(1).long("session")])
            .with_examples(&[CliExample::new(
                "Branch from a session to try a different direction",
                "everruns sessions fork ses_01h9",
            )]);
        Some(ROUTE)
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

#[path = "filters.rs"]
mod filters;
pub use filters::SessionFilterArgs;

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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute =
            CliRoute::new(&["sessions"], "list").with_examples(&[CliExample::new(
                "Find recent sessions when you do not know the id",
                "everruns sessions list --limit 20",
            )]);
        Some(ROUTE)
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute =
            CliRoute::new(&["sessions"], "facets").with_examples(&[CliExample::new(
                "Break the session list down by status, agent and source",
                "everruns sessions facets --search triage",
            )]);
        Some(ROUTE)
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions"], "get")
            .with_args(&[CliArg::new("session_id").at(1).long("session")])
            .with_examples(&[CliExample::new(
                "Show one session's state and configuration",
                "everruns sessions get ses_01h9",
            )]);
        Some(ROUTE)
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions"], "context")
            .with_args(&[CliArg::new("session_id").at(1).long("session")])
            .with_examples(&[CliExample::new(
                "See what is filling a session's context window",
                "everruns sessions context ses_01h9",
            )]);
        Some(ROUTE)
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
#[path = "command_query_tests.rs"]
mod tests;

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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions"], "update")
            .with_args(&[CliArg::new("session_id").at(1).long("session")])
            .with_examples(&[CliExample::new(
                "Retitle a session so it is findable later",
                "everruns sessions update ses_01h9 --title 'Release triage'",
            )]);
        Some(ROUTE)
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions"], "delete")
            .with_args(&[CliArg::new("session_id").at(1).long("session")])
            .with_examples(&[CliExample::new(
                "Archive a session, keeping it restorable",
                "everruns sessions delete ses_01h9",
            )]);
        Some(ROUTE)
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute =
            CliRoute::new(&["sessions"], "stats").with_examples(&[CliExample::new(
                "Check token and cost totals across sessions",
                "everruns sessions stats",
            )]);
        Some(ROUTE)
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions"], "pin")
            .with_args(&[CliArg::new("session_id").at(1).long("session")])
            .with_examples(&[CliExample::new(
                "Keep a session at the top of the list",
                "everruns sessions pin ses_01h9",
            )]);
        Some(ROUTE)
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions"], "unpin")
            .with_args(&[CliArg::new("session_id").at(1).long("session")])
            .with_examples(&[CliExample::new(
                "Stop keeping a session at the top of the list",
                "everruns sessions unpin ses_01h9",
            )]);
        Some(ROUTE)
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions"], "archive")
            .with_args(&[CliArg::new("session_id").at(1).long("session")])
            .with_examples(&[CliExample::new(
                "Move a finished session out of the active list",
                "everruns sessions archive ses_01h9",
            )]);
        Some(ROUTE)
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions"], "unarchive")
            .with_args(&[CliArg::new("session_id").at(1).long("session")])
            .with_examples(&[CliExample::new(
                "Bring an archived session back to the active list",
                "everruns sessions unarchive ses_01h9",
            )]);
        Some(ROUTE)
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

    fn cli() -> Option<CliRoute> {
        // A const so the declared slices get 'static promotion:
        // `CliArg::new(..).short(..)` is a const fn, but an array of them
        // is only promoted inside a const initializer.
        const ROUTE: CliRoute = CliRoute::new(&["sessions"], "cancel")
            .with_args(&[CliArg::new("session_id").at(1).long("session")])
            .with_examples(&[CliExample::new(
                "Stop a session that is running away",
                "everruns sessions cancel ses_01h9",
            )]);
        Some(ROUTE)
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<CancelTurnResponse, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        let session = q::get_session(ctx, session_id, None).await?;

        if session.status != crate::records::SessionStatus::Active {
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
                InputMessageData::new(RuntimeMessage::user("User requested to cancel the work.")),
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
