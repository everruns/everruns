// Agent Execution API on an `api` channel: who is calling, which sessions are
// theirs, and what of a session's events they may see.
//
// Decisions (see knowledge/integrations/agent-execution-api.md):
// - Order of checks: the channel resolves and is live, then the agent key, then
//   the rate limit, then the session. A wrong key and a missing channel are both
//   generic, so a caller cannot probe which channels exist with a bad key.
// - A session belongs to a caller when it was started by this channel
//   (`sessions.channel_id`, which only ingress sets) and carries the caller's
//   key tag. Both are required: tags alone can be written through the
//   management API, the channel id cannot. A session of another caller or
//   another channel answers 404, like one that does not exist.
// - Visibility is applied to the stored events, never to a copy the caller
//   could ask for in another way: `messages` and `activity` drop reasoning,
//   tool arguments and results, usage and internal error text.
// THREAT[TM-AGENTKEY-002]: caller confinement by channel id and key tag.
// - Questions and held-back tool calls surface at every visibility, since the
//   caller is who must answer them. A held-back call is answered by an
//   operator unless the channel's `tool_approvals` is `caller`.
// THREAT[TM-AGENTKEY-003]: event visibility filter.
// THREAT[TM-AGENTKEY-006]: who may approve a held-back tool call.

use chrono::{DateTime, Utc};
use everruns_contracts::execution_phase::ExecutionPhase;
use everruns_contracts::tool_types::ToolApprovalRequired;
use everruns_contracts::typed_id::SessionId;
use everruns_core::Event;
use everruns_core::builtins::ask_user::ASK_USER_TOOL_NAME;
use everruns_core::events::{
    INPUT_MESSAGE, OUTPUT_MESSAGE_COMPLETED, TOOL_CALL_REQUESTED, TOOL_COMPLETED, TOOL_STARTED,
    TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED, TURN_STARTED,
};
use serde::Serialize;
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use super::ingress::{IngressChannel, IngressContext};
use super::record::api::{
    AGENT_KEY_PREFIX, AgentApiChannelConfig, AgentKeyPermission, ApiErrorDetail, ApiToolApprovals,
    ApiVisibility, agent_key_public_id, hash_agent_key,
};
use crate::domains::common::CommandError;
use crate::domains::common::public_error::PublicError;
use crate::domains::messages::types::{CreateMessageRequest, InputMessage, MessageRole};
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::domains::sessions::record::{SessionSource, SessionStatus};
use crate::domains::sessions::types::CreateSessionRequest;
use crate::execution_metadata;
use crate::storage::{SessionRow, StorageBackend};

/// The caller behind one request: an agent key of this channel.
#[derive(Debug, Clone)]
pub struct ApiCaller {
    pub key_id: Uuid,
    pub config: AgentApiChannelConfig,
}

impl ApiCaller {
    /// Tags every session this caller starts carries.
    pub fn session_tags(&self, channel: &IngressChannel) -> Vec<String> {
        vec![
            format!("api_channel:{}", channel.public_id),
            format!("api_key:{}", agent_key_public_id(self.key_id)),
        ]
    }
}

/// Why a request was turned away before reaching a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiAuthError {
    /// No agent key, an unknown one, or one of another channel.
    Unauthorized,
    /// The channel's stored config does not parse.
    Misconfigured,
}

/// Resolve the agent key in `bearer` to a caller of `channel`.
pub async fn authorize_agent_key(
    db: &StorageBackend,
    context: &IngressContext,
    channel: &IngressChannel,
    bearer: Option<&str>,
) -> anyhow::Result<Result<ApiCaller, ApiAuthError>> {
    let Ok(config) =
        serde_json::from_value::<AgentApiChannelConfig>(channel.channel_config.clone())
    else {
        return Ok(Err(ApiAuthError::Misconfigured));
    };
    let Some(secret) = bearer.filter(|token| token.starts_with(AGENT_KEY_PREFIX)) else {
        return Ok(Err(ApiAuthError::Unauthorized));
    };
    // THREAT[TM-AGENTKEY-001]: only the SHA-256 hash is stored; the lookup is
    // an indexed equality on that hash, so the secret is never compared in a
    // data-dependent loop.
    let Some(key) = db.find_agent_key_by_hash(&hash_agent_key(secret)).await? else {
        return Ok(Err(ApiAuthError::Unauthorized));
    };
    let granted = key.org_id == context.org_id
        && key.channel_id == channel.internal_id
        && key
            .permissions
            .iter()
            .any(|p| p == AgentKeyPermission::Sessions.as_str());
    if !granted {
        return Ok(Err(ApiAuthError::Unauthorized));
    }
    if let Err(error) = db.touch_agent_key(key.id).await {
        tracing::warn!(%error, "failed to record agent key use");
    }
    Ok(Ok(ApiCaller {
        key_id: key.id,
        config,
    }))
}

/// Whether `session` belongs to `caller` on `channel`.
pub fn session_is_callers(
    session: &SessionRow,
    channel: &IngressChannel,
    caller: &ApiCaller,
) -> bool {
    session.channel_id == Some(channel.internal_id)
        && session.archived_at.is_none()
        && caller
            .session_tags(channel)
            .iter()
            .all(|tag| session.tags.contains(tag))
}

/// A session as an execution caller sees it: no instructions, tools, owners
/// or costs, which are management's.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentSessionView {
    #[schema(example = "session_01933b5a000070008000000000000001")]
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `started`, `active`, `idle`, `waitingforinput` or `failed`.
    #[schema(example = "idle")]
    pub status: String,
    /// Outcome of the last finished turn: `completed`, `failed` or `cancelled`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_turn_status: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Question sets the session waits on. Only `GET …/sessions/{id}` reads
    /// them; a list leaves them out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_questions: Option<Vec<PendingQuestionView>>,
    /// Held-back tool calls the session waits on. Same rule as
    /// `pending_questions`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_approvals: Option<Vec<PendingApprovalView>>,
}

impl AgentSessionView {
    pub fn with_pending(mut self, pending: PendingInput) -> Self {
        self.pending_questions = Some(pending.pending_questions);
        self.pending_approvals = Some(pending.pending_approvals);
        self
    }
}

impl From<&SessionRow> for AgentSessionView {
    fn from(row: &SessionRow) -> Self {
        Self {
            id: row.id.to_string(),
            title: row.title.clone(),
            status: row.status.clone(),
            last_turn_status: row.last_turn_status.clone(),
            created_at: row.created_at,
            updated_at: row.updated_at,
            pending_questions: None,
            pending_approvals: None,
        }
    }
}

/// Start a session for `caller`. It runs as the channel, like every other
/// channel session, and is tagged so only this caller reaches it.
pub async fn create_api_session(
    db: &StorageBackend,
    session_service: &SessionService,
    context: &IngressContext,
    channel: &IngressChannel,
    caller: &ApiCaller,
    title: Option<String>,
) -> Result<SessionRow, CommandError> {
    if title.as_ref().is_some_and(|t| t.chars().count() > 200) {
        return Err(CommandError::bad_request(
            "title must be at most 200 characters",
        ));
    }
    let session = session_service
        .create_from_app(
            &everruns_core::Caller::internal(context.org_id),
            context.harness_id.uuid(),
            Some(context.agent_internal_id),
            context.agent_id,
            context.historical_app_id,
            Some(channel.internal_id),
            None,
            context.owner_principal_id,
            context.resolved_owner_user_id,
            SessionSource::Api,
            CreateSessionRequest {
                playground_user_id: None,
                source: None,
                workspace_id: None,
                harness_id: Some(context.harness_id),
                harness_name: None,
                agent_id: context.agent_id,
                agent_name: None,
                virtual_user_id: context.virtual_user_id,
                title,
                goal: None,
                locale: None,
                tags: caller.session_tags(channel),
                model_id: None,
                capabilities: vec![],
                sandbox: None,
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
                fork_up_to_sequence: None,
            },
        )
        .await?;
    db.get_session(context.org_id, session.id)
        .await?
        .ok_or_else(|| CommandError::not_found("Session"))
}

/// Send the caller's message: starts a turn when the session is idle and
/// steers it when one is running. Text parts only for now, as the card says.
pub async fn send_api_message(
    message_service: &MessageService,
    context: &IngressContext,
    channel: &IngressChannel,
    caller: &ApiCaller,
    session_id: SessionId,
    message: InputMessage,
    request_id: Option<String>,
) -> Result<crate::domains::messages::types::Message, CommandError> {
    if message.role != MessageRole::User {
        return Err(CommandError::bad_request("only user messages can be sent"));
    }
    if message.content.is_empty()
        || !message
            .content
            .iter()
            .all(|part| matches!(part, everruns_core::InputContentPart::Text(_)))
    {
        return Err(CommandError::bad_request(
            "message content must be one or more text parts",
        ));
    }
    let metadata = [
        ("source", json!("api_channel")),
        ("api_channel_id", json!(channel.public_id.to_string())),
        ("api_key_id", json!(agent_key_public_id(caller.key_id))),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    Ok(message_service
        .create(
            CreateMessageContext {
                runtime_subject_principal_id: None,
                org_id: context.org_id,
                user_id: None,
                harness_id: context.harness_id.uuid(),
                agent_id: Some(context.agent_internal_id),
                session_id: session_id.uuid(),
                event_metadata: Some(execution_metadata::channel_message_metadata(
                    context.public_id,
                    context.owner_principal_id,
                    context.virtual_user_id,
                )),
                request_id,
            },
            CreateMessageRequest {
                message,
                addressed_participant_id: None,
                controls: None,
                metadata: Some(metadata),
                tags: None,
                external_actor: None,
                client_message_id: None,
            },
        )
        .await?)
}

/// Event types each visibility reads from storage. `None` reads every type.
///
/// `tool.call_requested` is read at every visibility because a question or an
/// approval the caller must see arrives in it; [`project_event`] keeps only
/// those calls.
pub fn visible_event_types(visibility: ApiVisibility) -> Option<Vec<String>> {
    let mut types = vec![
        INPUT_MESSAGE,
        OUTPUT_MESSAGE_COMPLETED,
        TURN_STARTED,
        TURN_COMPLETED,
        TURN_FAILED,
        TURN_CANCELLED,
        TOOL_CALL_REQUESTED,
    ];
    match visibility {
        ApiVisibility::Full => return None,
        ApiVisibility::Activity => types.extend([TOOL_STARTED, TOOL_COMPLETED]),
        ApiVisibility::Messages => {}
    }
    Some(types.into_iter().map(str::to_string).collect())
}

/// An event as every API surface publishes it: the canonical envelope after
/// [`Event::into_public`], which strips reasoning replay state. Projection
/// starts from this, so no visibility can return more than `/v1` would.
pub fn public_event_json(event: &Event) -> Option<Value> {
    if event.data.needs_public_projection() {
        serde_json::to_value(event.clone().into_public()).ok()
    } else {
        serde_json::to_value(event).ok()
    }
}

/// One public event envelope ([`public_event_json`]) as the caller may see
/// it, or `None` when it is hidden.
pub fn project_event(mut event: Value, config: &AgentApiChannelConfig) -> Option<Value> {
    let event_type = event["type"].as_str()?.to_string();
    if config.visibility == ApiVisibility::Full {
        if config.errors == ApiErrorDetail::Public && event_type == TURN_FAILED {
            event["data"] = public_turn_failure(&event["data"]);
        }
        return Some(event);
    }
    let raw = &event["data"];
    let data = match event_type.as_str() {
        INPUT_MESSAGE | OUTPUT_MESSAGE_COMPLETED => visible_message(raw)?,
        TURN_STARTED | TURN_COMPLETED | TURN_CANCELLED => json!({ "turn_id": raw["turn_id"] }),
        TURN_FAILED => match config.errors {
            ApiErrorDetail::Public => public_turn_failure(raw),
            ApiErrorDetail::Detailed => json!({
                "turn_id": raw["turn_id"],
                "error": raw["error"],
                "error_code": raw["error_code"],
            }),
        },
        TOOL_STARTED if config.visibility == ApiVisibility::Activity => json!({
            "tool_call_id": raw["tool_call"]["id"],
            "activity": config.activity_text(),
        }),
        TOOL_COMPLETED if config.visibility == ApiVisibility::Activity => json!({
            "tool_call_id": raw["tool_call_id"],
            "success": raw["success"],
        }),
        TOOL_CALL_REQUESTED => {
            let pending = pending_input(raw, config);
            if pending.is_empty() {
                return None;
            }
            serde_json::to_value(pending).ok()?
        }
        _ => return None,
    };
    // Event metadata and tags carry internal routing and execution detail, so
    // only `full` returns them.
    Some(json!({
        "id": event["id"],
        "type": event["type"],
        "ts": event["ts"],
        "session_id": event["session_id"],
        "sequence": event["sequence"],
        "context": event["context"],
        "data": data,
    }))
}

/// An `ask_user` question set the session waits on. Same shape as serve's
/// `pending_questions` item.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PendingQuestionView {
    /// The `ask_user` call to name in `POST …/question-answers`.
    pub tool_call_id: String,
    /// The questions as asked: `kind`, `id`, `header`, `question`, `options`,
    /// `multi_select`, `allow_other`.
    #[schema(value_type = Vec<Object>)]
    pub questions: Value,
    /// When the server stops waiting (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// A held-back tool call the session waits on. Same shape as serve's
/// `pending_approvals` item, plus `answerable`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PendingApprovalView {
    /// The request to name in `POST …/tool-approvals`.
    pub tool_call_id: String,
    /// Whether this caller may answer it. When `false` an operator decides in
    /// Everruns and the tool is not shown.
    pub answerable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub arguments: Option<Value>,
    /// `arguments` is a truncated preview; the decision still binds to the
    /// full arguments.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub arguments_truncated: bool,
    /// Why the gate asked: `destructive`, `open_world`, `mutating` or `policy`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
    /// When the request counts as rejected (RFC 3339).
    pub expires_at: String,
}

/// What a parked session waits on from outside.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct PendingInput {
    pub pending_questions: Vec<PendingQuestionView>,
    pub pending_approvals: Vec<PendingApprovalView>,
}

impl PendingInput {
    pub fn is_empty(&self) -> bool {
        self.pending_questions.is_empty() && self.pending_approvals.is_empty()
    }
}

/// The questions and approvals in a `tool.call_requested` payload. Every
/// other call in the batch is the agent's own and stays hidden.
pub fn pending_input(requested: &Value, config: &AgentApiChannelConfig) -> PendingInput {
    let mut pending = PendingInput::default();
    let Some(calls) = requested["tool_calls"].as_array() else {
        return pending;
    };
    let answerable = config.tool_approvals == ApiToolApprovals::Caller;
    for call in calls {
        let (Some(id), Some(name)) = (call["id"].as_str(), call["name"].as_str()) else {
            continue;
        };
        if name == ASK_USER_TOOL_NAME {
            pending.pending_questions.push(PendingQuestionView {
                tool_call_id: id.to_string(),
                questions: call["arguments"]["questions"].clone(),
                expires_at: call["arguments"]["expires_at"].as_str().map(str::to_string),
            });
        } else if let Some(request) =
            ToolApprovalRequired::from_request_call(id, name, &call["arguments"])
        {
            // THREAT[TM-AGENTKEY-006]: a caller that may not answer does not
            // learn which tool waits or with what arguments either.
            pending.pending_approvals.push(PendingApprovalView {
                tool_call_id: id.to_string(),
                answerable,
                tool_name: answerable.then(|| request.tool.clone()),
                arguments: answerable.then(|| request.arguments.clone()),
                arguments_truncated: answerable && request.arguments_truncated,
                risk: answerable.then(|| request.risk.clone()),
                expires_at: request.expires_at,
            });
        }
    }
    pending
}

/// What `session` waits on, read from its last `tool.call_requested`.
pub async fn session_pending_input(
    db: &StorageBackend,
    session: &SessionRow,
    config: &AgentApiChannelConfig,
) -> anyhow::Result<PendingInput> {
    if SessionStatus::from(session.status.as_str()) != SessionStatus::WaitingForToolResults {
        return Ok(PendingInput::default());
    }
    let requested = db
        .list_events(
            session.id,
            None,
            None,
            &[TOOL_CALL_REQUESTED.to_string()],
            &[],
            None,
            Some(1),
        )
        .await?;
    Ok(requested
        .last()
        .map(|row| pending_input(&row.data, config))
        .unwrap_or_default())
}

/// A message event's text parts. Commentary and messages with no text are
/// hidden: they are the agent's working notes and tool calls.
fn visible_message(data: &Value) -> Option<Value> {
    let message = &data["message"];
    let commentary = serde_json::to_value(ExecutionPhase::Commentary).ok()?;
    if message["phase"] == commentary {
        return None;
    }
    let content: Vec<Value> = message["content"]
        .as_array()?
        .iter()
        .filter(|part| part["type"] == "text")
        .map(|part| json!({ "type": "text", "text": part["text"] }))
        .collect();
    if content.is_empty() {
        return None;
    }
    Some(json!({
        "message": {
            "id": message["id"],
            "role": message["role"],
            "content": content,
        }
    }))
}

fn public_turn_failure(data: &Value) -> Value {
    let public = PublicError::from_internal_code(data["error_code"].as_str());
    json!({
        "turn_id": data["turn_id"],
        "error": public.message,
        "error_code": public.code.as_str(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(event_type: &str, data: Value) -> Value {
        json!({
            "id": "event_01933b5a000070008000000000000001",
            "type": event_type,
            "ts": "2026-10-10T00:00:00Z",
            "session_id": "session_01933b5a000070008000000000000001",
            "sequence": 1,
            "context": {},
            "data": data,
            "metadata": {"acting_principal": "secret"},
        })
    }

    fn config(visibility: ApiVisibility, errors: ApiErrorDetail) -> AgentApiChannelConfig {
        AgentApiChannelConfig {
            visibility,
            errors,
            ..Default::default()
        }
    }

    fn assistant(phase: &str, content: Value) -> Value {
        json!({"message": {"id": "message_1", "role": "assistant", "content": content, "phase": phase}})
    }

    #[test]
    fn messages_visibility_shows_only_final_text() {
        let config = config(ApiVisibility::Messages, ApiErrorDetail::Public);
        let final_answer = event(
            OUTPUT_MESSAGE_COMPLETED,
            assistant(
                "final_answer",
                json!([
                    {"type": "text", "text": "the answer"},
                    {"type": "tool_call", "id": "c1", "name": "secret_tool", "arguments": {"k": "v"}}
                ]),
            ),
        );
        let projected = project_event(final_answer, &config).unwrap();
        let text = projected.to_string();
        assert!(text.contains("the answer"));
        assert!(!text.contains("secret_tool"));
        assert!(!text.contains("acting_principal"));

        let commentary = event(
            OUTPUT_MESSAGE_COMPLETED,
            assistant("commentary", json!([{"type": "text", "text": "thinking"}])),
        );
        assert!(project_event(commentary, &config).is_none());

        let tool = event(
            TOOL_STARTED,
            json!({"tool_call": {"id": "c1", "name": "secret_tool"}}),
        );
        assert!(project_event(tool, &config).is_none());
    }

    #[test]
    fn activity_visibility_shows_tools_without_names() {
        let config = config(ApiVisibility::Activity, ApiErrorDetail::Public);
        let started = event(
            TOOL_STARTED,
            json!({"tool_call": {"id": "c1", "name": "secret_tool", "arguments": {"k": "v"}}}),
        );
        let projected = project_event(started, &config).unwrap();
        assert_eq!(projected["data"]["tool_call_id"], "c1");
        assert!(!projected.to_string().contains("secret_tool"));
        let completed = event(
            TOOL_COMPLETED,
            json!({"tool_call_id": "c1", "tool_name": "secret_tool", "success": true, "result": [{"type": "text", "text": "private"}]}),
        );
        let projected = project_event(completed, &config).unwrap();
        assert_eq!(
            projected["data"],
            json!({"tool_call_id": "c1", "success": true})
        );
        assert!(project_event(event("reason.started", json!({})), &config).is_none());
    }

    #[test]
    fn public_errors_hide_internal_failure_text() {
        let failed = event(
            TURN_FAILED,
            json!({"turn_id": "turn_1", "error": "db password wrong at 10.0.0.1", "error_code": "nope"}),
        );
        for visibility in [ApiVisibility::Messages, ApiVisibility::Full] {
            let projected =
                project_event(failed.clone(), &config(visibility, ApiErrorDetail::Public)).unwrap();
            assert!(!projected.to_string().contains("10.0.0.1"));
            assert_eq!(projected["data"]["error_code"], "internal_error");
        }
        let detailed = project_event(
            failed,
            &config(ApiVisibility::Messages, ApiErrorDetail::Detailed),
        )
        .unwrap();
        assert!(detailed.to_string().contains("10.0.0.1"));
    }

    #[test]
    fn full_visibility_keeps_raw_events() {
        let config = config(ApiVisibility::Full, ApiErrorDetail::Detailed);
        let started = event(
            TOOL_STARTED,
            json!({"tool_call": {"id": "c1", "name": "secret_tool"}}),
        );
        assert!(
            project_event(started, &config)
                .unwrap()
                .to_string()
                .contains("secret_tool")
        );
        assert_eq!(visible_event_types(ApiVisibility::Full), None);
    }

    fn approval_call() -> Value {
        json!({
            "id": "tool_approval_call_1",
            "name": "approve_tool_call",
            "arguments": {
                "code": "tool_approval_required", "error": "Waiting", "tool_call_id": "call_1",
                "tool": "send_email", "arguments": {"to": "a@example.com"},
                "fingerprint": "sha256:ab", "risk": "open_world", "mode": "normal",
                "asked_at": "2026-10-01T00:00:00Z", "expires_at": "2026-10-01T00:15:00Z"
            }
        })
    }

    fn requested() -> Value {
        event(
            TOOL_CALL_REQUESTED,
            json!({"tool_calls": [
                {"id": "q1", "name": "ask_user", "arguments": {
                    "questions": [{"kind": "text", "id": "name", "header": "Name", "question": "Your name?"}],
                    "expires_at": "2026-10-01T00:05:00Z"
                }},
                approval_call(),
                {"id": "c9", "name": "internal_lookup", "arguments": {"secret": "s3cr3t"}}
            ]}),
        )
    }

    #[test]
    fn requested_calls_show_only_questions_and_approvals() {
        let config = config(ApiVisibility::Messages, ApiErrorDetail::Public);
        let projected = project_event(requested(), &config).unwrap();
        let data = &projected["data"];
        assert_eq!(data["pending_questions"][0]["tool_call_id"], "q1");
        assert_eq!(
            data["pending_questions"][0]["questions"][0]["question"],
            "Your name?"
        );
        // An operator decides by default: the caller sees that a call waits,
        // never which one.
        let approval = &data["pending_approvals"][0];
        assert_eq!(approval["tool_call_id"], "tool_approval_call_1");
        assert_eq!(approval["answerable"], false);
        assert!(!projected.to_string().contains("send_email"));
        assert!(!projected.to_string().contains("s3cr3t"));
        assert!(!projected.to_string().contains("internal_lookup"));

        let caller = AgentApiChannelConfig {
            tool_approvals: ApiToolApprovals::Caller,
            ..config.clone()
        };
        let projected = project_event(requested(), &caller).unwrap();
        let approval = &projected["data"]["pending_approvals"][0];
        assert_eq!(approval["answerable"], true);
        assert_eq!(approval["tool_name"], "send_email");
        assert_eq!(approval["arguments"]["to"], "a@example.com");
        assert!(!projected.to_string().contains("s3cr3t"));

        let only_agent_calls = event(
            TOOL_CALL_REQUESTED,
            json!({"tool_calls": [{"id": "c9", "name": "internal_lookup", "arguments": {}}]}),
        );
        assert!(project_event(only_agent_calls, &config).is_none());
    }
}
