// Direct implementation of WorkerAdapters for in-process worker
//
// Decision: Uses StorageBackend and services directly (no gRPC)
// Decision: Used by in-process worker in DEV_MODE
//
// This implementation provides the same interface as GrpcWorkerAdapters
// but with direct access to the storage backend and services.

use async_trait::async_trait;
use everruns_core::capabilities::{
    AgentCapabilityConfig, CapabilityRegistry, SystemPromptContext, collect_capabilities,
    is_mcp_capability,
};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::events::{Event, EventRequest};
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use everruns_core::traits::ResolvedImage;
use everruns_core::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use everruns_core::{
    Agent, AgentStatus, ContentPart, DEFAULT_ORG_PUBLIC_ID, DriverRegistry, EventData, Harness,
    HarnessStatus, LlmProviderType, Message, MessageRole, Session, SessionStatus, ToolDefinition,
    ToolRegistry, ToolResultContentPart,
};
use everruns_worker::create_driver_registry;
use everruns_worker::mcp_executor::McpServerInfo;
use everruns_worker::worker_adapters::{ModelWithProvider, TurnContext, WorkerAdapters};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::services::{EventService, LlmResolverService, McpServerService};
use crate::storage::StorageBackend;
use crate::storage::models::{AgentCapabilityRow, UpdateSession};

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
    sqldb_store: everruns_core::traits::SessionSqlDbStoreRef,
    storage_store: Option<Arc<dyn everruns_core::traits::SessionStorageStore>>,
    connection_resolver: Option<Arc<dyn everruns_core::traits::UserConnectionResolver>>,
    runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
}

impl DirectWorkerAdapters {
    /// Create new direct adapters with access to storage and services
    pub fn new(
        db: Arc<StorageBackend>,
        event_service: Arc<EventService>,
        llm_resolver: Arc<LlmResolverService>,
        mcp_server_service: Arc<McpServerService>,
        capability_registry: CapabilityRegistry,
        sqldb_store: everruns_core::traits::SessionSqlDbStoreRef,
    ) -> Self {
        Self {
            db,
            event_service,
            llm_resolver,
            mcp_server_service,
            capability_registry,
            sqldb_store,
            storage_store: None,
            connection_resolver: None,
            runner: None,
        }
    }

    /// Set the agent runner for platform management tools (send_message, etc.)
    pub fn with_runner(mut self, runner: Arc<dyn everruns_worker::AgentRunner>) -> Self {
        self.runner = Some(runner);
        self
    }

    /// Set the session storage store for kv_store/secret_store tools
    pub fn with_storage_store(
        mut self,
        store: Arc<dyn everruns_core::traits::SessionStorageStore>,
    ) -> Self {
        self.storage_store = Some(store);
        self
    }

    /// Set the user connection resolver for lazy token lookup
    pub fn with_connection_resolver(
        mut self,
        resolver: Arc<dyn everruns_core::traits::UserConnectionResolver>,
    ) -> Self {
        self.connection_resolver = Some(resolver);
        self
    }

    /// Ensure a directory exists, creating it and parents if needed
    async fn ensure_directory_exists(&self, session_id: Uuid, path: &str) -> Result<()> {
        use crate::storage::models::CreateSessionFileRow;

        if path == "/" {
            return Ok(()); // Root always exists
        }

        // Check if directory exists
        if let Some(existing) = self
            .db
            .get_session_file(session_id, path)
            .await
            .map_err(|e| store_error(format!("Failed to check directory: {}", e)))?
        {
            if existing.is_directory {
                return Ok(());
            } else {
                return Err(store_error(format!("A file exists at path: {}", path)));
            }
        }

        // Create parent first
        if let Some(parent) = FileInfo::parent_path(path) {
            Box::pin(self.ensure_directory_exists(session_id, &parent)).await?;
        }

        // Create this directory
        let input = CreateSessionFileRow {
            session_id: SessionId::from_uuid(session_id),
            path: path.to_string(),
            content: None,
            is_directory: true,
            is_readonly: false,
        };

        self.db
            .create_session_file(input)
            .await
            .map_err(|e| store_error(format!("Failed to create directory: {}", e)))?;
        Ok(())
    }
}

#[async_trait]
impl WorkerAdapters for DirectWorkerAdapters {
    // =========================================================================
    // Agent Operations
    // =========================================================================

    async fn get_harness(&self, org_id: i64, harness_id: Uuid) -> Result<Option<Harness>> {
        // Delegate to the private helper method
        Self::get_harness_impl(self, org_id, harness_id).await
    }

    async fn get_agent(&self, org_id: i64, agent_id: Uuid) -> Result<Option<Agent>> {
        let capabilities = self
            .db
            .get_agent_capabilities(agent_id)
            .await
            .unwrap_or_default();
        self.get_agent_with_capabilities(org_id, agent_id, capabilities)
            .await
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

        Ok(row.map(|r| {
            // Parse capabilities from JSON
            let capabilities = serde_json::from_value(r.capabilities).unwrap_or_default();
            Session {
                id: r.id,
                organization_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
                harness_id: r.harness_id.unwrap_or_else(|| HarnessId::from_seed(1)),
                agent_id: r.agent_id,
                title: r.title,
                preview: None,
                output_preview: None,
                tags: r.tags,
                model_id: r.model_id,
                capabilities,
                tools: vec![],
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
                is_pinned: None,
                active_schedule_count: None,
                features: vec![],
            }
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

    async fn set_session_title(
        &self,
        org_id: i64,
        session_id: Uuid,
        title: String,
    ) -> Result<Session> {
        let session_id_typed = SessionId::from_uuid(session_id);
        let update = UpdateSession {
            title: Some(title),
            ..Default::default()
        };

        self.db
            .update_session(org_id, session_id_typed, update)
            .await
            .map_err(|e| {
                tracing::error!("Failed to update session title: {}", e);
                store_error("Failed to update session title")
            })?;

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

    async fn resolve_image(&self, org_id: i64, image_id: Uuid) -> Result<Option<ResolvedImage>> {
        let image_row = self.db.get_image(org_id, image_id).await.map_err(|e| {
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
        org_id: i64,
        image_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, ResolvedImage>> {
        let mut result = HashMap::new();
        for &image_id in image_ids {
            if let Some(resolved) = self.resolve_image(org_id, image_id).await? {
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
            // Ensure parent directory exists
            if let Some(parent) = FileInfo::parent_path(path) {
                self.ensure_directory_exists(session_id, &parent).await?;
            }

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
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("not a directory") {
                    tracing::debug!("Directory not found: {}", path);
                } else {
                    tracing::error!("Failed to list directory: {}", e);
                }
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
        let regex = regex::Regex::new(pattern)
            .map_err(|e| store_error(format!("Invalid regex pattern: {}", e)))?;

        let rows = self
            .db
            .grep_session_files(session_id, pattern, path_pattern)
            .await
            .map_err(|e| {
                tracing::error!("Failed to grep files: {}", e);
                store_error("Failed to grep files")
            })?;

        let mut results = Vec::new();

        for file_info in rows {
            let file = self
                .db
                .get_session_file(session_id, &file_info.path)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to read file for grep: {}", e);
                    store_error("Failed to read file for grep")
                })?;

            if let Some(f) = file
                && let Some(content) = f.content
                && let Ok(text) = String::from_utf8(content)
            {
                for (i, line) in text.lines().enumerate() {
                    if regex.is_match(line) {
                        results.push(GrepMatch {
                            path: file_info.path.clone(),
                            line_number: i + 1,
                            line: line.to_string(),
                        });
                    }
                }
            }
        }

        Ok(results)
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
        org_id: i64,
        server_prefix: &str,
    ) -> Result<McpServerInfo> {
        // Search for MCP server by name prefix (using sanitized name matching)
        let server_prefix_lower = server_prefix.to_lowercase();
        let servers = self.mcp_server_service.list(org_id).await.map_err(|e| {
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

        // Decrypt API key if set
        let api_key = if server.api_key_set {
            self.mcp_server_service
                .decrypt_api_key(org_id, server.id.uuid())
                .await
                .map_err(|e| {
                    tracing::error!("Failed to decrypt MCP server API key: {}", e);
                    store_error("Failed to decrypt MCP server API key")
                })?
        } else {
            None
        };

        Ok(McpServerInfo {
            id: server.id.uuid(),
            name: server.name,
            url: server.url,
            api_key,
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

        // Load agent capabilities once, reuse for get_agent + build_mcp_tool_definitions
        let (agent, mcp_tool_definitions) = if let Some(agent_id) = session.agent_id {
            let capability_rows = self
                .db
                .get_agent_capabilities(agent_id.uuid())
                .await
                .unwrap_or_default();

            let agent = self
                .get_agent_with_capabilities(org_id, agent_id.uuid(), capability_rows.clone())
                .await?;

            let mcp_tools = self
                .build_mcp_tool_definitions_with_capabilities(org_id, &capability_rows)
                .await
                .unwrap_or_default();

            (agent, mcp_tools)
        } else {
            (None, vec![])
        };

        // Load harness
        let harness = self
            .get_harness_impl(org_id, session.harness_id.uuid())
            .await?;

        // Load messages
        let messages = self.load_messages(session_id).await?;

        // Load model (session > agent > harness > default)
        let model = if let Some(model_id) = session.model_id {
            self.get_model_with_provider(org_id, model_id.uuid())
                .await?
        } else if let Some(ref a) = agent {
            if let Some(model_id) = a.default_model_id {
                self.get_model_with_provider(org_id, model_id.uuid())
                    .await?
            } else if let Some(ref h) = harness {
                if let Some(model_id) = h.default_model_id {
                    self.get_model_with_provider(org_id, model_id.uuid())
                        .await?
                } else {
                    self.get_default_model(org_id).await?
                }
            } else {
                self.get_default_model(org_id).await?
            }
        } else if let Some(ref h) = harness {
            if let Some(model_id) = h.default_model_id {
                self.get_model_with_provider(org_id, model_id.uuid())
                    .await?
            } else {
                self.get_default_model(org_id).await?
            }
        } else {
            self.get_default_model(org_id).await?
        };

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

    fn sqldb_store(&self) -> everruns_core::traits::SessionSqlDbStoreRef {
        self.sqldb_store.clone()
    }

    fn storage_store(&self) -> Arc<dyn everruns_core::traits::SessionStorageStore> {
        self.storage_store
            .clone()
            .expect("DirectWorkerAdapters: storage_store not set (call with_storage_store)")
    }

    fn connection_resolver(&self) -> Arc<dyn everruns_core::traits::UserConnectionResolver> {
        self.connection_resolver.clone().expect(
            "DirectWorkerAdapters: connection_resolver not set (call with_connection_resolver)",
        )
    }

    fn schedule_store(&self, org_id: i64) -> Arc<dyn everruns_core::traits::SessionScheduleStore> {
        Arc::new(crate::storage::DbSessionScheduleStore::new(
            self.db.clone(),
            org_id,
        ))
    }

    fn platform_store(&self, org_id: i64) -> Arc<dyn everruns_core::platform_store::PlatformStore> {
        Arc::new(DirectPlatformStore::new(
            org_id,
            self.db.clone(),
            self.event_service.clone(),
            self.runner.clone(),
        ))
    }

    async fn build_tool_registry(&self, _org_id: i64, agent_id: Uuid) -> Result<ToolRegistry> {
        let capability_rows = self
            .db
            .get_agent_capabilities(agent_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get agent capabilities: {}", e);
                store_error("Failed to get agent capabilities")
            })?;
        self.build_tool_registry_with_capabilities(&capability_rows)
            .await
    }
}

impl DirectWorkerAdapters {
    /// Get a harness by ID (direct DB access)
    async fn get_harness_impl(&self, org_id: i64, harness_id: Uuid) -> Result<Option<Harness>> {
        let harness_id_typed = HarnessId::from_uuid(harness_id);
        let row = self
            .db
            .get_harness(org_id, harness_id_typed)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get harness: {}", e);
                store_error("Failed to get harness")
            })?;

        let capabilities = if row.is_some() {
            self.db
                .get_harness_capabilities(harness_id)
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };

        Ok(row.map(|r| Harness {
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
                "active" => HarnessStatus::Active,
                "archived" => HarnessStatus::Archived,
                _ => HarnessStatus::Active,
            },
            created_at: r.created_at,
            updated_at: r.updated_at,
        }))
    }

    /// Build an Agent from a DB row and pre-loaded capability rows.
    ///
    /// Used by both `get_agent` (standalone) and `load_turn_context`
    /// (where capabilities are loaded once and shared across consumers).
    async fn get_agent_with_capabilities(
        &self,
        org_id: i64,
        agent_id: Uuid,
        capability_rows: Vec<AgentCapabilityRow>,
    ) -> Result<Option<Agent>> {
        let agent_id_typed = AgentId::from_uuid(agent_id);
        let row = self
            .db
            .get_agent(org_id, agent_id_typed)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get agent: {}", e);
                store_error("Failed to get agent")
            })?;

        Ok(row.map(|r| Agent {
            public_id: r
                .public_id
                .parse()
                .unwrap_or_else(|_| AgentId::from_uuid(r.id.uuid())),
            internal_id: r.id.uuid(),
            name: r.name,
            description: r.description,
            system_prompt: r.system_prompt,
            default_model_id: r.default_model_id,
            tags: r.tags,
            capabilities: capability_rows
                .into_iter()
                .map(|c| AgentCapabilityConfig::with_config(c.capability_id, c.config))
                .collect(),
            tools: vec![],
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

    /// Build a tool registry from pre-loaded capability rows.
    ///
    /// Shared logic for `build_tool_registry` (standalone, loads its own rows)
    /// and `load_turn_context` (passes pre-loaded rows).
    async fn build_tool_registry_with_capabilities(
        &self,
        capability_rows: &[AgentCapabilityRow],
    ) -> Result<ToolRegistry> {
        let mut registry = ToolRegistry::with_defaults();

        let builtin_cap_ids: Vec<String> = capability_rows
            .iter()
            .map(|r| r.capability_id.clone())
            .filter(|id| !is_mcp_capability(id))
            .collect();

        if !builtin_cap_ids.is_empty() {
            let ctx = SystemPromptContext::without_file_store(everruns_core::SessionId::new());
            let collected =
                collect_capabilities(&builtin_cap_ids, &self.capability_registry, &ctx).await;
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

    /// Build MCP tool definitions from pre-loaded capability rows.
    ///
    /// Shared logic for `build_mcp_tool_definitions` (standalone) and
    /// `load_turn_context` (passes pre-loaded rows to avoid redundant DB call).
    async fn build_mcp_tool_definitions_with_capabilities(
        &self,
        org_id: i64,
        capability_rows: &[AgentCapabilityRow],
    ) -> Result<Vec<ToolDefinition>> {
        use everruns_core::capabilities::mcp::parse_mcp_capability_id;
        use everruns_core::mcp_server::mcp_tool_name;
        use everruns_core::tool_types::{BuiltinTool, DeferrablePolicy, ToolPolicy};

        let mut mcp_tools = Vec::new();

        for cap_row in capability_rows {
            let cap_id = &cap_row.capability_id;
            let server_id = match parse_mcp_capability_id(cap_id) {
                Some(id) => id,
                None => continue,
            };

            let tools = match self
                .mcp_server_service
                .get_tools(org_id, server_id, false)
                .await
            {
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

            let server_name = match self.mcp_server_service.get(org_id, server_id).await {
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
                    display_name: None,
                    description,
                    parameters: tool.input_schema,
                    policy: ToolPolicy::Auto,
                    category: None,
                    deferrable: DeferrablePolicy::default(),
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
        "gemini" => LlmProviderType::Gemini,
        "llmsim" => LlmProviderType::LlmSim,
        _ => {
            tracing::warn!(provider_type = %s, "Unknown provider_type in database; falling back to llmsim");
            LlmProviderType::LlmSim
        }
    }
}

/// Convert an event to a message
fn event_to_message(event: Event) -> Option<Message> {
    match &event.data {
        EventData::InputMessage(data) => Some(data.message.clone()),
        EventData::OutputMessageCompleted(data) => Some(data.message.clone()),
        EventData::ToolCompleted(data) => {
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
                thinking: None,
                thinking_signature: None,
                controls: None,
                metadata: None,
                external_actor: None,
                created_at: event.ts,
            })
        }
        _ => None,
    }
}

// =============================================================================
// DirectPlatformStore - PlatformStore implementation for in-process worker
// =============================================================================

/// Direct PlatformStore backed by StorageBackend + EventService + AgentRunner.
// THREAT[TM-AGENT-017]: All ops org-scoped via org_id field
pub struct DirectPlatformStore {
    org_id: i64,
    db: Arc<StorageBackend>,
    event_service: Arc<EventService>,
    runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
    capability_service: crate::services::CapabilityService,
}

impl DirectPlatformStore {
    pub fn new(
        org_id: i64,
        db: Arc<StorageBackend>,
        event_service: Arc<EventService>,
        runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
    ) -> Self {
        let capability_service = crate::services::CapabilityService::new(db.clone(), None);
        Self {
            org_id,
            db,
            event_service,
            runner,
            capability_service,
        }
    }

    fn base_url_from_env() -> String {
        std::env::var("PUBLIC_APP_URL")
            .or_else(|_| std::env::var("APP_URL"))
            .unwrap_or_else(|_| "http://localhost:9300".to_string())
    }

    fn row_to_harness(
        row: crate::storage::HarnessRow,
        capabilities: Vec<everruns_core::AgentCapabilityConfig>,
    ) -> everruns_core::Harness {
        everruns_core::Harness {
            id: row.id,
            name: row.name,
            description: row.description,
            system_prompt: row.system_prompt,
            default_model_id: row.default_model_id,
            tags: row.tags,
            capabilities,
            status: HarnessStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    fn row_to_agent(
        row: crate::storage::AgentRow,
        capabilities: Vec<everruns_core::AgentCapabilityConfig>,
    ) -> Agent {
        let public_id: AgentId = row
            .public_id
            .parse()
            .unwrap_or_else(|_| AgentId::from_uuid(row.id.uuid()));
        Agent {
            public_id,
            internal_id: row.id.uuid(),
            name: row.name,
            description: row.description,
            system_prompt: row.system_prompt,
            default_model_id: row.default_model_id,
            tags: row.tags,
            capabilities,
            tools: serde_json::from_value(row.tools).unwrap_or_default(),
            status: AgentStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
            usage: None,
        }
    }

    fn row_to_session(row: crate::storage::SessionRow) -> Session {
        let capabilities = serde_json::from_value(row.capabilities).unwrap_or_default();
        Session {
            id: row.id,
            organization_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id: row.harness_id.unwrap_or_else(|| HarnessId::from_seed(1)),
            agent_id: row.agent_id,
            title: row.title,
            preview: None,
            output_preview: None,
            tags: row.tags,
            model_id: row.model_id,
            capabilities,
            tools: serde_json::from_value(row.tools).unwrap_or_default(),
            status: SessionStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
            usage: None,
            is_pinned: None,
            active_schedule_count: None,
            features: vec![],
        }
    }
}

#[async_trait]
impl everruns_core::platform_store::PlatformStore for DirectPlatformStore {
    // =========================================================================
    // Harness Operations
    // =========================================================================

    async fn list_harnesses(&self) -> everruns_core::error::Result<Vec<Harness>> {
        let rows = self
            .db
            .list_harnesses(self.org_id)
            .await
            .map_err(|e| store_error(format!("Failed to list harnesses: {e}")))?;

        let mut harnesses = Vec::with_capacity(rows.len());
        for row in rows {
            let caps = self
                .db
                .get_harness_capabilities(row.id.uuid())
                .await
                .map_err(|e| store_error(format!("Failed to get harness capabilities: {e}")))?
                .into_iter()
                .map(|c| {
                    everruns_core::AgentCapabilityConfig::with_config(c.capability_id, c.config)
                })
                .collect();
            harnesses.push(Self::row_to_harness(row, caps));
        }
        Ok(harnesses)
    }

    async fn get_harness(&self, id: HarnessId) -> everruns_core::error::Result<Option<Harness>> {
        let row = self
            .db
            .get_harness(self.org_id, id)
            .await
            .map_err(|e| store_error(format!("Failed to get harness: {e}")))?;

        match row {
            Some(row) => {
                let caps = self
                    .db
                    .get_harness_capabilities(row.id.uuid())
                    .await
                    .map_err(|e| store_error(format!("Failed to get harness capabilities: {e}")))?
                    .into_iter()
                    .map(|c| {
                        everruns_core::AgentCapabilityConfig::with_config(c.capability_id, c.config)
                    })
                    .collect();
                Ok(Some(Self::row_to_harness(row, caps)))
            }
            None => Ok(None),
        }
    }

    async fn create_harness(
        &self,
        name: &str,
        description: Option<&str>,
        system_prompt: &str,
        capabilities: &[String],
    ) -> everruns_core::error::Result<Harness> {
        use crate::storage::models::CreateHarnessRow;

        let input = CreateHarnessRow {
            name: name.to_string(),
            description: description.map(|s| s.to_string()),
            system_prompt: system_prompt.to_string(),
            default_model_id: None,
            tags: vec!["managed".to_string()],
        };
        let row = self
            .db
            .create_harness(self.org_id, input)
            .await
            .map_err(|e| store_error(format!("Failed to create harness: {e}")))?;

        // Set capabilities
        if !capabilities.is_empty() {
            let caps: Vec<(String, i32, serde_json::Value)> = capabilities
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    (
                        c.clone(),
                        i as i32,
                        serde_json::Value::Object(Default::default()),
                    )
                })
                .collect();
            self.db
                .set_harness_capabilities(row.id.uuid(), caps)
                .await
                .map_err(|e| store_error(format!("Failed to set harness capabilities: {e}")))?;
        }

        self.get_harness(row.id)
            .await?
            .ok_or_else(|| store_error("Harness not found after create"))
    }

    async fn update_harness(
        &self,
        id: HarnessId,
        name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
    ) -> everruns_core::error::Result<Harness> {
        use crate::storage::models::UpdateHarness;

        let update = UpdateHarness {
            name: name.map(|s| s.to_string()),
            description: description.map(|s| s.to_string()),
            system_prompt: system_prompt.map(|s| s.to_string()),
            ..Default::default()
        };
        self.db
            .update_harness(self.org_id, id, update)
            .await
            .map_err(|e| store_error(format!("Failed to update harness: {e}")))?;

        self.get_harness(id)
            .await?
            .ok_or_else(|| store_error("Harness not found after update"))
    }

    async fn delete_harness(&self, id: HarnessId) -> everruns_core::error::Result<()> {
        self.db
            .delete_harness(self.org_id, id)
            .await
            .map_err(|e| store_error(format!("Failed to delete harness: {e}")))?;
        Ok(())
    }

    async fn copy_harness(
        &self,
        id: HarnessId,
        new_name: Option<&str>,
    ) -> everruns_core::error::Result<Harness> {
        // Get the source harness
        let source = self
            .get_harness(id)
            .await?
            .ok_or_else(|| store_error("Source harness not found"))?;

        let copy_name = new_name
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{} (copy)", source.name));

        let cap_ids: Vec<String> = source
            .capabilities
            .iter()
            .map(|c| c.capability_id().to_string())
            .collect();

        self.create_harness(
            &copy_name,
            source.description.as_deref(),
            &source.system_prompt,
            &cap_ids,
        )
        .await
    }

    // =========================================================================
    // Agent Operations
    // =========================================================================

    async fn list_agents(&self) -> everruns_core::error::Result<Vec<Agent>> {
        let rows = self
            .db
            .list_agents(self.org_id)
            .await
            .map_err(|e| store_error(format!("Failed to list agents: {e}")))?;

        let mut agents = Vec::with_capacity(rows.len());
        for row in rows {
            let caps = self
                .db
                .get_agent_capabilities(row.id.uuid())
                .await
                .map_err(|e| store_error(format!("Failed to get agent capabilities: {e}")))?
                .into_iter()
                .map(|c| {
                    everruns_core::AgentCapabilityConfig::with_config(c.capability_id, c.config)
                })
                .collect();
            agents.push(Self::row_to_agent(row, caps));
        }
        Ok(agents)
    }

    async fn get_agent_by_id(&self, id: AgentId) -> everruns_core::error::Result<Option<Agent>> {
        let row = self
            .db
            .get_agent(self.org_id, id)
            .await
            .map_err(|e| store_error(format!("Failed to get agent: {e}")))?;

        match row {
            Some(row) => {
                let caps = self
                    .db
                    .get_agent_capabilities(row.id.uuid())
                    .await
                    .map_err(|e| store_error(format!("Failed to get agent capabilities: {e}")))?
                    .into_iter()
                    .map(|c| {
                        everruns_core::AgentCapabilityConfig::with_config(c.capability_id, c.config)
                    })
                    .collect();
                Ok(Some(Self::row_to_agent(row, caps)))
            }
            None => Ok(None),
        }
    }

    async fn create_agent(
        &self,
        name: &str,
        description: Option<&str>,
        system_prompt: &str,
        capabilities: &[String],
    ) -> everruns_core::error::Result<Agent> {
        use crate::storage::models::CreateAgentRow;

        let public_id = everruns_core::generate_agent_public_id();
        let input = CreateAgentRow {
            public_id: public_id.to_string(),
            name: name.to_string(),
            description: description.map(|s| s.to_string()),
            system_prompt: system_prompt.to_string(),
            default_model_id: None,
            tags: vec!["managed".to_string()],
            tools: serde_json::Value::Array(vec![]),
        };
        let row = self
            .db
            .create_agent(self.org_id, input)
            .await
            .map_err(|e| store_error(format!("Failed to create agent: {e}")))?;

        // Set capabilities
        if !capabilities.is_empty() {
            let caps: Vec<(String, i32, serde_json::Value)> = capabilities
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    (
                        c.clone(),
                        i as i32,
                        serde_json::Value::Object(Default::default()),
                    )
                })
                .collect();
            self.db
                .set_agent_capabilities(row.id.uuid(), caps)
                .await
                .map_err(|e| store_error(format!("Failed to set agent capabilities: {e}")))?;
        }

        self.get_agent_by_id(row.id)
            .await?
            .ok_or_else(|| store_error("Agent not found after create"))
    }

    async fn update_agent(
        &self,
        id: AgentId,
        name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
    ) -> everruns_core::error::Result<Agent> {
        use crate::storage::models::UpdateAgent;

        let update = UpdateAgent {
            name: name.map(|s| s.to_string()),
            description: description.map(|s| s.to_string()),
            system_prompt: system_prompt.map(|s| s.to_string()),
            ..Default::default()
        };
        self.db
            .update_agent(self.org_id, id, update)
            .await
            .map_err(|e| store_error(format!("Failed to update agent: {e}")))?;

        self.get_agent_by_id(id)
            .await?
            .ok_or_else(|| store_error("Agent not found after update"))
    }

    async fn delete_agent(&self, id: AgentId) -> everruns_core::error::Result<()> {
        self.db
            .delete_agent(self.org_id, id)
            .await
            .map_err(|e| store_error(format!("Failed to delete agent: {e}")))?;
        Ok(())
    }

    // =========================================================================
    // Session Operations
    // =========================================================================

    async fn list_sessions(
        &self,
        limit: Option<usize>,
        agent_id: Option<AgentId>,
    ) -> everruns_core::error::Result<Vec<Session>> {
        use crate::api::common::Pagination;

        let pagination = Pagination {
            offset: 0,
            limit: limit.unwrap_or(20) as u32,
        };
        let (rows, _total) = self
            .db
            .list_sessions(self.org_id, agent_id, pagination)
            .await
            .map_err(|e| store_error(format!("Failed to list sessions: {e}")))?;

        Ok(rows.into_iter().map(Self::row_to_session).collect())
    }

    async fn create_session(
        &self,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
    ) -> everruns_core::error::Result<Session> {
        use crate::storage::models::CreateSessionRow;

        let input = CreateSessionRow {
            org_id: self.org_id,
            harness_id: Some(harness_id),
            agent_id,
            title: title.map(|s| s.to_string()),
            tags: vec!["managed".to_string()],
            model_id: None,
            capabilities: serde_json::Value::Array(vec![]),
            tools: serde_json::Value::Array(vec![]),
        };
        let row = self
            .db
            .create_session(input)
            .await
            .map_err(|e| store_error(format!("Failed to create session: {e}")))?;

        Ok(Self::row_to_session(row))
    }

    async fn get_session_by_id(
        &self,
        id: SessionId,
    ) -> everruns_core::error::Result<Option<Session>> {
        let row = self
            .db
            .get_session(self.org_id, id)
            .await
            .map_err(|e| store_error(format!("Failed to get session: {e}")))?;

        Ok(row.map(Self::row_to_session))
    }

    async fn delete_session(&self, id: SessionId) -> everruns_core::error::Result<()> {
        self.db
            .delete_session(self.org_id, id)
            .await
            .map_err(|e| store_error(format!("Failed to delete session: {e}")))?;
        Ok(())
    }

    // =========================================================================
    // Messaging
    // =========================================================================

    async fn send_message(
        &self,
        session_id: SessionId,
        content: &str,
    ) -> everruns_core::error::Result<()> {
        use everruns_core::events::{EventContext, InputMessageData};

        // Get session to retrieve harness_id and agent_id
        let session = self
            .get_session_by_id(session_id)
            .await?
            .ok_or_else(|| store_error("Session not found"))?;

        let message_id = everruns_core::typed_id::MessageId::new();
        let now = chrono::Utc::now();

        // Build input message
        let core_message = everruns_core::Message {
            id: message_id,
            role: everruns_core::MessageRole::User,
            content: vec![everruns_core::ContentPart::text(content)],
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: now,
        };

        // Emit input.message event
        self.event_service
            .emit(EventRequest::new(
                session_id,
                EventContext::empty(),
                InputMessageData::new(core_message),
            ))
            .await
            .map_err(|e| store_error(format!("Failed to emit input message: {e}")))?;

        // Start turn workflow via runner
        if let Some(ref runner) = self.runner {
            runner
                .start_run(
                    self.org_id,
                    session_id,
                    session.harness_id,
                    session.agent_id,
                    message_id,
                )
                .await
                .map_err(|e| store_error(format!("Failed to start turn: {e}")))?;
        } else {
            tracing::warn!("No runner available - message stored but turn not started");
        }

        Ok(())
    }

    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> everruns_core::error::Result<Vec<everruns_core::platform_store::PlatformMessage>> {
        let limit_i32 = limit.unwrap_or(10) as i32;
        let events = self
            .db
            .list_message_events_limited(session_id, Some(limit_i32))
            .await
            .map_err(|e| store_error(format!("Failed to get messages: {e}")))?;

        let messages = events
            .into_iter()
            .filter_map(|ev| {
                let role = match ev.event_type.as_str() {
                    "input.message" => "user".to_string(),
                    "output.message.completed" => "agent".to_string(),
                    _ => return None,
                };

                // Extract text content from the event data
                let content = ev
                    .data
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|p| {
                                if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                                    p.get("text")
                                        .and_then(|t| t.as_str())
                                        .map(|s| s.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();

                if content.is_empty() {
                    return None;
                }

                Some(everruns_core::platform_store::PlatformMessage {
                    role,
                    content,
                    created_at: ev.ts,
                })
            })
            .collect();

        Ok(messages)
    }

    // =========================================================================
    // Turn Management
    // =========================================================================

    async fn wait_for_idle(
        &self,
        session_id: SessionId,
        timeout_secs: Option<u64>,
    ) -> everruns_core::error::Result<String> {
        let timeout = std::time::Duration::from_secs(timeout_secs.unwrap_or(120));
        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_millis(500);

        loop {
            let session = self
                .get_session_by_id(session_id)
                .await?
                .ok_or_else(|| store_error("Session not found"))?;

            match session.status {
                SessionStatus::Idle => return Ok("idle".to_string()),
                SessionStatus::Started => {
                    // Not yet active, keep waiting
                }
                SessionStatus::Active => {
                    // Turn in progress, keep waiting
                }
                SessionStatus::WaitingForToolResults => {
                    return Ok("waiting_for_tool_results".to_string());
                }
            }

            if start.elapsed() > timeout {
                return Ok(format!("timeout (last status: {:?})", session.status));
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    // =========================================================================
    // Capabilities
    // =========================================================================

    async fn list_capabilities(
        &self,
        search: Option<&str>,
    ) -> everruns_core::error::Result<Vec<everruns_core::CapabilityInfo>> {
        let mut caps = self
            .capability_service
            .list_all(self.org_id)
            .await
            .map_err(|e| store_error(format!("Failed to list capabilities: {e}")))?;

        if let Some(q) = search {
            caps.retain(|c| c.matches_search(q));
        }

        caps.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(caps)
    }

    // =========================================================================
    // UI Links
    // =========================================================================

    fn base_url(&self) -> &str {
        // Leak a static string from env for lifetime reasons
        // This is called infrequently and the value is stable
        Box::leak(Self::base_url_from_env().into_boxed_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_to_provider_type_maps_gemini() {
        assert_eq!(string_to_provider_type("gemini").to_string(), "gemini");
    }

    #[test]
    fn string_to_provider_type_falls_back_for_unknown() {
        assert_eq!(
            string_to_provider_type("custom-provider").to_string(),
            "llmsim"
        );
    }

    // =========================================================================
    // Capability deduplication helpers (EVE-47)
    // =========================================================================

    /// Build a DirectWorkerAdapters with in-memory backends for unit tests.
    fn test_adapters() -> DirectWorkerAdapters {
        let db = Arc::new(crate::storage::StorageBackend::in_memory());
        let event_service = Arc::new(crate::services::EventService::new(db.clone()));
        let llm_resolver = Arc::new(crate::services::LlmResolverService::new(db.clone(), None));
        let mcp_server_service = Arc::new(crate::services::McpServerService::new(db.clone(), None));
        let cap_registry = CapabilityRegistry::new();
        let sqldb_backend = Arc::new(everruns_session_sqldb::InMemorySqlDbBackend::new());
        let sqldb_store: everruns_core::traits::SessionSqlDbStoreRef = Arc::new(
            everruns_session_sqldb::InMemorySqlDbStore::new(sqldb_backend),
        );

        DirectWorkerAdapters::new(
            db,
            event_service,
            llm_resolver,
            mcp_server_service,
            cap_registry,
            sqldb_store,
        )
    }

    #[tokio::test]
    async fn get_agent_with_capabilities_returns_none_for_missing_agent() {
        let adapters = test_adapters();
        let result = adapters
            .get_agent_with_capabilities(everruns_core::DEFAULT_ORG_ID, Uuid::new_v4(), vec![])
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_agent_with_capabilities_attaches_supplied_capabilities() {
        let adapters = test_adapters();
        // Seed an agent in the in-memory DB
        let agent_id = seed_agent(&adapters.db).await;

        let cap_rows = vec![fake_capability_row(agent_id, "web_search")];

        let agent = adapters
            .get_agent_with_capabilities(everruns_core::DEFAULT_ORG_ID, agent_id, cap_rows)
            .await
            .unwrap()
            .expect("agent should exist");

        assert_eq!(agent.capabilities.len(), 1);
        assert_eq!(agent.capabilities[0].capability_id(), "web_search");
    }

    #[tokio::test]
    async fn build_tool_registry_with_empty_capabilities() {
        let adapters = test_adapters();
        let registry = adapters
            .build_tool_registry_with_capabilities(&[])
            .await
            .unwrap();
        // Default registry has built-in tools; verify it returns a valid
        // registry without panicking.
        assert!(!registry.is_empty());
    }

    #[tokio::test]
    async fn build_mcp_tool_definitions_with_empty_capabilities() {
        let adapters = test_adapters();
        let tools = adapters
            .build_mcp_tool_definitions_with_capabilities(everruns_core::DEFAULT_ORG_ID, &[])
            .await
            .unwrap();
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn build_mcp_tool_definitions_skips_non_mcp_capabilities() {
        let adapters = test_adapters();
        let agent_id = Uuid::new_v4();
        let rows = vec![
            fake_capability_row(agent_id, "web_search"),
            fake_capability_row(agent_id, "code_execution"),
        ];
        let tools = adapters
            .build_mcp_tool_definitions_with_capabilities(everruns_core::DEFAULT_ORG_ID, &rows)
            .await
            .unwrap();
        // None of these are MCP capabilities, so no tool definitions produced
        assert!(tools.is_empty());
    }

    // =========================================================================
    // grep_files parity tests (EVE-58)
    // =========================================================================

    /// Seed a file into the in-memory store for grep tests.
    async fn seed_file(db: &StorageBackend, session_id: Uuid, path: &str, content: &str) {
        use crate::storage::models::CreateSessionFileRow;
        let create = CreateSessionFileRow {
            session_id: SessionId::from_uuid(session_id),
            path: path.to_string(),
            content: Some(content.as_bytes().to_vec()),
            is_directory: false,
            is_readonly: false,
        };
        db.create_session_file(create).await.expect("seed file");
    }

    #[tokio::test]
    async fn grep_files_returns_matching_lines() {
        let adapters = test_adapters();
        let session_id = Uuid::new_v4();

        seed_file(
            &adapters.db,
            session_id,
            "/src/main.rs",
            "fn main() {\n    println!(\"hello world\");\n}\n",
        )
        .await;

        let results = adapters
            .grep_files(session_id, "hello", None)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "/src/main.rs");
        assert_eq!(results[0].line_number, 2);
        assert!(results[0].line.contains("hello world"));
    }

    #[tokio::test]
    async fn grep_files_returns_empty_for_no_match() {
        let adapters = test_adapters();
        let session_id = Uuid::new_v4();

        seed_file(
            &adapters.db,
            session_id,
            "/src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
        )
        .await;

        let results = adapters
            .grep_files(session_id, "nonexistent_pattern", None)
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn grep_files_matches_multiple_lines_and_files() {
        let adapters = test_adapters();
        let session_id = Uuid::new_v4();

        seed_file(
            &adapters.db,
            session_id,
            "/a.txt",
            "TODO fix this\nall good\nTODO refactor\n",
        )
        .await;
        seed_file(
            &adapters.db,
            session_id,
            "/b.txt",
            "all good\nTODO cleanup\n",
        )
        .await;

        let results = adapters.grep_files(session_id, "TODO", None).await.unwrap();

        assert_eq!(results.len(), 3);
        // Verify line numbers
        let a_matches: Vec<_> = results.iter().filter(|m| m.path == "/a.txt").collect();
        assert_eq!(a_matches.len(), 2);
        assert_eq!(a_matches[0].line_number, 1);
        assert_eq!(a_matches[1].line_number, 3);

        let b_matches: Vec<_> = results.iter().filter(|m| m.path == "/b.txt").collect();
        assert_eq!(b_matches.len(), 1);
        assert_eq!(b_matches[0].line_number, 2);
    }

    #[tokio::test]
    async fn grep_files_supports_regex_patterns() {
        let adapters = test_adapters();
        let session_id = Uuid::new_v4();

        seed_file(
            &adapters.db,
            session_id,
            "/nums.rs",
            "let x = 42;\nlet y = 100;\nlet z = 7;\n",
        )
        .await;

        let results = adapters
            .grep_files(session_id, r"\d{3}", None)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].line.contains("100"));
    }

    #[tokio::test]
    async fn grep_files_invalid_regex_returns_error() {
        let adapters = test_adapters();
        let session_id = Uuid::new_v4();

        let result = adapters.grep_files(session_id, "[invalid", None).await;
        assert!(result.is_err());
    }

    // ---- helpers ----

    /// Seed a minimal agent into the in-memory store and return its UUID.
    async fn seed_agent(db: &StorageBackend) -> Uuid {
        use crate::storage::models::CreateAgentRow;
        let create = CreateAgentRow {
            public_id: AgentId::new().to_string(),
            name: "test-agent".to_string(),
            description: None,
            system_prompt: String::new(),
            default_model_id: None,
            tags: vec![],
            tools: serde_json::Value::Array(vec![]),
        };
        let row = db
            .create_agent(everruns_core::DEFAULT_ORG_ID, create)
            .await
            .expect("seed agent");
        row.id.uuid()
    }

    // =========================================================================
    // Cross-org isolation regression tests (EVE-56)
    // =========================================================================

    /// Regression test: schedule_store must be scoped to the provided org_id,
    /// not a hardcoded default. Before EVE-56, schedule_store() had no org_id
    /// parameter and used DEFAULT_ORG_ID for all calls.
    #[test]
    fn schedule_store_is_scoped_to_org_id() {
        let adapters = test_adapters();
        let store_org1 = adapters.schedule_store(1);
        let store_org2 = adapters.schedule_store(2);
        // Verify these are distinct instances (different Arc pointers)
        assert!(!Arc::ptr_eq(&store_org1, &store_org2));
    }

    /// Regression test: platform_store must use the provided org_id, not
    /// DEFAULT_ORG_ID. Agents created in org 1 must not be visible through
    /// org 2's platform store.
    #[tokio::test]
    async fn platform_store_cross_org_isolation() {
        use everruns_core::platform_store::PlatformStore;

        let adapters = test_adapters();
        let agent_id = seed_agent(&adapters.db).await;

        // Agent seeded in org 1 should be visible via org 1's platform store
        let store_org1 = adapters.platform_store(everruns_core::DEFAULT_ORG_ID);
        let agent = store_org1
            .get_agent_by_id(everruns_core::AgentId::from_uuid(agent_id))
            .await
            .unwrap();
        assert!(agent.is_some(), "agent should be visible in org 1");

        // Same agent must NOT be visible via org 2's platform store
        let store_org2 = adapters.platform_store(999);
        let agent = store_org2
            .get_agent_by_id(everruns_core::AgentId::from_uuid(agent_id))
            .await
            .unwrap();
        assert!(agent.is_none(), "agent must NOT be visible in org 999");
    }

    /// Regression test: image resolution must receive org_id so the gRPC
    /// service scopes the lookup. Before EVE-56, resolve_image had no org_id
    /// parameter.
    #[tokio::test]
    async fn resolve_image_requires_org_id() {
        let adapters = test_adapters();
        // resolve_image now requires org_id — compile-time proof the parameter exists
        let result = adapters.resolve_image(1, Uuid::new_v4()).await.unwrap();
        assert!(result.is_none());
    }

    /// Build a fake `AgentCapabilityRow` for testing.
    fn fake_capability_row(agent_id: Uuid, capability_id: &str) -> AgentCapabilityRow {
        AgentCapabilityRow {
            id: Uuid::new_v4(),
            agent_id: AgentId::from_uuid(agent_id),
            capability_id: capability_id.to_string(),
            position: 0,
            config: serde_json::Value::Object(Default::default()),
            created_at: chrono::Utc::now(),
        }
    }

    // =========================================================================
    // MCP server API key decryption tests (EVE-55)
    // =========================================================================

    fn test_encryption() -> Arc<crate::storage::EncryptionService> {
        Arc::new(
            crate::storage::EncryptionService::new(
                "kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=",
                &[],
            )
            .unwrap(),
        )
    }

    fn test_adapters_with_encryption() -> DirectWorkerAdapters {
        let encryption = test_encryption();
        let db = Arc::new(crate::storage::StorageBackend::in_memory());
        let event_service = Arc::new(crate::services::EventService::new(db.clone()));
        let llm_resolver = Arc::new(crate::services::LlmResolverService::new(db.clone(), None));
        let mcp_server_service = Arc::new(crate::services::McpServerService::new(
            db.clone(),
            Some(encryption),
        ));
        let cap_registry = CapabilityRegistry::new();
        let sqldb_backend = Arc::new(everruns_session_sqldb::InMemorySqlDbBackend::new());
        let sqldb_store: everruns_core::traits::SessionSqlDbStoreRef = Arc::new(
            everruns_session_sqldb::InMemorySqlDbStore::new(sqldb_backend),
        );

        DirectWorkerAdapters::new(
            db,
            event_service,
            llm_resolver,
            mcp_server_service,
            cap_registry,
            sqldb_store,
        )
    }

    /// Seed an MCP server with an optional encrypted API key. Returns server UUID.
    async fn seed_mcp_server(
        db: &crate::storage::StorageBackend,
        name: &str,
        api_key_encrypted: Option<Vec<u8>>,
    ) -> Uuid {
        use crate::storage::models::CreateMcpServerRow;
        let row = db
            .create_mcp_server(
                everruns_core::DEFAULT_ORG_ID,
                CreateMcpServerRow {
                    name: name.to_string(),
                    description: None,
                    url: "https://example.com/mcp".to_string(),
                    transport_type: "streamable_http".to_string(),
                    api_key_encrypted,
                    headers: None,
                    settings: None,
                },
            )
            .await
            .expect("seed mcp server");
        row.id.uuid()
    }

    #[tokio::test]
    async fn get_mcp_server_by_prefix_returns_none_api_key_when_not_set() {
        let adapters = test_adapters();
        seed_mcp_server(&adapters.db, "My Server", None).await;

        let info = adapters
            .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, "my_server")
            .await
            .unwrap();
        assert!(info.api_key.is_none());
        assert_eq!(info.name, "My Server");
    }

    #[tokio::test]
    async fn get_mcp_server_by_prefix_returns_decrypted_api_key() {
        let adapters = test_adapters_with_encryption();
        let encrypted = test_encryption().encrypt_string("sk-live-key").unwrap();
        seed_mcp_server(&adapters.db, "Auth Server", Some(encrypted)).await;

        let info = adapters
            .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, "auth_server")
            .await
            .unwrap();
        assert_eq!(info.api_key.as_deref(), Some("sk-live-key"));
    }

    #[tokio::test]
    async fn get_mcp_server_by_prefix_errors_when_encryption_not_configured() {
        // Adapters without encryption
        let adapters = test_adapters();
        let encrypted = test_encryption().encrypt_string("sk-secret").unwrap();
        seed_mcp_server(&adapters.db, "No Enc Server", Some(encrypted)).await;

        let result = adapters
            .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, "no_enc_server")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_mcp_server_by_prefix_errors_when_server_not_found() {
        let adapters = test_adapters();
        let result = adapters
            .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, "nonexistent")
            .await;
        assert!(result.is_err());
    }
}
