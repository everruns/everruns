// Direct implementation of WorkerAdapters for in-process worker
//
// Decision: Uses StorageBackend and services directly (no gRPC)
// Decision: Used by in-process worker in DEV_MODE
//
// This implementation provides the same interface as GrpcWorkerAdapters
// but with direct access to the storage backend and services.

use async_trait::async_trait;
use everruns_core::capabilities::{
    AgentCapabilityConfig, CapabilityRegistry, collect_capabilities, is_mcp_capability,
};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::events::{Event, EventRequest};
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use everruns_core::traits::ResolvedImage;
use everruns_core::typed_id::{AgentId, MessageId, SessionId};
use everruns_core::{
    Agent, AgentStatus, ContentPart, DriverRegistry, EventData, LlmProviderType, Message,
    MessageRole, Session, SessionStatus, ToolDefinition, ToolRegistry, ToolResultContentPart,
};
use everruns_worker::create_driver_registry;
use everruns_worker::mcp_executor::McpServerInfo;
use everruns_worker::worker_adapters::{ModelWithProvider, TurnContext, WorkerAdapters};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::services::{EventService, LlmResolverService, McpServerService};
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

// =============================================================================
// DirectWorkerAdapters Implementation
// =============================================================================

/// Direct storage-backed worker adapters for in-process worker
#[derive(Clone)]
pub struct DirectWorkerAdapters {
    db: Arc<StorageBackend>,
    event_service: Arc<EventService>,
    llm_resolver: Arc<LlmResolverService>,
    mcp_server_service: Arc<McpServerService>,
    capability_registry: CapabilityRegistry,
}

impl DirectWorkerAdapters {
    /// Create new direct adapters with access to storage and services
    pub fn new(
        db: Arc<StorageBackend>,
        event_service: Arc<EventService>,
        llm_resolver: Arc<LlmResolverService>,
        mcp_server_service: Arc<McpServerService>,
        capability_registry: CapabilityRegistry,
    ) -> Self {
        Self {
            db,
            event_service,
            llm_resolver,
            mcp_server_service,
            capability_registry,
        }
    }
}

#[async_trait]
impl WorkerAdapters for DirectWorkerAdapters {
    // =========================================================================
    // Agent Operations
    // =========================================================================

    async fn get_agent(&self, org_id: i64, agent_id: Uuid) -> Result<Option<Agent>> {
        let agent_id_typed = AgentId::from_uuid(agent_id);
        let row = self
            .db
            .get_agent(org_id, agent_id_typed)
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
            id: r.id,
            name: r.name,
            description: r.description,
            system_prompt: r.system_prompt,
            default_model_id: r.default_model_id,
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
            usage: None,
        }))
    }

    // =========================================================================
    // Session Operations
    // =========================================================================

    async fn get_session(&self, org_id: i64, session_id: Uuid) -> Result<Option<Session>> {
        let session_id_typed = SessionId::from_uuid(session_id);
        let row = self
            .db
            .get_session(org_id, session_id_typed)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get session: {}", e);
                store_error("Failed to get session")
            })?;

        Ok(row.map(|r| Session {
            id: r.id,
            agent_id: r.agent_id,
            title: r.title,
            preview: None,
            output_preview: None,
            tags: r.tags,
            model_id: r.model_id,
            status: match r.status.as_str() {
                "started" => SessionStatus::Started,
                "active" => SessionStatus::Active,
                "idle" => SessionStatus::Idle,
                "running" => SessionStatus::Active,
                _ => SessionStatus::Started,
            },
            created_at: r.created_at,
            updated_at: r.updated_at,
            started_at: r.started_at,
            finished_at: r.finished_at,
            usage: None,
        }))
    }

    async fn set_session_status(
        &self,
        org_id: i64,
        session_id: Uuid,
        status: &str,
    ) -> Result<Session> {
        let session_id_typed = SessionId::from_uuid(session_id);
        let update = UpdateSession {
            status: Some(status.to_string()),
            ..Default::default()
        };

        self.db
            .update_session(org_id, session_id_typed, update)
            .await
            .map_err(|e| {
                tracing::error!("Failed to update session status: {}", e);
                store_error("Failed to update session status")
            })?;

        // Fetch and return updated session
        self.get_session(org_id, session_id)
            .await?
            .ok_or_else(|| store_error("Session not found after update"))
    }

    // =========================================================================
    // Message Operations
    // =========================================================================

    async fn get_message(&self, session_id: Uuid, message_id: Uuid) -> Result<Option<Message>> {
        let messages = self.load_messages(session_id).await?;
        Ok(messages.into_iter().find(|m| m.id == message_id))
    }

    async fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>> {
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

    // =========================================================================
    // Event Operations
    // =========================================================================

    async fn emit_event(&self, request: EventRequest) -> Result<Event> {
        self.event_service.emit(request).await.map_err(|e| {
            tracing::error!("Failed to emit event: {}", e);
            store_error("Failed to emit event")
        })
    }

    // =========================================================================
    // LLM Provider Operations
    // =========================================================================

    async fn get_model_with_provider(
        &self,
        _org_id: i64,
        model_id: Uuid,
    ) -> Result<Option<ModelWithProvider>> {
        let resolved = self
            .llm_resolver
            .resolve_model(model_id)
            .await
            .map_err(|e| {
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

    async fn get_default_model(&self, _org_id: i64) -> Result<Option<ModelWithProvider>> {
        let resolved = self
            .llm_resolver
            .resolve_default_model()
            .await
            .map_err(|e| {
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

    // =========================================================================
    // Image Resolution Operations
    // =========================================================================

    async fn resolve_image(&self, image_id: Uuid) -> Result<Option<ResolvedImage>> {
        // Get the image from storage
        let image_row = self.db.get_image(image_id).await.map_err(|e| {
            tracing::error!("Failed to get image: {}", e);
            store_error("Failed to get image")
        })?;

        match image_row {
            Some(row) => {
                // Convert image data to base64
                use base64::Engine;
                let base64_data = base64::engine::general_purpose::STANDARD.encode(&row.data);
                Ok(Some(ResolvedImage::new(base64_data, row.content_type)))
            }
            None => Ok(None),
        }
    }

    async fn resolve_images_batch(
        &self,
        image_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, ResolvedImage>> {
        let mut result = HashMap::new();
        for &image_id in image_ids {
            if let Some(resolved) = self.resolve_image(image_id).await? {
                result.insert(image_id, resolved);
            }
        }
        Ok(result)
    }

    // =========================================================================
    // Session File Operations
    // =========================================================================

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
            let (content, encoding) = if let Some(bytes) = &r.content {
                match String::from_utf8(bytes.clone()) {
                    Ok(text) => (Some(text), "text".to_string()),
                    Err(_) => {
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
                session_id: r.session_id.uuid(),
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

        let content_bytes = if encoding == "base64" {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(content)
                .map_err(|e| store_error(format!("Invalid base64 content: {}", e)))?
        } else {
            content.as_bytes().to_vec()
        };

        let existing = self
            .db
            .get_session_file(session_id, path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to check existing file: {}", e);
                store_error("Failed to write file")
            })?;

        let row = if existing.is_some() {
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
            let create = CreateSessionFileRow {
                session_id: SessionId::from_uuid(session_id),
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
            session_id: row.session_id.uuid(),
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
                session_id: r.session_id.uuid(),
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

        // TODO: Implement actual grep - for now return empty
        Ok(vec![])
    }

    async fn create_directory(&self, session_id: Uuid, path: &str) -> Result<FileInfo> {
        use crate::storage::models::CreateSessionFileRow;

        let create = CreateSessionFileRow {
            session_id: SessionId::from_uuid(session_id),
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
            session_id: row.session_id.uuid(),
            path: row.path.clone(),
            name: name_from_path(&row.path),
            is_directory: row.is_directory,
            is_readonly: row.is_readonly,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    // =========================================================================
    // MCP Server Operations
    // =========================================================================

    async fn get_mcp_server_by_prefix(
        &self,
        _org_id: i64,
        server_prefix: &str,
    ) -> Result<McpServerInfo> {
        // Search for MCP server by name prefix (using sanitized name matching)
        let server_prefix_lower = server_prefix.to_lowercase();
        let servers = self.mcp_server_service.list().await.map_err(|e| {
            tracing::error!("Failed to list MCP servers: {}", e);
            store_error("Failed to get MCP server")
        })?;

        let server = servers
            .into_iter()
            .find(|s| {
                let sanitized_name = s
                    .name
                    .to_lowercase()
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '_' })
                    .collect::<String>();
                sanitized_name == server_prefix_lower
            })
            .ok_or_else(|| store_error(format!("MCP server not found: {}", server_prefix)))?;

        Ok(McpServerInfo {
            id: server.id.uuid(),
            name: server.name,
            url: server.url,
            // API key is encrypted - would need decryption service access
            // TODO: Add decryption support when needed for MCP tool execution
            api_key: None,
            headers: server.headers,
        })
    }

    // =========================================================================
    // Turn Context (batch operation)
    // =========================================================================

    async fn load_turn_context(&self, org_id: i64, session_id: Uuid) -> Result<TurnContext> {
        // Load session
        let session = self
            .get_session(org_id, session_id)
            .await?
            .ok_or_else(|| store_error("Session not found"))?;

        // Load agent
        let agent = self
            .get_agent(org_id, session.agent_id.uuid())
            .await?
            .ok_or_else(|| store_error("Agent not found"))?;

        // Load messages
        let messages = self.load_messages(session_id).await?;

        // Load model
        let model = if let Some(model_id) = session.model_id {
            self.get_model_with_provider(org_id, model_id.uuid())
                .await?
        } else if let Some(model_id) = agent.default_model_id {
            self.get_model_with_provider(org_id, model_id.uuid())
                .await?
        } else {
            self.get_default_model(org_id).await?
        };

        // Build MCP tool definitions
        let mcp_tool_definitions = self
            .build_mcp_tool_definitions(agent.id.uuid())
            .await
            .unwrap_or_default();

        Ok(TurnContext {
            agent,
            session,
            messages,
            model,
            mcp_tool_definitions,
        })
    }

    // =========================================================================
    // Factory Methods
    // =========================================================================

    fn capability_registry(&self) -> CapabilityRegistry {
        self.capability_registry.clone()
    }

    fn driver_registry(&self) -> DriverRegistry {
        create_driver_registry()
    }

    async fn build_tool_registry(&self, agent_id: Uuid) -> Result<ToolRegistry> {
        let mut registry = ToolRegistry::with_defaults();

        let capability_rows = self
            .db
            .get_agent_capabilities(agent_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get agent capabilities: {}", e);
                store_error("Failed to get agent capabilities")
            })?;

        let builtin_cap_ids: Vec<String> = capability_rows
            .iter()
            .map(|r| r.capability_id.clone())
            .filter(|id| !is_mcp_capability(id))
            .collect();

        if !builtin_cap_ids.is_empty() {
            let collected = collect_capabilities(&builtin_cap_ids, &self.capability_registry);
            for tool in collected.tools {
                registry.register_boxed(tool);
            }
            tracing::debug!(
                capability_count = builtin_cap_ids.len(),
                tool_count = collected.tool_definitions.len(),
                "Registered capability tools"
            );
        }

        Ok(registry)
    }
}

impl DirectWorkerAdapters {
    /// Build MCP tool definitions from agent's MCP capabilities
    async fn build_mcp_tool_definitions(&self, agent_id: Uuid) -> Result<Vec<ToolDefinition>> {
        use everruns_core::capabilities::mcp::parse_mcp_capability_id;
        use everruns_core::mcp_server::mcp_tool_name;
        use everruns_core::tool_types::{BuiltinTool, ToolPolicy};

        let capability_rows = self
            .db
            .get_agent_capabilities(agent_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get agent capabilities: {}", e);
                store_error("Failed to get agent capabilities")
            })?;

        let mut mcp_tools = Vec::new();

        for cap_row in &capability_rows {
            let cap_id = &cap_row.capability_id;
            let server_id = match parse_mcp_capability_id(cap_id) {
                Some(id) => id,
                None => continue,
            };

            let tools = match self.mcp_server_service.get_tools(server_id, false).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(
                        server_id = %server_id,
                        error = %e,
                        "Failed to get MCP server tools, skipping"
                    );
                    continue;
                }
            };

            let server_name = match self.mcp_server_service.get(server_id).await {
                Ok(Some(s)) => s.name,
                _ => {
                    tracing::warn!(server_id = %server_id, "MCP server not found, skipping");
                    continue;
                }
            };

            for tool in tools {
                let prefixed_name = mcp_tool_name(&server_name, &tool.name);
                let description = tool
                    .description
                    .unwrap_or_else(|| format!("Tool from MCP server: {}", server_name));

                mcp_tools.push(ToolDefinition::Builtin(BuiltinTool {
                    name: prefixed_name,
                    description,
                    parameters: tool.input_schema,
                    policy: ToolPolicy::Auto,
                }));
            }
        }

        Ok(mcp_tools)
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

fn string_to_provider_type(s: &str) -> LlmProviderType {
    match s.to_lowercase().as_str() {
        "openai" => LlmProviderType::Openai,
        "openai_completions" => LlmProviderType::OpenaiCompletions,
        "anthropic" => LlmProviderType::Anthropic,
        "llmsim" => LlmProviderType::LlmSim,
        _ => LlmProviderType::Openai,
    }
}

/// Convert an event to a message
fn event_to_message(event: Event) -> Option<Message> {
    match &event.data {
        EventData::MessageUser(data) => Some(data.message.clone()),
        EventData::MessageAgent(data) => Some(data.message.clone()),
        EventData::ToolCallCompleted(data) => {
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
                id: MessageId::from_uuid(event.id.uuid()),
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
