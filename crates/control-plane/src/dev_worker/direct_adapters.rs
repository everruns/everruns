// Direct adapters for in-process worker (DEV_MODE)
//
// These adapters implement the core traits using StorageBackend directly,
// bypassing gRPC communication for in-process execution.
//
// Decision: Worker traits implemented directly against StorageBackend for DEV_MODE

use async_trait::async_trait;
use everruns_core::capabilities::AgentCapabilityConfig;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::events::{Event, EventRequest, MessageAgentData, MessageUserData};
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use everruns_core::traits::{
    AgentStore, EventEmitter, LlmProviderStore, ModelWithProvider, SessionFileStore, SessionStore,
};
use everruns_core::{
    Agent, AgentStatus, ContentPart, DEFAULT_ORG_ID, LlmProviderType, Message, MessageRole,
    Session, SessionStatus, ToolResultContentPart,
};
use everruns_core::{InputMessage, MessageRetriever};
use std::sync::Arc;
use uuid::Uuid;

use crate::services::{EventService, LlmResolverService};
use crate::storage::StorageBackend;
use crate::storage::models::UpdateSession;

// Helper to create store errors
fn store_error(msg: impl Into<String>) -> AgentLoopError {
    AgentLoopError::store(msg)
}

/// Extract file name from path
fn name_from_path(path: &str) -> String {
    if path == "/" {
        return "/".to_string();
    }
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string()
}

// ============================================================================
// DirectAgentStore
// ============================================================================

/// Direct agent store using StorageBackend
pub struct DirectAgentStore {
    db: Arc<StorageBackend>,
}

impl DirectAgentStore {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl AgentStore for DirectAgentStore {
    async fn get_agent(&self, agent_id: Uuid) -> Result<Option<Agent>> {
        // TODO: Get org_id from context after Phase 3
        let row = self
            .db
            .get_agent(DEFAULT_ORG_ID, agent_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get agent: {}", e);
                store_error("Failed to get agent")
            })?;

        // Also get capabilities for the agent
        let capabilities = if let Some(ref _r) = row {
            self.db
                .get_agent_capabilities(agent_id)
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };

        Ok(row.map(|r| Agent {
            id: r.id.into(),
            name: r.name,
            description: r.description,
            system_prompt: r.system_prompt,
            default_model_id: r.default_model_id.map(|id| id.into()),
            tags: r.tags,
            capabilities: capabilities
                .into_iter()
                .map(|c| AgentCapabilityConfig::with_config(c.capability_id, c.config))
                .collect(),
            status: match r.status.as_str() {
                "active" => AgentStatus::Active,
                "archived" => AgentStatus::Archived,
                _ => AgentStatus::Active,
            },
            created_at: r.created_at,
            updated_at: r.updated_at,
            usage: None, // Usage not tracked in direct adapter context
        }))
    }
}

// ============================================================================
// DirectSessionStore
// ============================================================================

/// Direct session store using StorageBackend
pub struct DirectSessionStore {
    db: Arc<StorageBackend>,
}

impl DirectSessionStore {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SessionStore for DirectSessionStore {
    async fn get_session(&self, session_id: Uuid) -> Result<Option<Session>> {
        // TODO: Get org_id from context after Phase 3
        let row = self
            .db
            .get_session(DEFAULT_ORG_ID, session_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get session: {}", e);
                store_error("Failed to get session")
            })?;

        Ok(row.map(|r| Session {
            id: r.id.into(),
            agent_id: r.agent_id.into(),
            title: r.title,
            preview: None,
            output_preview: None,
            tags: r.tags,
            model_id: r.model_id.map(|id| id.into()),
            status: match r.status.as_str() {
                "started" => SessionStatus::Started,
                "active" => SessionStatus::Active,
                "idle" => SessionStatus::Idle,
                "running" => SessionStatus::Active,
                _ => SessionStatus::Started,
            },
            created_at: r.created_at,
            started_at: r.started_at,
            finished_at: r.finished_at,
            usage: None, // Usage not tracked in direct adapter context
        }))
    }
}

// ============================================================================
// DirectMessageRetriever
// ============================================================================

/// Direct message retriever using StorageBackend
///
/// Retrieves messages reconstructed from events.
/// Message storage is handled via EventService (messages are stored as events).
pub struct DirectMessageRetriever {
    event_service: Arc<EventService>,
}

impl DirectMessageRetriever {
    pub fn new(_db: Arc<StorageBackend>, event_service: Arc<EventService>) -> Self {
        Self { event_service }
    }

    /// Add a new message and store it as an event
    ///
    /// Note: This is provided for API layer convenience.
    /// Messages are stored via EventService as typed events.
    pub async fn add(&self, session_id: Uuid, input: InputMessage) -> Result<Message> {
        use everruns_core::EventData;
        use everruns_core::events::EventContext;

        // Create a Message from the input
        let message = Message {
            id: Uuid::now_v7().into(),
            role: input.role.clone(),
            content: input.content.clone(),
            controls: input.controls.clone(),
            metadata: input.metadata.clone(),
            created_at: chrono::Utc::now(),
        };

        // Create the appropriate event based on role
        let (event_type, data) = match input.role {
            MessageRole::User => {
                let data = MessageUserData::new(message.clone());
                ("message.user", EventData::MessageUser(data))
            }
            MessageRole::Assistant => {
                let data = MessageAgentData::new(message.clone());
                ("message.agent", EventData::MessageAgent(data))
            }
            _ => {
                return Err(store_error("Unsupported message role for add"));
            }
        };

        let metadata_json = input
            .metadata
            .as_ref()
            .and_then(|m| serde_json::to_value(m).ok());
        let request = EventRequest {
            session_id,
            event_type: event_type.to_string(),
            ts: message.created_at,
            context: EventContext::default(),
            data,
            metadata: metadata_json,
            tags: Some(input.tags),
        };

        self.event_service.emit(request).await.map_err(|e| {
            tracing::error!("Failed to emit message event: {}", e);
            store_error("Failed to store message")
        })?;

        Ok(message)
    }
}

#[async_trait]
impl MessageRetriever for DirectMessageRetriever {
    async fn get(&self, session_id: Uuid, message_id: Uuid) -> Result<Option<Message>> {
        let messages = self.load(session_id).await?;
        Ok(messages.into_iter().find(|m| m.id == message_id))
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let events = self
            .event_service
            .list_message_events(session_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to load message events: {}", e);
                store_error("Failed to load messages")
            })?;

        let messages: Vec<Message> = events.into_iter().filter_map(event_to_message).collect();

        Ok(messages)
    }
}

/// Convert an event to a message
fn event_to_message(event: Event) -> Option<Message> {
    use everruns_core::EventData;

    match &event.data {
        EventData::MessageUser(data) => Some(data.message.clone()),
        EventData::MessageAgent(data) => Some(data.message.clone()),
        EventData::ToolCallCompleted(data) => {
            // Convert tool result to a message with tool result content
            // ToolCallCompletedData.result is Option<Vec<ContentPart>>, we need to convert it to JSON
            let result_json = data
                .result
                .as_ref()
                .and_then(|r| serde_json::to_value(r).ok());
            let content = vec![ContentPart::ToolResult(ToolResultContentPart {
                tool_call_id: data.tool_call_id.clone(),
                result: result_json,
                error: data.error.clone(),
            })];
            Some(Message {
                id: event.id.into(),
                role: MessageRole::ToolResult,
                content,
                controls: None,
                metadata: None,
                created_at: event.ts,
            })
        }
        _ => None,
    }
}

// ============================================================================
// DirectLlmProviderStore
// ============================================================================

/// Direct LLM provider store using StorageBackend
pub struct DirectLlmProviderStore {
    resolver: Arc<LlmResolverService>,
}

impl DirectLlmProviderStore {
    pub fn new(resolver: Arc<LlmResolverService>) -> Self {
        Self { resolver }
    }
}

#[async_trait]
impl LlmProviderStore for DirectLlmProviderStore {
    async fn get_model_with_provider(&self, model_id: Uuid) -> Result<Option<ModelWithProvider>> {
        let resolved = self.resolver.resolve_model(model_id).await.map_err(|e| {
            tracing::error!("Failed to resolve model: {}", e);
            store_error("Failed to resolve model")
        })?;

        Ok(resolved.map(|r| ModelWithProvider {
            model: r.model_id,
            provider_type: string_to_provider_type(&r.provider_type),
            api_key: r.api_key,
            base_url: r.base_url,
        }))
    }

    async fn get_default_model(&self) -> Result<Option<ModelWithProvider>> {
        let resolved = self.resolver.resolve_default_model().await.map_err(|e| {
            tracing::error!("Failed to resolve default model: {}", e);
            store_error("Failed to resolve default model")
        })?;

        Ok(resolved.map(|r| ModelWithProvider {
            model: r.model_id,
            provider_type: string_to_provider_type(&r.provider_type),
            api_key: r.api_key,
            base_url: r.base_url,
        }))
    }
}

fn string_to_provider_type(s: &str) -> LlmProviderType {
    match s.to_lowercase().as_str() {
        "openai" => LlmProviderType::Openai,
        "openai_completions" => LlmProviderType::OpenaiCompletions,
        "anthropic" => LlmProviderType::Anthropic,
        "llmsim" => LlmProviderType::LlmSim,
        _ => LlmProviderType::Openai, // Default
    }
}

// ============================================================================
// DirectEventEmitter
// ============================================================================

/// Direct event emitter using EventService
pub struct DirectEventEmitter {
    event_service: Arc<EventService>,
}

impl DirectEventEmitter {
    pub fn new(event_service: Arc<EventService>) -> Self {
        Self { event_service }
    }
}

#[async_trait]
impl EventEmitter for DirectEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        self.event_service.emit(request).await.map_err(|e| {
            tracing::error!("Failed to emit event: {}", e);
            store_error("Failed to emit event")
        })
    }
}

// ============================================================================
// DirectSessionFileStore
// ============================================================================

/// Direct session file store using StorageBackend
pub struct DirectSessionFileStore {
    db: Arc<StorageBackend>,
}

impl DirectSessionFileStore {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SessionFileStore for DirectSessionFileStore {
    async fn read_file(&self, session_id: Uuid, path: &str) -> Result<Option<SessionFile>> {
        let row = self
            .db
            .get_session_file(session_id, path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to read file: {}", e);
                store_error("Failed to read file")
            })?;

        Ok(row.map(|r| {
            // Convert Vec<u8> content to base64-encoded String
            let (content, encoding) = if let Some(bytes) = &r.content {
                // Try to interpret as UTF-8 text first
                match String::from_utf8(bytes.clone()) {
                    Ok(text) => (Some(text), "text".to_string()),
                    Err(_) => {
                        // Fall back to base64 for binary content
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                        (Some(b64), "base64".to_string())
                    }
                }
            } else {
                (None, "text".to_string())
            };

            SessionFile {
                id: r.id,
                session_id: r.session_id,
                path: r.path.clone(),
                name: name_from_path(&r.path),
                content,
                encoding,
                is_directory: r.is_directory,
                is_readonly: r.is_readonly,
                size_bytes: r.size_bytes,
                created_at: r.created_at,
                updated_at: r.updated_at,
            }
        }))
    }

    async fn write_file(
        &self,
        session_id: Uuid,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        use crate::storage::models::{CreateSessionFileRow, UpdateSessionFile};

        // Convert string content to bytes based on encoding
        let content_bytes = if encoding == "base64" {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(content)
                .map_err(|e| store_error(format!("Invalid base64 content: {}", e)))?
        } else {
            content.as_bytes().to_vec()
        };

        // Check if file exists
        let existing = self
            .db
            .get_session_file(session_id, path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to check existing file: {}", e);
                store_error("Failed to write file")
            })?;

        let row = if existing.is_some() {
            // Update existing file
            let update = UpdateSessionFile {
                content: Some(content_bytes.clone()),
                ..Default::default()
            };
            self.db
                .update_session_file(session_id, path, update)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to update file: {}", e);
                    store_error("Failed to write file")
                })?
                .ok_or_else(|| store_error("File disappeared during update"))?
        } else {
            // Create new file
            let create = CreateSessionFileRow {
                session_id,
                path: path.to_string(),
                content: Some(content_bytes.clone()),
                is_directory: false,
                is_readonly: false,
            };
            self.db.create_session_file(create).await.map_err(|e| {
                tracing::error!("Failed to create file: {}", e);
                store_error("Failed to write file")
            })?
        };

        Ok(SessionFile {
            id: row.id,
            session_id: row.session_id,
            path: row.path.clone(),
            name: name_from_path(&row.path),
            content: Some(content.to_string()),
            encoding: encoding.to_string(),
            is_directory: row.is_directory,
            is_readonly: row.is_readonly,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    async fn delete_file(&self, session_id: Uuid, path: &str, recursive: bool) -> Result<bool> {
        if recursive {
            let count = self
                .db
                .delete_session_file_recursive(session_id, path)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to delete file recursively: {}", e);
                    store_error("Failed to delete file")
                })?;
            Ok(count > 0)
        } else {
            self.db
                .delete_session_file(session_id, path)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to delete file: {}", e);
                    store_error("Failed to delete file")
                })
        }
    }

    async fn list_directory(&self, session_id: Uuid, path: &str) -> Result<Vec<FileInfo>> {
        let rows = self
            .db
            .list_session_files(session_id, path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to list directory: {}", e);
                store_error("Failed to list directory")
            })?;

        Ok(rows
            .into_iter()
            .map(|r| FileInfo {
                id: r.id,
                session_id: r.session_id,
                path: r.path.clone(),
                name: name_from_path(&r.path),
                is_directory: r.is_directory,
                is_readonly: r.is_readonly,
                size_bytes: r.size_bytes,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect())
    }

    async fn stat_file(&self, session_id: Uuid, path: &str) -> Result<Option<FileStat>> {
        let row = self
            .db
            .get_session_file(session_id, path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to stat file: {}", e);
                store_error("Failed to stat file")
            })?;

        Ok(row.map(|r| FileStat {
            path: r.path.clone(),
            name: name_from_path(&r.path),
            is_directory: r.is_directory,
            is_readonly: r.is_readonly,
            size_bytes: r.size_bytes,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }))
    }

    async fn grep_files(
        &self,
        session_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        let _rows = self
            .db
            .grep_session_files(session_id, pattern, path_pattern)
            .await
            .map_err(|e| {
                tracing::error!("Failed to grep files: {}", e);
                store_error("Failed to grep files")
            })?;

        // The storage layer returns file info rows, we need to do actual grep
        // For now, return empty - full implementation requires reading file contents
        // This is acceptable for DEV_MODE as it's primarily for testing
        Ok(vec![])
    }

    async fn create_directory(&self, session_id: Uuid, path: &str) -> Result<FileInfo> {
        use crate::storage::models::CreateSessionFileRow;

        let create = CreateSessionFileRow {
            session_id,
            path: path.to_string(),
            content: None,
            is_directory: true,
            is_readonly: false,
        };

        let row = self.db.create_session_file(create).await.map_err(|e| {
            tracing::error!("Failed to create directory: {}", e);
            store_error("Failed to create directory")
        })?;

        Ok(FileInfo {
            id: row.id,
            session_id: row.session_id,
            path: row.path.clone(),
            name: name_from_path(&row.path),
            is_directory: row.is_directory,
            is_readonly: row.is_readonly,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

// ============================================================================
// SessionStatusUpdater - for updating session status directly
// ============================================================================

/// Session status updater for in-process worker
pub struct SessionStatusUpdater {
    db: Arc<StorageBackend>,
}

impl SessionStatusUpdater {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    pub async fn set_status(&self, session_id: Uuid, status: &str) -> Result<()> {
        let update = UpdateSession {
            status: Some(status.to_string()),
            ..Default::default()
        };

        // TODO: Get org_id from context after Phase 3
        self.db
            .update_session(DEFAULT_ORG_ID, session_id, update)
            .await
            .map_err(|e| {
                tracing::error!("Failed to update session status: {}", e);
                store_error("Failed to update session status")
            })?;

        Ok(())
    }
}
