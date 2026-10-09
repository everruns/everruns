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
// THREAT[TM-AGENTKEY-003]: event visibility filter.

use chrono::{DateTime, Utc};
use everruns_contracts::execution_phase::ExecutionPhase;
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::{
    INPUT_MESSAGE, OUTPUT_MESSAGE_COMPLETED, TOOL_COMPLETED, TOOL_STARTED, TURN_CANCELLED,
    TURN_COMPLETED, TURN_FAILED, TURN_STARTED,
};
use serde::Serialize;
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use super::ingress::{IngressChannel, IngressContext};
use super::record::api::{
    AGENT_KEY_PREFIX, AgentApiChannelConfig, AgentKeyPermission, ApiErrorDetail, ApiVisibility,
    agent_key_public_id, hash_agent_key,
};
use crate::domains::common::public_error::PublicError;
use crate::domains::common::CommandError;
use crate::domains::messages::types::{CreateMessageRequest, InputMessage, MessageRole};
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::domains::sessions::record::SessionSource;
use crate::domains::sessions::types::CreateSessionRequest;
use crate::execution_metadata;
use crate::storage::{EventRow, SessionRow, StorageBackend};

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
            },
        )
        .await?)
}

/// Event types each visibility reads from storage. `None` reads every type.
pub fn visible_event_types(visibility: ApiVisibility) -> Option<Vec<String>> {
    let mut types = vec![
        INPUT_MESSAGE,
        OUTPUT_MESSAGE_COMPLETED,
        TURN_STARTED,
        TURN_COMPLETED,
        TURN_FAILED,
        TURN_CANCELLED,
    ];
    match visibility {
        ApiVisibility::Full => return None,
        ApiVisibility::Activity => types.extend([TOOL_STARTED, TOOL_COMPLETED]),
        ApiVisibility::Messages => {}
    }
    Some(types.into_iter().map(str::to_string).collect())
}

/// One stored event as the caller may see it, or `None` when it is hidden.
pub fn project_event(row: &EventRow, config: &AgentApiChannelConfig) -> Option<Value> {
    if config.visibility == ApiVisibility::Full {
        let data = if config.errors == ApiErrorDetail::Public && row.event_type == TURN_FAILED {
            public_turn_failure(&row.data)
        } else {
            row.data.clone()
        };
        let mut value = envelope(row, data);
        value["metadata"] = row.metadata.clone().unwrap_or(Value::Null);
        value["tags"] = json!(row.tags);
        return Some(value);
    }
    let data = match row.event_type.as_str() {
        INPUT_MESSAGE | OUTPUT_MESSAGE_COMPLETED => visible_message(&row.data)?,
        TURN_STARTED | TURN_COMPLETED | TURN_CANCELLED => json!({ "turn_id": row.data["turn_id"] }),
        TURN_FAILED => match config.errors {
            ApiErrorDetail::Public => public_turn_failure(&row.data),
            ApiErrorDetail::Detailed => json!({
                "turn_id": row.data["turn_id"],
                "error": row.data["error"],
                "error_code": row.data["error_code"],
            }),
        },
        TOOL_STARTED if config.visibility == ApiVisibility::Activity => json!({
            "tool_call_id": row.data["tool_call"]["id"],
            "activity": config.activity_text(),
        }),
        TOOL_COMPLETED if config.visibility == ApiVisibility::Activity => json!({
            "tool_call_id": row.data["tool_call_id"],
            "success": row.data["success"],
        }),
        _ => return None,
    };
    // Event metadata and tags carry internal routing and execution detail, so
    // only `full` returns them.
    Some(envelope(row, data))
}

/// The canonical event envelope (`/v1/sessions/{id}/events`) around `data`.
fn envelope(row: &EventRow, data: Value) -> Value {
    json!({
        "id": row.id,
        "type": row.event_type,
        "ts": row.ts,
        "session_id": row.session_id,
        "sequence": row.sequence,
        "context": row.context,
        "data": data,
    })
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
    use everruns_contracts::typed_id::EventId;

    fn row(event_type: &str, data: Value) -> EventRow {
        EventRow {
            id: EventId::new(),
            session_id: SessionId::new(),
            sequence: 1,
            event_type: event_type.to_string(),
            ts: Utc::now(),
            context: json!({}),
            data,
            metadata: Some(json!({"acting_principal": "secret"})),
            tags: None,
            created_at: Utc::now(),
        }
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
        let final_answer = row(
            OUTPUT_MESSAGE_COMPLETED,
            assistant(
                "final_answer",
                json!([
                    {"type": "text", "text": "the answer"},
                    {"type": "tool_call", "id": "c1", "name": "secret_tool", "arguments": {"k": "v"}}
                ]),
            ),
        );
        let projected = project_event(&final_answer, &config).unwrap();
        let text = projected.to_string();
        assert!(text.contains("the answer"));
        assert!(!text.contains("secret_tool"));
        assert!(!text.contains("acting_principal"));

        let commentary = row(
            OUTPUT_MESSAGE_COMPLETED,
            assistant("commentary", json!([{"type": "text", "text": "thinking"}])),
        );
        assert!(project_event(&commentary, &config).is_none());

        let tool = row(
            TOOL_STARTED,
            json!({"tool_call": {"id": "c1", "name": "secret_tool"}}),
        );
        assert!(project_event(&tool, &config).is_none());
    }

    #[test]
    fn activity_visibility_shows_tools_without_names() {
        let config = config(ApiVisibility::Activity, ApiErrorDetail::Public);
        let started = row(
            TOOL_STARTED,
            json!({"tool_call": {"id": "c1", "name": "secret_tool", "arguments": {"k": "v"}}}),
        );
        let projected = project_event(&started, &config).unwrap();
        assert_eq!(projected["data"]["tool_call_id"], "c1");
        assert!(!projected.to_string().contains("secret_tool"));
        let completed = row(
            TOOL_COMPLETED,
            json!({"tool_call_id": "c1", "tool_name": "secret_tool", "success": true, "result": [{"type": "text", "text": "private"}]}),
        );
        let projected = project_event(&completed, &config).unwrap();
        assert_eq!(
            projected["data"],
            json!({"tool_call_id": "c1", "success": true})
        );
        assert!(project_event(&row("reason.started", json!({})), &config).is_none());
    }

    #[test]
    fn public_errors_hide_internal_failure_text() {
        let failed = row(
            TURN_FAILED,
            json!({"turn_id": "turn_1", "error": "db password wrong at 10.0.0.1", "error_code": "nope"}),
        );
        for visibility in [ApiVisibility::Messages, ApiVisibility::Full] {
            let projected =
                project_event(&failed, &config(visibility, ApiErrorDetail::Public)).unwrap();
            assert!(!projected.to_string().contains("10.0.0.1"));
            assert_eq!(projected["data"]["error_code"], "internal_error");
        }
        let detailed = project_event(
            &failed,
            &config(ApiVisibility::Messages, ApiErrorDetail::Detailed),
        )
        .unwrap();
        assert!(detailed.to_string().contains("10.0.0.1"));
    }

    #[test]
    fn full_visibility_keeps_raw_events() {
        let config = config(ApiVisibility::Full, ApiErrorDetail::Detailed);
        let started = row(
            TOOL_STARTED,
            json!({"tool_call": {"id": "c1", "name": "secret_tool"}}),
        );
        assert!(
            project_event(&started, &config)
                .unwrap()
                .to_string()
                .contains("secret_tool")
        );
        assert_eq!(visible_event_types(ApiVisibility::Full), None);
    }
}
