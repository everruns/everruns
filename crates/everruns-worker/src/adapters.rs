// Database-backed adapters for core traits
//
// These implementations connect the core agent-loop abstraction to the
// actual database and LLM providers used in production.

use async_trait::async_trait;
use everruns_contracts::events::AgUiEvent;
use everruns_contracts::tools::ToolCall;
use everruns_core::{
    traits::{EventEmitter, MessageStore},
    AgentLoopError, ConversationMessage, LoopEvent, MessageRole, Result,
};
use everruns_openai::OpenAiProvider;
use everruns_storage::models::CreateMessage;
use everruns_storage::repositories::Database;
use tracing::{error, info};
use uuid::Uuid;

use crate::unified_tool_executor::UnifiedToolExecutor;

// ============================================================================
// DbEventEmitter - Persists events to database
// ============================================================================

/// Database-backed event emitter
///
/// Persists AG-UI events to the events table for SSE streaming.
pub struct DbEventEmitter {
    db: Database,
}

impl DbEventEmitter {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    async fn persist_ag_ui_event(&self, session_id: Uuid, event: &AgUiEvent) -> Result<()> {
        let event_type = match event {
            AgUiEvent::RunStarted(_) => "session.started",
            AgUiEvent::RunFinished(_) => "session.finished",
            AgUiEvent::RunError(_) => "session.error",
            AgUiEvent::StepStarted(_) => "step.started",
            AgUiEvent::StepFinished(_) => "step.finished",
            AgUiEvent::TextMessageStart(_) => "text.start",
            AgUiEvent::TextMessageContent(_) => "text.delta",
            AgUiEvent::TextMessageEnd(_) => "text.end",
            AgUiEvent::ToolCallStart(_) => "tool.call.start",
            AgUiEvent::ToolCallArgs(_) => "tool.call.args",
            AgUiEvent::ToolCallEnd(_) => "tool.call.end",
            AgUiEvent::ToolCallResult(_) => "tool.result",
            AgUiEvent::StateSnapshot(_) => "state.snapshot",
            AgUiEvent::StateDelta(_) => "state.delta",
            AgUiEvent::MessagesSnapshot(_) => "messages.snapshot",
            AgUiEvent::Custom(_) => "custom",
        };

        let event_data =
            serde_json::to_value(event).map_err(|e| AgentLoopError::event(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO events (session_id, sequence, event_type, data)
            VALUES ($1, COALESCE((SELECT MAX(sequence) + 1 FROM events WHERE session_id = $1), 1), $2, $3)
            "#,
        )
        .bind(session_id)
        .bind(event_type)
        .bind(event_data)
        .execute(self.db.pool())
        .await
        .map_err(|e| AgentLoopError::event(e.to_string()))?;

        Ok(())
    }
}

#[async_trait]
impl EventEmitter for DbEventEmitter {
    async fn emit(&self, event: LoopEvent) -> Result<()> {
        // Extract session_id from event
        let session_id_str = event.session_id();
        let session_id = if session_id_str.is_empty() {
            // For events without session_id, skip persistence
            return Ok(());
        } else {
            Uuid::parse_str(session_id_str).map_err(|e| AgentLoopError::event(e.to_string()))?
        };

        // Only persist AG-UI events to the database
        if let LoopEvent::AgUi(ag_ui_event) = &event {
            self.persist_ag_ui_event(session_id, ag_ui_event).await?;
        }

        // Log other events for debugging
        match &event {
            LoopEvent::LoopStarted { .. } => info!("Loop started"),
            LoopEvent::LoopCompleted {
                total_iterations, ..
            } => {
                info!(iterations = total_iterations, "Loop completed")
            }
            LoopEvent::LoopError { error, .. } => error!(error = %error, "Loop error"),
            _ => {}
        }

        Ok(())
    }
}

// ============================================================================
// DbMessageStore - Stores messages in database
// ============================================================================

/// Database-backed message store
///
/// Stores conversation messages in the messages table.
pub struct DbMessageStore {
    db: Database,
}

impl DbMessageStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl MessageStore for DbMessageStore {
    async fn store(&self, session_id: Uuid, message: ConversationMessage) -> Result<()> {
        let role = message.role.to_string();
        let content = match &message.content {
            everruns_core::message::MessageContent::Text(text) => {
                serde_json::json!({ "text": text })
            }
            everruns_core::message::MessageContent::ToolCall {
                id,
                name,
                arguments,
            } => {
                serde_json::json!({
                    "id": id,
                    "name": name,
                    "arguments": arguments
                })
            }
            everruns_core::message::MessageContent::ToolResult { result, error } => {
                serde_json::json!({
                    "result": result,
                    "error": error
                })
            }
        };

        let create_msg = CreateMessage {
            session_id,
            role,
            content,
            tool_call_id: message.tool_call_id,
        };

        self.db
            .create_message(create_msg)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(())
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<ConversationMessage>> {
        let messages = self
            .db
            .list_messages(session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        let converted: Vec<ConversationMessage> = messages
            .into_iter()
            .map(|msg| {
                let role = MessageRole::from(msg.role.as_str());
                let content = match role {
                    MessageRole::User | MessageRole::Assistant | MessageRole::System => {
                        let text = msg
                            .content
                            .get("text")
                            .and_then(|t| t.as_str())
                            .unwrap_or("")
                            .to_string();
                        everruns_core::message::MessageContent::Text(text)
                    }
                    MessageRole::ToolCall => {
                        let id = msg
                            .content
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = msg
                            .content
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let arguments = msg
                            .content
                            .get("arguments")
                            .cloned()
                            .unwrap_or(serde_json::json!({}));
                        everruns_core::message::MessageContent::ToolCall {
                            id,
                            name,
                            arguments,
                        }
                    }
                    MessageRole::ToolResult => {
                        let result = msg.content.get("result").cloned();
                        let error = msg
                            .content
                            .get("error")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        everruns_core::message::MessageContent::ToolResult { result, error }
                    }
                };

                // Parse tool_calls from assistant messages if present
                let tool_calls = if role == MessageRole::Assistant {
                    msg.content
                        .get("tool_calls")
                        .and_then(|tc| serde_json::from_value::<Vec<ToolCall>>(tc.clone()).ok())
                } else {
                    None
                };

                ConversationMessage {
                    id: msg.id,
                    role,
                    content,
                    tool_call_id: msg.tool_call_id,
                    tool_calls,
                    created_at: msg.created_at,
                }
            })
            .collect();

        Ok(converted)
    }

    async fn count(&self, session_id: Uuid) -> Result<usize> {
        let messages = self.load(session_id).await?;
        Ok(messages.len())
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed event emitter
pub fn create_db_event_emitter(db: Database) -> DbEventEmitter {
    DbEventEmitter::new(db)
}

/// Create a database-backed message store
pub fn create_db_message_store(db: Database) -> DbMessageStore {
    DbMessageStore::new(db)
}

/// Create an OpenAI LLM provider
pub fn create_openai_provider() -> Result<OpenAiProvider> {
    OpenAiProvider::new().map_err(|e| AgentLoopError::llm(e.to_string()))
}

/// Create a unified tool executor with default built-in tools
pub fn create_unified_tool_executor() -> UnifiedToolExecutor {
    UnifiedToolExecutor::with_default_tools()
}

/// Create a unified tool executor with a custom tool registry
pub fn create_unified_tool_executor_with_registry(
    registry: everruns_core::ToolRegistry,
) -> UnifiedToolExecutor {
    UnifiedToolExecutor::new(registry)
}

/// Create a fully configured AgentLoop with database backends
///
/// Uses the `UnifiedToolExecutor` with ToolRegistry for built-in tools.
pub fn create_db_agent_loop(
    config: everruns_core::AgentConfig,
    db: Database,
) -> Result<
    everruns_core::AgentLoop<DbEventEmitter, DbMessageStore, OpenAiProvider, UnifiedToolExecutor>,
> {
    let event_emitter = create_db_event_emitter(db.clone());
    let message_store = create_db_message_store(db);
    let llm_provider = create_openai_provider()?;
    let tool_executor = create_unified_tool_executor();

    Ok(everruns_core::AgentLoop::new(
        config,
        event_emitter,
        message_store,
        llm_provider,
        tool_executor,
    ))
}

/// Create a fully configured AgentLoop with database backends and a custom tool registry
///
/// This allows passing a custom `ToolRegistry` with specific built-in tools.
pub fn create_db_agent_loop_with_registry(
    config: everruns_core::AgentConfig,
    db: Database,
    registry: everruns_core::ToolRegistry,
) -> Result<
    everruns_core::AgentLoop<DbEventEmitter, DbMessageStore, OpenAiProvider, UnifiedToolExecutor>,
> {
    let event_emitter = create_db_event_emitter(db.clone());
    let message_store = create_db_message_store(db);
    let llm_provider = create_openai_provider()?;
    let tool_executor = create_unified_tool_executor_with_registry(registry);

    Ok(everruns_core::AgentLoop::new(
        config,
        event_emitter,
        message_store,
        llm_provider,
        tool_executor,
    ))
}
