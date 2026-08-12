//! Session Capability
//!
//! Provides session metadata tools:
//! - `write_session_title`: update session title
//! - `get_session_info`: return session id, title, agent name, and cumulative usage

use async_trait::async_trait;
use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::events::{EventContext, EventRequest, SessionTitleUpdatedData, TokenUsage};
use everruns_core::session::ExecutionSession;
use everruns_core::tool_types::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::{EventEmitter, SessionMutator, SessionStore, ToolContext};
use everruns_core::typed_id::SessionId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const SESSION_CAPABILITY_ID: &str = "session";

/// Per-agent/session settings for the session capability.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCapabilityConfig {
    /// Ask the model to maintain a concise title as the conversation evolves.
    #[serde(default)]
    pub auto_title: bool,
}

impl SessionCapabilityConfig {
    fn from_value(config: &Value) -> Self {
        serde_json::from_value(config.clone()).unwrap_or_default()
    }
}

/// Result of a semantic session-title mutation.
#[derive(Debug, Clone)]
pub struct SessionTitleMutation {
    /// Current session snapshot.
    pub session: ExecutionSession,
    /// Whether the stored title changed and an event was emitted.
    pub changed: bool,
}

/// Build the semantic event for an actual title change.
///
/// Returns `None` when `title` is already current, giving tool and non-tool
/// mutation paths one source of truth for no-op suppression and payload shape.
pub fn session_title_updated_event(
    session_id: SessionId,
    event_context: EventContext,
    previous_title: Option<String>,
    title: String,
) -> Option<EventRequest> {
    if previous_title.as_deref() == Some(title.as_str()) {
        return None;
    }
    Some(EventRequest::new(
        session_id,
        event_context,
        SessionTitleUpdatedData {
            previous_title,
            title,
        },
    ))
}

/// Update a session title and emit the corresponding semantic event.
///
/// Embedders and non-tool mutation paths can use this helper to share the same
/// change detection, typed payload, and correlation semantics as
/// [`WriteSessionTitleTool`]. The event is emitted only after an actual change;
/// an unchanged title is a successful no-op.
pub async fn update_session_title_with_event(
    session_id: SessionId,
    title: String,
    event_context: EventContext,
    session_store: &dyn SessionStore,
    session_mutator: &dyn SessionMutator,
    event_emitter: &dyn EventEmitter,
) -> Result<SessionTitleMutation> {
    let current = session_store
        .get_session(session_id)
        .await?
        .ok_or_else(|| AgentLoopError::store(format!("session not found: {session_id}")))?;
    let previous_title = current.title.clone();

    let Some(event) =
        session_title_updated_event(session_id, event_context, previous_title, title.clone())
    else {
        return Ok(SessionTitleMutation {
            session: current,
            changed: false,
        });
    };

    let session = session_mutator
        .update_session_title(session_id, title.clone())
        .await?;
    event_emitter.emit(event).await?;

    Ok(SessionTitleMutation {
        session,
        changed: true,
    })
}

/// Session capability - read/update session metadata.
pub struct SessionCapability;

#[async_trait]
impl Capability for SessionCapability {
    fn id(&self) -> &str {
        SESSION_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Session"
    }

    fn description(&self) -> &str {
        "Read and update current session metadata like title and agent info."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Сесія",
            "Читання та оновлення метаданих поточної сесії, як-от назви й інформації про агента.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("panel-left")
    }

    fn category(&self) -> Option<&str> {
        Some("Session")
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "auto_title": {
                    "type": "boolean",
                    "title": "Automatic session titles",
                    "description": "Require a concise title before handling the first substantive request and update it only when the conversation's primary theme materially changes.",
                    "default": false
                }
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &Value) -> std::result::Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        serde_json::from_value::<SessionCapabilityConfig>(config.clone())
            .map(|_| ())
            .map_err(|error| format!("invalid session config: {error}"))
    }

    async fn system_prompt_contribution_with_config(
        &self,
        _ctx: &everruns_core::capabilities::SystemPromptContext,
        config: &Value,
    ) -> Option<String> {
        if !SessionCapabilityConfig::from_value(config).auto_title {
            return None;
        }

        Some(format!(
            "<capability id=\"{}\">\nTitle maintenance is mandatory when automatic titles are enabled. On the first substantive user request, you MUST call `write_session_title` with a concise 3–7 word title for the conversation's primary theme before using any other tool, doing substantive work, or giving a substantive response. Ignore greetings, acknowledgements, and other non-substantive messages. On later turns, if the primary theme materially changes, you MUST call `write_session_title` before using any other tool, doing substantive work, or responding. You MUST NOT update the title for minor subtopics, follow-ups, or implementation details. Calling `write_session_title` updates session metadata only; it does not change project or workspace files and must not be treated as a project-file change.\n</capability>",
            self.id()
        ))
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(WriteSessionTitleTool),
            Box::new(GetSessionInfoTool),
        ]
    }
}

/// Tool: write_session_title
pub struct WriteSessionTitleTool;

#[async_trait]
impl Tool for WriteSessionTitleTool {
    fn narrate(
        &self,
        tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_write_session_title(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "write_session_title"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Write Session Title")
    }

    fn description(&self) -> &str {
        "Update the current session title."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "New session title"
                }
            },
            "required": ["title"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "write_session_title requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let title = match arguments.get("title").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => return ToolExecutionResult::tool_error("Missing required parameter: title"),
        };

        let Some(session_store) = &context.session_store else {
            return ToolExecutionResult::tool_error("Session store not available in this context");
        };
        let Some(mutator) = &context.session_mutator else {
            return ToolExecutionResult::tool_error(
                "Session mutator not available in this context",
            );
        };
        let Some(event_emitter) = &context.event_emitter else {
            return ToolExecutionResult::tool_error("Event emitter not available in this context");
        };

        match update_session_title_with_event(
            context.session_id,
            title,
            context.event_context.clone().unwrap_or_default(),
            session_store.as_ref(),
            mutator.as_ref(),
            event_emitter.as_ref(),
        )
        .await
        {
            Ok(outcome) => ToolExecutionResult::success(json!({
                "session_id": outcome.session.id.to_string(),
                "title": outcome.session.title,
                "updated": outcome.changed,
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }
}

/// Tool: get_session_info
pub struct GetSessionInfoTool;

#[async_trait]
impl Tool for GetSessionInfoTool {
    fn narrate(
        &self,
        _tool_call: &everruns_core::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_get_session_info(
            phase, locale,
        ))
    }

    fn name(&self) -> &str {
        "get_session_info"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get Session Info")
    }

    fn description(&self) -> &str {
        "Get current session metadata: id, title, locale, agent name, and cumulative token usage."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "get_session_info requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(session_store) = &context.session_store else {
            return ToolExecutionResult::tool_error("Session store not available in this context");
        };

        let session = match session_store.get_session(context.session_id).await {
            Ok(Some(session)) => session,
            Ok(None) => return ToolExecutionResult::tool_error("Session not found"),
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

        let agent_name = if let (Some(agent_id), Some(agent_store)) =
            (session.agent_id, &context.agent_store)
        {
            match agent_store.get_agent(agent_id).await {
                Ok(Some(agent)) => Some(agent.display_name.unwrap_or_else(|| agent.name.clone())),
                Ok(None) => None,
                Err(e) => return ToolExecutionResult::internal_error(e),
            }
        } else {
            None
        };

        ToolExecutionResult::success(json!({
            "session_id": session.id.to_string(),
            "title": session.title,
            "locale": session.locale,
            "agent_name": agent_name,
            "usage": session.usage.as_ref().map(usage_json),
        }))
    }
}

fn usage_json(usage: &TokenUsage) -> Value {
    json!({
        "input_tokens": usage.input_tokens,
        "output_tokens": usage.output_tokens,
        "cache_read_tokens": usage.cache_read_tokens,
        "cache_creation_tokens": usage.cache_creation_tokens,
        "total_tokens": usage.total_tokens(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_core::AgentDefinition;
    use everruns_core::events::{Event, EventRequest};
    use everruns_core::session::{ExecutionSession, SessionExecutionState};
    use everruns_core::typed_id::{
        AgentId, EventId, HarnessId, MessageId, ModelId, SessionId, TurnId,
    };
    use everruns_core::{AgentCapabilityConfig, Tool};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MockSessionStore {
        session: Arc<Mutex<Option<ExecutionSession>>>,
    }

    #[async_trait]
    impl everruns_core::traits::SessionStore for MockSessionStore {
        async fn get_session(&self, _session_id: SessionId) -> Result<Option<ExecutionSession>> {
            Ok(self.session.lock().expect("poisoned").clone())
        }
    }

    #[derive(Clone)]
    struct MockSessionMutator {
        session: Arc<Mutex<ExecutionSession>>,
    }

    #[async_trait]
    impl everruns_core::traits::SessionMutator for MockSessionMutator {
        async fn update_session_title(
            &self,
            _session_id: SessionId,
            title: String,
        ) -> Result<ExecutionSession> {
            let mut session = self.session.lock().expect("poisoned");
            session.title = Some(title);
            Ok(session.clone())
        }
    }

    struct MockAgentStore {
        agent: Option<AgentDefinition>,
    }

    #[derive(Clone, Default)]
    struct RecordingEventEmitter {
        requests: Arc<Mutex<Vec<EventRequest>>>,
    }

    #[async_trait]
    impl everruns_core::traits::EventEmitter for RecordingEventEmitter {
        async fn emit(&self, request: EventRequest) -> Result<Event> {
            self.requests
                .lock()
                .expect("poisoned")
                .push(request.clone());
            Ok(request.into_event(EventId::new(), 1))
        }
    }

    #[async_trait]
    impl everruns_core::traits::AgentStore for MockAgentStore {
        async fn get_agent(&self, _agent_id: AgentId) -> Result<Option<AgentDefinition>> {
            Ok(self.agent.clone())
        }
    }

    fn build_session(agent_id: Option<AgentId>) -> ExecutionSession {
        ExecutionSession {
            agent_id,
            title: Some("Old title".to_string()),
            model_id: Some(ModelId::new()),
            status: SessionExecutionState::Idle,
            ..ExecutionSession::with_own_workspace(SessionId::new(), HarnessId::new())
        }
    }

    #[test]
    fn session_tools_narrate_all_phases() {
        use everruns_core::tool_narration::{ToolNarrationContext, ToolNarrationPhase};
        use everruns_core::tool_types::ToolCall;

        let ctx = ToolNarrationContext::default();
        let title_call = ToolCall {
            id: "c1".to_string(),
            name: "write_session_title".to_string(),
            arguments: json!({ "title": "Improve tool narration" }),
        };
        assert_eq!(
            WriteSessionTitleTool.narrate(&title_call, ToolNarrationPhase::Started, None, ctx),
            Some("Updating session title: Improve tool narration".to_string())
        );
        assert_eq!(
            WriteSessionTitleTool.narrate(&title_call, ToolNarrationPhase::Completed, None, ctx),
            Some("Updated session title: Improve tool narration".to_string())
        );
        assert_eq!(
            WriteSessionTitleTool.narrate(&title_call, ToolNarrationPhase::Failed, None, ctx),
            Some("Failed to update session title: Improve tool narration".to_string())
        );

        // The capability surfaces the tool's narration via the default dispatch.
        let cap = SessionCapability;
        let def = WriteSessionTitleTool.to_definition();
        assert_eq!(
            cap.narrate(
                Some(&def),
                &title_call,
                ToolNarrationPhase::Started,
                None,
                ctx
            ),
            Some("Updating session title: Improve tool narration".to_string())
        );

        let info_call = ToolCall {
            id: "c2".to_string(),
            name: "get_session_info".to_string(),
            arguments: json!({}),
        };
        assert_eq!(
            GetSessionInfoTool.narrate(&info_call, ToolNarrationPhase::Completed, None, ctx),
            Some("Read session info".to_string())
        );
    }

    #[tokio::test]
    async fn write_session_title_updates_title() {
        let session = build_session(None);
        let session_id = session.id;
        let stored = Arc::new(Mutex::new(Some(session.clone())));
        let emitter = RecordingEventEmitter::default();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();
        let mut context = ToolContext::new(session_id);
        context.session_store = Some(Arc::new(MockSessionStore { session: stored }));
        context.session_mutator = Some(Arc::new(MockSessionMutator {
            session: Arc::new(Mutex::new(session)),
        }));
        context.event_emitter = Some(Arc::new(emitter.clone()));
        context.event_context = Some(EventContext::turn(turn_id, input_message_id));

        let tool = WriteSessionTitleTool;
        let result = tool
            .execute_with_context(json!({"title": "New title"}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["title"], "New title");
                assert_eq!(value["updated"], true);
            }
            _ => panic!("expected success"),
        }

        let requests = emitter.requests.lock().expect("poisoned");
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].event_type,
            everruns_core::events::SESSION_TITLE_UPDATED
        );
        assert_eq!(requests[0].context.turn_id, Some(turn_id));
        assert_eq!(requests[0].context.input_message_id, Some(input_message_id));
        match &requests[0].data {
            everruns_core::events::EventData::SessionTitleUpdated(data) => {
                assert_eq!(data.previous_title.as_deref(), Some("Old title"));
                assert_eq!(data.title, "New title");
            }
            data => panic!("unexpected event data: {data:?}"),
        }
    }

    #[tokio::test]
    async fn write_session_title_is_noop_when_title_is_unchanged() {
        let session = build_session(None);
        let session_id = session.id;
        let emitter = RecordingEventEmitter::default();
        let mut context = ToolContext::new(session_id);
        context.session_store = Some(Arc::new(MockSessionStore {
            session: Arc::new(Mutex::new(Some(session.clone()))),
        }));
        context.session_mutator = Some(Arc::new(MockSessionMutator {
            session: Arc::new(Mutex::new(session)),
        }));
        context.event_emitter = Some(Arc::new(emitter.clone()));

        let result = WriteSessionTitleTool
            .execute_with_context(json!({"title": "Old title"}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(value) => assert_eq!(value["updated"], false),
            _ => panic!("expected success"),
        }
        assert!(emitter.requests.lock().expect("poisoned").is_empty());
    }

    #[tokio::test]
    async fn auto_title_policy_is_opt_in_and_mandatory_when_enabled() {
        let capability = SessionCapability;
        let ctx =
            everruns_core::capabilities::SystemPromptContext::without_file_store(SessionId::new());

        assert!(
            capability
                .system_prompt_contribution_with_config(&ctx, &json!({}))
                .await
                .is_none()
        );
        let prompt = capability
            .system_prompt_contribution_with_config(&ctx, &json!({"auto_title": true}))
            .await
            .expect("auto-title prompt");
        assert_eq!(
            prompt,
            "<capability id=\"session\">\nTitle maintenance is mandatory when automatic titles are enabled. On the first substantive user request, you MUST call `write_session_title` with a concise 3–7 word title for the conversation's primary theme before using any other tool, doing substantive work, or giving a substantive response. Ignore greetings, acknowledgements, and other non-substantive messages. On later turns, if the primary theme materially changes, you MUST call `write_session_title` before using any other tool, doing substantive work, or responding. You MUST NOT update the title for minor subtopics, follow-ups, or implementation details. Calling `write_session_title` updates session metadata only; it does not change project or workspace files and must not be treated as a project-file change.\n</capability>"
        );
    }

    #[tokio::test]
    async fn get_session_info_returns_agent_name_when_assigned() {
        let agent_id = AgentId::new();
        let session = build_session(Some(agent_id));
        let session_id = session.id;

        let agent = AgentDefinition {
            display_name: Some("Research Agent".to_string()),
            description: Some("desc".to_string()),
            capabilities: vec![AgentCapabilityConfig::new("session")],
            ..AgentDefinition::new(agent_id, "research-agent", "prompt")
        };

        let context = ToolContext::new(session_id)
            .with_session_store(Arc::new(MockSessionStore {
                session: Arc::new(Mutex::new(Some(session))),
            }))
            .with_agent_store(Arc::new(MockAgentStore { agent: Some(agent) }));

        let tool = GetSessionInfoTool;
        let result = tool.execute_with_context(json!({}), &context).await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["title"], "Old title");
                assert_eq!(value["agent_name"], "Research Agent");
                assert!(value["usage"].is_null());
            }
            _ => panic!("expected success"),
        }
    }

    #[tokio::test]
    async fn get_session_info_returns_cumulative_usage() {
        let mut session = build_session(None);
        session.usage = Some(TokenUsage::with_cache(120, 45, Some(30), Some(10)));
        let session_id = session.id;

        let context = ToolContext::new(session_id).with_session_store(Arc::new(MockSessionStore {
            session: Arc::new(Mutex::new(Some(session))),
        }));

        let tool = GetSessionInfoTool;
        let result = tool.execute_with_context(json!({}), &context).await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["usage"]["input_tokens"], 120);
                assert_eq!(value["usage"]["output_tokens"], 45);
                assert_eq!(value["usage"]["cache_read_tokens"], 30);
                assert_eq!(value["usage"]["cache_creation_tokens"], 10);
                assert_eq!(value["usage"]["total_tokens"], 165);
            }
            _ => panic!("expected success"),
        }
    }
}
