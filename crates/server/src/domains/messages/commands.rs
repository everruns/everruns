use super::queries as q;
use crate::api::messages::{Message, MessageRole};
use crate::domains::common::*;
use crate::domains::messages::CreateMessageContext;
use everruns_platform::{SessionParticipantKind, SessionParticipantRole};
use everruns_provider::typed_id::{AgentId, SessionId, SessionParticipantId};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMessage {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub message: crate::api::messages::InputMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addressed_participant_id: Option<SessionParticipantId>,
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
        if self.message.role != MessageRole::User {
            return Err(CommandError::bad_request(
                "Only user messages can be created through this endpoint",
            ));
        }

        let mut req = crate::api::messages::CreateMessageRequest {
            message: self.message,
            addressed_participant_id: self.addressed_participant_id,
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
        require_platform_chat_owner(ctx, &session).await?;
        let responder_agent_id = resolve_responder_agent_id(
            ctx,
            session_id,
            session.agent_id,
            req.addressed_participant_id,
        )
        .await?;

        q::message_service(ctx)?
            .create(
                CreateMessageContext {
                    org_id: ctx.org_id(),
                    user_id: ctx.caller.user_id,
                    harness_id: session.harness_id.uuid(),
                    agent_id: responder_agent_id.map(|id| id.uuid()),
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

async fn require_platform_chat_owner(
    ctx: &Ctx,
    session: &everruns_platform::Session,
) -> Result<(), CommandError> {
    let harness = ctx
        .db
        .get_harness(ctx.org_id(), session.harness_id)
        .await
        .map_err(classify_anyhow)?;
    let is_platform_chat = harness
        .as_ref()
        .is_some_and(|harness| harness.is_built_in && harness.name == "platform-chat");

    if !crate::domains::sessions::platform_chat_owner_matches(
        &ctx.caller,
        session,
        is_platform_chat,
    ) {
        return Err(CommandError::forbidden(
            "Only the Platform Chat session owner can create messages",
        ));
    }

    Ok(())
}

async fn resolve_responder_agent_id(
    ctx: &Ctx,
    session_id: SessionId,
    default_agent_id: Option<AgentId>,
    addressed_participant_id: Option<SessionParticipantId>,
) -> Result<Option<AgentId>, CommandError> {
    let Some(participant_id) = addressed_participant_id else {
        return Ok(default_agent_id);
    };

    let participants = ctx
        .db
        .list_session_participants(ctx.org_id(), session_id)
        .await
        .map_err(classify_anyhow)?;
    let participant = participants
        .iter()
        .find(|row| row.id == participant_id)
        .ok_or_else(|| CommandError::not_found("Participant"))?;

    if participant.left_at.is_some() {
        return Err(CommandError::conflict(
            "Addressed participant is no longer active",
        ));
    }
    if participant.kind != SessionParticipantKind::Agent.to_string() {
        return Err(CommandError::bad_request(
            "Addressed participant must be an agent",
        ));
    }

    let agent_id = participant.agent_id.ok_or_else(|| {
        CommandError::internal(anyhow::anyhow!("agent participant missing agent_id"))
    })?;

    if participant.role != SessionParticipantRole::Host.to_string()
        && participant.role != SessionParticipantRole::Member.to_string()
    {
        return Err(CommandError::bad_request(
            "Addressed participant has an unsupported role",
        ));
    }

    let public_id = ctx
        .db
        .get_agent_public_id(ctx.org_id(), agent_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::internal(anyhow::anyhow!("participant agent not found")))?
        .parse::<AgentId>()
        .map_err(|err| CommandError::internal(anyhow::anyhow!(err)))?;

    Ok(Some(public_id))
}

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
    /// Image content parts that could not be materialized into an ATIF image
    /// ContentPart and were flattened to `"[image]"` markers in the ATIF
    /// document (typically 0, since most images export as content parts; always
    /// 0 for the JSONL format, which keeps parts verbatim). Surfaced by the HTTP
    /// route as the `X-Atif-Images-Omitted` header.
    pub atif_images_omitted: usize,
}

/// Output format for session export.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionExportFormat {
    /// One materialized message per line (`application/x-ndjson`).
    #[default]
    Jsonl,
    /// A single ATIF trajectory document folded from the session's event log
    /// (`application/json`). See `knowledge/evaluation/atif-adoption.md`.
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

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_VIEW)
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
            let body = serde_json::to_string(&trajectory.document)
                .map_err(|e| CommandError::internal(e.into()))?;
            return Ok(ExportSessionJsonl {
                body,
                atif_images_omitted: trajectory.images_omitted,
            });
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

        Ok(ExportSessionJsonl {
            body,
            atif_images_omitted: 0,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<ExportSessionMessages>() }

/// One byte-bounded segment of a segmented ATIF session export, ready for the
/// HTTP route.
#[derive(Debug)]
pub struct SegmentExport {
    /// Serialized standalone ATIF-v1.7 document for this segment.
    pub body: String,
    /// Images flattened to markers within this segment (per-segment
    /// `X-Atif-Images-Omitted`).
    pub images_omitted: usize,
    /// Opaque cursor for the next segment, or `None` on the final/only segment.
    pub next_cursor: Option<String>,
    /// 0-based index of this segment in the chain.
    pub segment_index: usize,
}

/// Build one segment of a segmented ATIF export.
///
/// Deliberately NOT a registered `Command`: segmentation is an HTTP-only
/// concern, so it is kept off the MCP/CLI scripting catalog (which exposes the
/// whole-document `export_session_messages`). It still reuses the same
/// org-scoped session resolution and event fetch as `ExportSessionMessages`, so
/// tenant scoping is identical — the session is resolved from the path and the
/// cursor only selects a step offset within THAT session.
pub async fn export_session_segment(
    ctx: &Ctx,
    session_id_str: &str,
    cursor: Option<&str>,
    max_bytes: usize,
    link_base: &str,
) -> Result<SegmentExport, CommandError> {
    let session_id = q::parse_session_id(session_id_str)?;
    q::session_service(ctx)?
        .get(&ctx.caller, session_id.uuid(), None)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Session"))?;

    if let Some(raw_cursor) = cursor {
        // THREAT[TM-DOS/TM-API]: reject malformed, oversized, or foreign-session
        // cursors before loading the session event log or folding ATIF steps.
        crate::atif::decode_segment_cursor(raw_cursor, &session_id.to_string()).map_err(|e| {
            CommandError::bad_request(e.to_string()).with_code("atif_cursor_invalid")
        })?;
    }

    let event_service = crate::services::EventService::new(
        ctx.db.clone(),
        crate::event_delivery::EventDelivery::in_memory(),
    );
    let events = event_service
        .list(session_id.uuid(), None, None, &[], &[], None, None)
        .await
        .map_err(classify_anyhow)?;

    let segment = crate::atif::build_segment(
        &session_id.to_string(),
        &events,
        crate::atif::AtifOptions::default(),
        cursor,
        max_bytes,
        link_base,
    )
    .map_err(|e| CommandError::bad_request(e.to_string()).with_code("atif_cursor_invalid"))?;

    let body =
        serde_json::to_string(&segment.document).map_err(|e| CommandError::internal(e.into()))?;
    Ok(SegmentExport {
        body,
        images_omitted: segment.images_omitted,
        next_cursor: segment.next_cursor,
        segment_index: segment.segment_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::messages::{InputContentPart, InputMessage};
    use crate::domains::sessions::SessionService;
    use crate::event_delivery::EventDelivery;
    use crate::storage::StorageBackend;
    use crate::storage::models::{
        AgentRow, CreateAgentRow, CreateHarnessRow, CreateSessionParticipantRow, CreateSessionRow,
        SessionParticipantRow, SessionRow,
    };
    use async_trait::async_trait;
    use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole};
    use everruns_provider::typed_id::PrincipalId;
    use everruns_provider::typed_id::{HarnessId, MessageId};
    use everruns_scale::RunController;
    use std::sync::{Arc, Mutex};
    use tokio::time::{Duration, sleep};
    use uuid::Uuid;

    #[derive(Default)]
    struct RecordingRunner {
        calls: Mutex<Vec<Option<AgentId>>>,
    }

    impl RecordingRunner {
        fn calls(&self) -> Vec<Option<AgentId>> {
            self.calls.lock().expect("runner calls lock").clone()
        }
    }

    #[async_trait]
    impl RunController for RecordingRunner {
        async fn start_run(
            &self,
            _org_id: i64,
            _session_id: SessionId,
            _harness_id: HarnessId,
            agent_id: Option<AgentId>,
            _input_message_id: MessageId,
            _request_id: Option<String>,
        ) -> anyhow::Result<()> {
            self.calls.lock().expect("runner calls lock").push(agent_id);
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
            self.calls.lock().expect("runner calls lock").len()
        }
    }

    struct RoutingFixture {
        ctx: Ctx,
        runner: Arc<RecordingRunner>,
        host_agent: AgentRow,
        guest_agent: AgentRow,
        session: SessionRow,
        guest_participant: SessionParticipantRow,
        user_participant: SessionParticipantRow,
    }

    async fn setup_routing_fixture() -> RoutingFixture {
        let db = Arc::new(StorageBackend::in_memory());
        let harness = db
            .create_harness(
                DEFAULT_ORG_ID,
                CreateHarnessRow {
                    name: "routing-harness".to_string(),
                    display_name: None,
                    description: None,
                    system_prompt: Some("You are helpful.".to_string()),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    embedder_metadata: serde_json::json!({}),
                    is_built_in: false,
                },
            )
            .await
            .expect("create harness");

        let host_agent = seed_agent(&db, harness.id, "routing-host").await;
        let guest_agent = seed_agent(&db, harness.id, "routing-guest").await;
        let session = db
            .create_session(CreateSessionRow {
                source: everruns_platform::SessionSource::Api,
                org_id: DEFAULT_ORG_ID,
                app_id: None,
                harness_id: Some(harness.id),
                agent_id: Some(host_agent.id),
                agent_version_id: None,
                agent_config_hash: None,
                agent_identity_id: None,
                owner_principal_id: PrincipalId::from_seed(DEFAULT_ORG_ID as u128),
                resolved_owner_user_id: None,
                title: Some("Routing session".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                system_prompt: None,
                initial_files: serde_json::json!([]),
                hints: None,
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                blueprint_id: None,
                blueprint_config: None,
                parent_session_id: None,
                budget_root_session_id: None,
                workspace_id: None,
            })
            .await
            .expect("create session");
        let guest_participant = db
            .create_session_participant(CreateSessionParticipantRow {
                org_id: DEFAULT_ORG_ID,
                session_id: session.id,
                kind: SessionParticipantKind::Agent,
                agent_id: Some(guest_agent.id),
                agent_version_id: None,
                principal_id: session.owner_principal_id,
                display_name: None,
                role: SessionParticipantRole::Member,
                joined_at: None,
            })
            .await
            .expect("create guest participant");
        let user_participant = db
            .list_session_participants(DEFAULT_ORG_ID, session.id)
            .await
            .expect("list participants")
            .into_iter()
            .find(|row| row.kind == SessionParticipantKind::User.to_string())
            .expect("user participant");

        let runner = Arc::new(RecordingRunner::default());
        let runner_trait: Arc<dyn RunController> = runner.clone();
        let message_service = Arc::new(crate::domains::messages::MessageService::new(
            db.clone(),
            runner_trait,
            false,
            EventDelivery::in_memory(),
        ));
        let ctx = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db.clone(), None)
            .with_session_service(Arc::new(SessionService::new(db)))
            .with_message_service(message_service);

        RoutingFixture {
            ctx,
            runner,
            host_agent,
            guest_agent,
            session,
            guest_participant,
            user_participant,
        }
    }

    async fn seed_agent(db: &StorageBackend, harness_id: HarnessId, name: &str) -> AgentRow {
        db.create_agent(
            DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: name.to_string(),
                display_name: None,
                description: None,
                system_prompt: format!("You are {name}."),
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
        .expect("create agent")
    }

    fn user_input(text: &str) -> InputMessage {
        InputMessage {
            role: MessageRole::User,
            content: vec![InputContentPart::text(text)],
        }
    }

    fn create_message_command(
        session_id: SessionId,
        addressed_participant_id: Option<SessionParticipantId>,
    ) -> CreateMessage {
        CreateMessage {
            session_id: session_id.to_string(),
            message: user_input("hello"),
            addressed_participant_id,
            controls: None,
            metadata: None,
            tags: None,
            external_actor: None,
            request_id: None,
        }
    }

    async fn platform_chat_owner_fixture(
        caller_user_id: Uuid,
        owner_user_id: Uuid,
    ) -> (Ctx, everruns_platform::Session) {
        let db = Arc::new(StorageBackend::in_memory());
        let harness = db
            .create_harness(
                DEFAULT_ORG_ID,
                CreateHarnessRow {
                    name: "platform-chat".to_string(),
                    display_name: Some("Platform Chat".to_string()),
                    description: None,
                    system_prompt: None,
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    network_access: None,
                    embedder_metadata: serde_json::json!({}),
                    is_built_in: true,
                },
            )
            .await
            .expect("create Platform Chat harness");
        let row = db
            .create_session(CreateSessionRow {
                source: everruns_platform::SessionSource::Api,
                org_id: DEFAULT_ORG_ID,
                app_id: None,
                harness_id: Some(harness.id),
                agent_id: None,
                agent_version_id: None,
                agent_config_hash: None,
                agent_identity_id: None,
                owner_principal_id: PrincipalId::from_seed(owner_user_id.as_u128()),
                resolved_owner_user_id: Some(owner_user_id),
                title: Some("Platform Chat".to_string()),
                locale: None,
                tags: vec!["global-chat".to_string()],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                system_prompt: None,
                initial_files: serde_json::json!([]),
                hints: None,
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                blueprint_id: None,
                blueprint_config: None,
                parent_session_id: None,
                budget_root_session_id: None,
                workspace_id: None,
            })
            .await
            .expect("create Platform Chat session");
        let caller = Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: everruns_core::organization::org_public_id_from_internal(DEFAULT_ORG_ID),
            user_id: Some(caller_user_id),
            role: OrgRole::Member,
            is_platform_user: false,
            is_internal: false,
        };
        let service = SessionService::new(db.clone());
        let session = service
            .get(&caller, row.id.uuid(), None)
            .await
            .expect("get Platform Chat session")
            .expect("Platform Chat session exists");
        (Ctx::minimal_for_test(caller, db, None), session)
    }

    #[tokio::test]
    async fn platform_chat_rejects_messages_from_non_owner() {
        let (ctx, session) = platform_chat_owner_fixture(Uuid::new_v4(), Uuid::new_v4()).await;

        let err = require_platform_chat_owner(&ctx, &session)
            .await
            .expect_err("another org member must not drive the owner's Platform Chat");

        assert_eq!(err.status().as_u16(), 403);
    }

    #[tokio::test]
    async fn platform_chat_accepts_messages_from_owner() {
        let owner_user_id = Uuid::new_v4();
        let (ctx, session) = platform_chat_owner_fixture(owner_user_id, owner_user_id).await;

        require_platform_chat_owner(&ctx, &session)
            .await
            .expect("owner may drive their Platform Chat");
    }

    async fn wait_for_runner_calls(runner: &RecordingRunner, expected_len: usize) {
        for _ in 0..20 {
            if runner.calls().len() >= expected_len {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "runner recorded {} calls, expected at least {expected_len}",
            runner.calls().len()
        );
    }

    #[tokio::test]
    async fn unaddressed_turn_routes_to_host_agent() {
        let fixture = setup_routing_fixture().await;

        create_message_command(fixture.session.id, None)
            .execute(&fixture.ctx)
            .await
            .expect("create message");

        wait_for_runner_calls(&fixture.runner, 1).await;
        assert_eq!(
            fixture.runner.calls(),
            vec![Some(
                fixture
                    .host_agent
                    .public_id
                    .parse()
                    .expect("host public id")
            )]
        );
    }

    #[tokio::test]
    async fn addressed_turn_routes_to_guest_agent() {
        let fixture = setup_routing_fixture().await;

        create_message_command(fixture.session.id, Some(fixture.guest_participant.id))
            .execute(&fixture.ctx)
            .await
            .expect("create message");

        wait_for_runner_calls(&fixture.runner, 1).await;
        assert_eq!(
            fixture.runner.calls(),
            vec![Some(
                fixture
                    .guest_agent
                    .public_id
                    .parse()
                    .expect("guest public id")
            )]
        );
    }

    #[tokio::test]
    async fn addressed_user_participant_is_rejected() {
        let fixture = setup_routing_fixture().await;

        let err = create_message_command(fixture.session.id, Some(fixture.user_participant.id))
            .execute(&fixture.ctx)
            .await
            .expect_err("user participant cannot be addressed for agent routing");

        assert_eq!(err.status().as_u16(), 400);
        assert!(fixture.runner.calls().is_empty());
    }

    #[tokio::test]
    async fn left_guest_participant_is_rejected() {
        let fixture = setup_routing_fixture().await;
        fixture
            .ctx
            .db
            .leave_session_participant(
                DEFAULT_ORG_ID,
                fixture.session.id,
                fixture.guest_participant.id,
            )
            .await
            .expect("leave guest");

        let err = create_message_command(fixture.session.id, Some(fixture.guest_participant.id))
            .execute(&fixture.ctx)
            .await
            .expect_err("left participant cannot be addressed");

        assert_eq!(err.status().as_u16(), 409);
        assert!(fixture.runner.calls().is_empty());
    }

    #[tokio::test]
    async fn agent_role_input_is_rejected_before_turn_starts() {
        let fixture = setup_routing_fixture().await;
        let mut command = create_message_command(fixture.session.id, None);
        command.message.role = MessageRole::Agent;

        let err = command
            .execute(&fixture.ctx)
            .await
            .expect_err("agent input role cannot create a user turn");

        assert_eq!(err.status().as_u16(), 400);
        assert!(fixture.runner.calls().is_empty());
    }
}
