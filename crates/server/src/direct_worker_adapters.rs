// Direct implementation of WorkerAdapters for in-process worker
//
// Decision: Uses StorageBackend, domains, and infra helpers directly (no gRPC)
// Decision: Used by in-process worker in DEV_MODE
//
// This implementation provides the same interface as GrpcWorkerAdapters
// but with direct access to the storage backend, domains, and infra helpers.

use crate::kernel_imports::{
    Caller, ContentPart, EgressRequest, EgressRequestKind, EgressService, EventData, Message,
    MessageRole, ToolResultContentPart, UtilityLlmService,
    everruns_provider::driver_registry::DriverRegistry, everruns_provider::provider::DriverId,
    everruns_provider::tool_types::ToolDefinition, resolve_runtime_capabilities,
};
use crate::kernel_imports::{
    connection_services::ProviderCredentialStore, delegation_services::SessionCreationAuthority,
    everruns_provider::model_spec::ModelSpec, image_services::CreateStoredImage,
    image_services::ImageArtifactStore, image_services::ResolvedImage, image_services::StoredImage,
    image_services::StoredImageInfo, tool_execution::BudgetChecker,
    tool_execution::PaymentAuthority,
};
use async_trait::async_trait;
use everruns_capability::CapabilityRef as AgentCapabilityConfig;
use everruns_core::budget::{BudgetSummary, BudgetToolResponse};
use everruns_core::capabilities::{CapabilityRegistry, collect_message_filters_only};
use everruns_core::connection_services::ProviderCredentials;
use everruns_core::events::{Event, EventRequest};
use everruns_core::message_retriever::MessageRetriever;
use everruns_core::permissions::PermissionResolver;
use everruns_core::session_file::{
    FileInfo, FileStat, GrepMatch, GrepOptions, GrepSearchResult, SessionFile,
};
use everruns_platform::{Agent, AgentStatus};
use everruns_platform::{Harness, HarnessStatus, merge_harness};
use everruns_platform::{Session, SessionParticipant, SessionStatus};
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use everruns_worker::mcp_executor::McpServerInfo;
use everruns_worker::worker_adapters::{TurnContext, WorkerAdapters};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::domains::budgets::BudgetService;
use crate::domains::mcp_servers::McpServerService;
use crate::domains::mcp_servers::scoped_mcp::{
    build_scoped_mcp_tool_definitions, merge_effective_scoped_mcp_servers_with_capabilities,
    resolve_scoped_mcp_server_with_capabilities, validate_scoped_mcp_servers,
};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::max_iterations;
use crate::services::{EventService, ProviderResolverService};
use crate::storage::models::{AgentCapabilityRow, AgentRow, UpdateSession};
use crate::storage::{EncryptionService, StorageBackend};
use everruns_durable::WorkflowEventStore;
use serde::Deserialize;

// Helper to create store errors
fn store_error(msg: impl Into<String>) -> AgentLoopError {
    AgentLoopError::store(msg)
}

#[derive(Debug, Deserialize)]
struct CommandPage<T> {
    data: Vec<T>,
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

struct DirectBudgetChecker {
    budget_service: Arc<BudgetService>,
    org_id: i64,
    agent_id: Option<String>,
}

struct DirectImageArtifactStore {
    db: Arc<StorageBackend>,
    org_id: i64,
}

#[async_trait]
impl ImageArtifactStore for DirectImageArtifactStore {
    async fn create_image(&self, input: CreateStoredImage) -> Result<StoredImageInfo> {
        let (thumbnail_data, thumbnail_content_type) =
            crate::api::images::generate_thumbnail(&input.data, &input.content_type)
                .map(|(data, content_type)| (Some(data), Some(content_type)))
                .unwrap_or((None, None));

        let row = self
            .db
            .create_image(
                self.org_id,
                crate::storage::models::CreateImageRow {
                    org_id: self.org_id,
                    filename: input.filename,
                    content_type: input.content_type,
                    size_bytes: input.data.len() as i64,
                    data: input.data,
                    thumbnail_data,
                    thumbnail_content_type,
                    metadata: input.metadata,
                },
            )
            .await
            .map_err(|e| store_error(format!("Failed to create image artifact: {e}")))?;

        Ok(StoredImageInfo {
            id: row.id,
            filename: row.filename,
            content_type: row.content_type,
            size_bytes: row.size_bytes,
            metadata: row.metadata,
            created_at: row.created_at,
        })
    }

    async fn get_image(
        &self,
        image_id: everruns_provider::typed_id::ImageId,
    ) -> Result<Option<StoredImage>> {
        let row = self
            .db
            .get_image(self.org_id, image_id.uuid())
            .await
            .map_err(|e| store_error(format!("Failed to get image artifact: {e}")))?;

        Ok(row.map(|row| StoredImage {
            info: StoredImageInfo {
                id: row.id,
                filename: row.filename,
                content_type: row.content_type,
                size_bytes: row.size_bytes,
                metadata: row.metadata,
                created_at: row.created_at,
            },
            data: row.data,
        }))
    }

    async fn get_image_info(
        &self,
        image_id: everruns_provider::typed_id::ImageId,
    ) -> Result<Option<StoredImageInfo>> {
        let row = self
            .db
            .get_image_info(self.org_id, image_id.uuid())
            .await
            .map_err(|e| store_error(format!("Failed to get image artifact info: {e}")))?;

        Ok(row.map(|row| StoredImageInfo {
            id: row.id,
            filename: row.filename,
            content_type: row.content_type,
            size_bytes: row.size_bytes,
            metadata: row.metadata,
            created_at: row.created_at,
        }))
    }
}

struct DirectProviderCredentialStore {
    provider_resolver: Arc<ProviderResolverService>,
    org_id: i64,
}

#[async_trait]
impl ProviderCredentialStore for DirectProviderCredentialStore {
    async fn get_default_provider_credentials(
        &self,
        provider_type: &str,
    ) -> Result<Option<ProviderCredentials>> {
        Ok(self
            .provider_resolver
            .resolve_provider_credentials(self.org_id, provider_type)
            .await
            .map_err(|e| store_error(format!("Failed to resolve provider credentials: {e}")))?
            .map(|resolved| ProviderCredentials {
                api_key: resolved.api_key,
                base_url: resolved.base_url,
            }))
    }
}

#[async_trait]
impl BudgetChecker for DirectBudgetChecker {
    async fn check_budgets(&self, session_id: &str) -> Result<BudgetToolResponse> {
        let all_budgets = self
            .budget_service
            .list_budgets_for_session_hierarchy(self.org_id, session_id, self.agent_id.as_deref())
            .await;

        if all_budgets.is_empty() {
            return Ok(BudgetToolResponse {
                status: "no_budgets".to_string(),
                budgets: vec![],
                hint: Some(
                    "No budgets are configured for this session. You can proceed without budget constraints.".to_string(),
                ),
            });
        }

        let check = self
            .budget_service
            .check_budgets_for_session(self.org_id, session_id, self.agent_id.as_deref())
            .await;

        let budgets = all_budgets
            .into_iter()
            .map(|budget| {
                let percent_remaining = if budget.limit > 0.0 {
                    (budget.balance / budget.limit * 100.0).clamp(0.0, 100.0)
                } else {
                    100.0
                };

                BudgetSummary {
                    currency: budget.currency,
                    limit: budget.limit,
                    balance: budget.balance,
                    soft_limit: budget.soft_limit,
                    percent_remaining: (percent_remaining * 10.0).round() / 10.0,
                    status: budget.status,
                }
            })
            .collect::<Vec<_>>();

        let status = match check.action.as_str() {
            "stop" => "exhausted",
            "pause" => "paused",
            "warn" => "warning",
            _ => "active",
        }
        .to_string();

        let hint = if status == "active" {
            let min_pct = budgets
                .iter()
                .map(|budget| budget.percent_remaining)
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or(100.0);
            if min_pct < 50.0 {
                Some(format!(
                    "{min_pct}% of budget remaining. Consider prioritizing task completion."
                ))
            } else {
                Some(format!("{min_pct}% of budget remaining."))
            }
        } else {
            check.message.clone()
        };

        Ok(BudgetToolResponse {
            status,
            budgets,
            hint,
        })
    }
}

// =============================================================================
// DirectWorkerAdapters Implementation
// =============================================================================

/// Direct storage-backed worker adapters for in-process worker
#[derive(Clone)]
pub struct DirectWorkerAdapters {
    db: Arc<StorageBackend>,
    event_service: Arc<EventService>,
    budget_service: Option<Arc<BudgetService>>,
    provider_resolver: Arc<ProviderResolverService>,
    mcp_server_service: Arc<McpServerService>,
    capability_registry: CapabilityRegistry,
    connector_registry: everruns_platform::connector::ConnectorRegistry,
    driver_registry: DriverRegistry,
    utility_llm_service: Option<Arc<dyn UtilityLlmService>>,
    egress_service: Option<Arc<dyn EgressService>>,
    sqldb_store: std::sync::Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore>,
    storage_store: Option<Arc<dyn everruns_core::session_services::SessionStorageStore>>,
    connection_resolver:
        Option<Arc<dyn everruns_core::connection_services::UserConnectionResolver>>,
    /// Platform vector store for Knowledge Index retrieval (`search_index`).
    vector_store: Option<Arc<dyn everruns_platform::vector_store::VectorStore>>,
    runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
    encryption: Option<Arc<EncryptionService>>,
    in_memory_compaction_checkpoint_store: Arc<everruns_host::InMemoryCompactionCheckpointStore>,
    proactive_compaction_attempts: Arc<everruns_core::ProactiveCompactionAttemptTracker>,
    workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    permission_resolver: Arc<dyn PermissionResolver>,
    virtual_registry:
        Option<Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>>,
    org_rate_limiter: Option<Arc<crate::auth::rate_limit::OrgRateLimiter>>,
    quota: crate::domains::session_files::limits::QuotaLimits,
}

impl DirectWorkerAdapters {
    /// Create new direct adapters with access to storage and services
    pub fn new(
        db: Arc<StorageBackend>,
        event_service: Arc<EventService>,
        provider_resolver: Arc<ProviderResolverService>,
        mcp_server_service: Arc<McpServerService>,
        capability_registry: CapabilityRegistry,
        driver_registry: DriverRegistry,
        sqldb_store: std::sync::Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore>,
    ) -> Self {
        Self {
            db,
            event_service,
            budget_service: None,
            provider_resolver,
            mcp_server_service,
            capability_registry,
            connector_registry: everruns_platform::connector::ConnectorRegistry::new(),
            driver_registry,
            utility_llm_service: None,
            egress_service: None,
            sqldb_store,
            storage_store: None,
            connection_resolver: None,
            vector_store: None,
            runner: None,
            encryption: None,
            in_memory_compaction_checkpoint_store: Arc::new(
                everruns_host::InMemoryCompactionCheckpointStore::default(),
            ),
            proactive_compaction_attempts: Arc::new(
                everruns_core::ProactiveCompactionAttemptTracker::default(),
            ),
            workflow_store: None,
            permission_resolver: Arc::new(everruns_core::DefaultPermissionResolver),
            virtual_registry: None,
            org_rate_limiter: None,
            quota: crate::domains::session_files::limits::QuotaLimits::from_env(),
        }
    }

    /// Load the stored platform Session record (server-internal; EVE-882).
    ///
    /// `WorkerAdapters::get_session` projects this into the portable
    /// `ExecutionSession`; internal paths that need platform-only fields
    /// (agent version pinning, scoped-MCP wiring) read the record here.
    pub(crate) async fn get_stored_session(
        &self,
        org_id: i64,
        session_id: Uuid,
    ) -> Result<Option<Session>> {
        let session_id_typed = SessionId::from_uuid(session_id);
        let row = self
            .db
            .get_session(org_id, session_id_typed)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get session: {}", e);
                store_error("Failed to get session")
            })?;

        let Some(r) = row else {
            return Ok(None);
        };
        let capabilities = serde_json::from_value(r.capabilities).unwrap_or_default();
        let capabilities =
            crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
                &self.db,
                org_id,
                capabilities,
            )
            .await
            .unwrap_or_default();

        Ok(Some({
            // Parse capabilities from JSON
            Session {
                source: everruns_platform::SessionSource::from(r.source.as_str()),
                run_summary: r.run_summary.clone(),
                activity: everruns_platform::SessionActivity::derive(
                    &everruns_platform::SessionStatus::from(r.status.as_str()),
                    r.last_turn_status.as_deref(),
                ),
                id: r.id,
                organization_id: everruns_core::org_public_id_from_internal(org_id),
                workspace_id: everruns_provider::typed_id::WorkspaceId::from_uuid(r.workspace_id),
                harness_id: r.harness_id.unwrap_or_else(|| HarnessId::from_seed(1)),
                agent_id: r.agent_id,
                agent_version_id: r.agent_version_id,
                agent_identity_id: r.agent_identity_id,
                owner_principal_id: r.owner_principal_id,
                resolved_owner_user_id: r.resolved_owner_user_id,
                owner: None,
                effective_owner: None,
                title: r.title,
                goal: r.goal,
                locale: r.locale,
                preview: None,
                output_preview: None,
                tags: r.tags,
                model_id: r.model_id,
                capabilities,
                tools: serde_json::from_value(r.tools).unwrap_or_default(),
                mcp_servers: serde_json::from_value(r.mcp_servers).unwrap_or_default(),
                system_prompt: r.system_prompt,
                initial_files: serde_json::from_value(r.initial_files).unwrap_or_default(),
                network_access: r
                    .network_access
                    .and_then(|v| serde_json::from_value(v).ok()),
                hints: r.hints.and_then(|v| serde_json::from_value(v).ok()),
                max_iterations: max_iterations::from_db(r.max_iterations),
                parallel_tool_calls: r.parallel_tool_calls,
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
                archived_at: None,
                active_schedule_count: None,
                event_count: None,
                task_count: None,
                file_count: None,
                features: vec![],
                parent_session_id: r.parent_session_id,
                forked_from_session_id: r.forked_from_session_id,
                forked_from_sequence: r.forked_from_sequence,
                blueprint_id: r.blueprint_id,
                blueprint_config: r.blueprint_config,
            }
        }))
    }

    async fn load_turn_messages(
        &self,
        session: &Session,
        agent: Option<&Agent>,
        harness: Option<&Harness>,
    ) -> Result<Vec<Message>> {
        // Status-agnostic projections: message-filter resolution historically
        // saw the stored records regardless of lifecycle status (EVE-877,
        // EVE-881). The harness arrives pre-merged, so its plain definition is
        // the effective configuration.
        let harness_definition = harness.map(|h| h.definition()).unwrap_or_default();
        let agent_definition = agent.map(|a| a.definition());
        // EVE-882: capability resolution consumes the portable execution view.
        let execution_session = session.execution_session();
        let resolved = resolve_runtime_capabilities(
            &harness_definition,
            agent_definition.as_ref(),
            &execution_session,
            &self.capability_registry,
        );
        let message_filters = collect_message_filters_only(
            &resolved.effective_overlay.capabilities,
            &self.capability_registry,
        );
        let mut query = everruns_core::MessageQuery::new(session.id);
        message_filters.apply_message_filters(&mut query);

        let retriever = crate::storage::DbMessageRetriever::new(self.db.clone());
        let mut messages = retriever.load_filtered(query).await.map_err(|error| {
            tracing::error!("Failed to load bounded turn messages: {error}");
            store_error("Failed to load messages")
        })?;
        message_filters.apply_post_load_filters(&mut messages);
        Ok(messages)
    }

    /// Set the agent runner for platform management tools (send_message, etc.)
    pub fn with_runner(mut self, runner: Arc<dyn everruns_worker::AgentRunner>) -> Self {
        self.runner = Some(runner);
        self
    }

    pub fn with_connector_registry(
        mut self,
        registry: everruns_platform::connector::ConnectorRegistry,
    ) -> Self {
        self.connector_registry = registry;
        self
    }

    /// Set the budget service for in-process budget tool parity.
    pub fn with_budget_service(mut self, service: Arc<BudgetService>) -> Self {
        self.budget_service = Some(service);
        self
    }

    /// Set the per-org outbound tool-call rate limiter (TM-TOOL-009).
    pub fn with_org_rate_limiter(
        mut self,
        limiter: Arc<crate::auth::rate_limit::OrgRateLimiter>,
    ) -> Self {
        self.org_rate_limiter = Some(limiter);
        self
    }

    /// Set the session storage store for kv_store/secret_store tools
    pub fn with_storage_store(
        mut self,
        store: Arc<dyn everruns_core::session_services::SessionStorageStore>,
    ) -> Self {
        self.storage_store = Some(store);
        self
    }

    /// Set the virtual mount registry for serving files from memory
    pub fn with_virtual_registry(
        mut self,
        registry: Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>,
    ) -> Self {
        self.virtual_registry = Some(registry);
        self
    }

    /// Set the user connection resolver for lazy token lookup
    pub fn with_connection_resolver(
        mut self,
        resolver: Arc<dyn everruns_core::connection_services::UserConnectionResolver>,
    ) -> Self {
        self.connection_resolver = Some(resolver);
        self
    }

    /// Set the platform vector store for Knowledge Index retrieval.
    pub fn with_vector_store(
        mut self,
        vector_store: Arc<dyn everruns_platform::vector_store::VectorStore>,
    ) -> Self {
        self.vector_store = Some(vector_store);
        self
    }

    pub fn with_encryption(mut self, encryption: Option<Arc<EncryptionService>>) -> Self {
        self.encryption = encryption;
        self
    }

    pub fn with_workflow_store(
        mut self,
        workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    ) -> Self {
        self.workflow_store = workflow_store;
        self
    }

    pub fn with_permission_resolver(mut self, resolver: Arc<dyn PermissionResolver>) -> Self {
        self.permission_resolver = resolver;
        self
    }

    pub fn with_utility_llm_service(mut self, service: Arc<dyn UtilityLlmService>) -> Self {
        self.utility_llm_service = Some(service);
        self
    }

    pub fn with_egress_service(mut self, service: Arc<dyn EgressService>) -> Self {
        self.egress_service = Some(service);
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

        match self.db.create_session_file(input).await {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("duplicate key")
                    || msg.contains("unique constraint")
                    || msg.contains("UNIQUE constraint")
                {
                    // Race: directory was created concurrently; verify it's a directory
                    if let Some(existing) = self
                        .db
                        .get_session_file(session_id, path)
                        .await
                        .map_err(|e| store_error(format!("Failed to check directory: {}", e)))?
                        && existing.is_directory
                    {
                        return Ok(());
                    }
                    Err(store_error(format!("A file exists at path: {}", path)))
                } else {
                    Err(store_error(format!("Failed to create directory: {}", e)))
                }
            }
        }
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
        // Look up by public_id, then fetch capabilities using internal id from the row.
        let public_id = AgentId::from_uuid(agent_id).to_string();
        let row = self
            .db
            .get_agent_by_public_id(org_id, &public_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to get agent: {}", e);
                store_error("Failed to get agent")
            })?;
        match row {
            Some(r) => {
                let capabilities = self
                    .db
                    .get_agent_capabilities(r.id.uuid())
                    .await
                    .unwrap_or_default();
                let capabilities = self
                    .hydrate_capability_rows(org_id, capabilities)
                    .await
                    .unwrap_or_default();
                Ok(Some(Self::row_to_agent(r, capabilities)))
            }
            None => Ok(None),
        }
    }

    // =========================================================================
    // Session Operations
    // =========================================================================

    async fn get_session(
        &self,
        org_id: i64,
        session_id: Uuid,
    ) -> Result<Option<everruns_core::ExecutionSession>> {
        // Loading seam (EVE-882): project the stored record into the portable
        // execution view before it reaches host execution.
        Ok(self
            .get_stored_session(org_id, session_id)
            .await?
            .map(|session| session.execution_session()))
    }

    async fn set_session_status(&self, org_id: i64, session_id: Uuid, status: &str) -> Result<()> {
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

        // Acknowledgement only (EVE-882): status mutation exposes no session
        // record to the worker path.
        Ok(())
    }

    async fn set_session_title(
        &self,
        org_id: i64,
        session_id: Uuid,
        title: String,
    ) -> Result<everruns_core::ExecutionSession> {
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

    async fn load_message_history(
        &self,
        query: everruns_core::MessageQuery,
    ) -> Result<everruns_core::MessageHistory> {
        everruns_core::MessageRetriever::load_filtered_history(
            &crate::storage::DbMessageRetriever::new(self.db.clone()),
            query,
        )
        .await
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

    async fn get_model_spec(&self, org_id: i64, model_id: Uuid) -> Result<Option<ModelSpec>> {
        let resolved = self
            .provider_resolver
            .resolve_model(org_id, model_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve model: {}", e);
                store_error("Failed to resolve model")
            })?;

        Ok(resolved.map(|r| ModelSpec::on(r.provider_id, r.model_id)))
    }

    async fn get_default_model_spec(&self, org_id: i64) -> Result<Option<ModelSpec>> {
        let resolved = self
            .provider_resolver
            .resolve_default_model(org_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve default model: {}", e);
                store_error("Failed to resolve default model")
            })?;

        Ok(resolved.map(|r| ModelSpec::on(r.provider_id, r.model_id)))
    }

    async fn get_provider_config(
        &self,
        org_id: i64,
        provider: &everruns_provider::runtime_provider::ProviderKey,
    ) -> Result<Option<everruns_provider::driver_registry::ProviderConfig>> {
        let resolved = self
            .provider_resolver
            .resolve_runtime_provider_config(org_id, provider.as_str())
            .await
            .map_err(|error| {
                tracing::error!(%error, "Failed to resolve provider");
                store_error("Failed to resolve provider")
            })?;
        Ok(
            resolved.map(|value| everruns_provider::driver_registry::ProviderConfig {
                provider: provider.clone(),
                provider_type: string_to_provider_type(&value.provider_type),
                api_key: value.api_key,
                base_url: value.base_url,
                metadata: everruns_provider::driver_registry::ProviderMetadata::default(),
                request_options: value.request_options,
            }),
        )
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
        // Check virtual mounts first
        if let Some(registry) = &self.virtual_registry
            && let Some(vf) = registry.read_file(&session_id, path)
        {
            let now = chrono::Utc::now();
            let (content, encoding) = if vf.is_directory {
                (None, "text".to_string())
            } else {
                let (c, e) = SessionFile::encode_content(&vf.content);
                (Some(c), e)
            };
            return Ok(Some(SessionFile {
                id: uuid::Uuid::nil(),
                session_id,
                path: vf.path.clone(),
                name: name_from_path(&vf.path),
                content,
                encoding,
                is_directory: vf.is_directory,
                is_readonly: true,
                size_bytes: vf.content.len() as i64,
                created_at: now,
                updated_at: now,
            }));
        }

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
        // Virtual files are readonly
        if let Some(registry) = &self.virtual_registry
            && registry.is_virtual_path(&session_id, path)
        {
            return Err(store_error(format!(
                "Cannot modify readonly file: {}",
                path
            )));
        }
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

        // Quota check (TM-FS-008 / TM-DOS-005)
        {
            use crate::domains::session_files::limits::check_write_quota;
            let incoming = content_bytes.len() as i64;
            let existing_size = existing.as_ref().map(|f| f.size_bytes).unwrap_or(0);
            check_write_quota(&self.db, session_id, incoming, existing_size, &self.quota)
                .await
                .map_err(|e| store_error(e.to_string()))?;
        }

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
            match self.db.create_session_file(create).await {
                Ok(row) => row,
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("duplicate key")
                        || msg.contains("unique constraint")
                        || msg.contains("UNIQUE constraint")
                    {
                        // Race: file was created concurrently; fall back to update
                        let update = UpdateSessionFile {
                            content: Some(content_bytes.clone()),
                            ..Default::default()
                        };
                        self.db
                            .update_session_file(session_id, path, update)
                            .await
                            .map_err(|e| {
                                tracing::error!("Failed to update file after race: {}", e);
                                store_error("Failed to write file")
                            })?
                            .ok_or_else(|| store_error("File disappeared during update"))?
                    } else {
                        tracing::error!("Failed to create file: {}", e);
                        return Err(store_error("Failed to write file"));
                    }
                }
            }
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

    async fn write_file_if_content_matches(
        &self,
        session_id: Uuid,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        if let Some(registry) = &self.virtual_registry
            && registry.is_virtual_path(&session_id, path)
        {
            return Err(store_error(format!(
                "Cannot modify readonly file: {}",
                path
            )));
        }
        use crate::storage::models::UpdateSessionFile;

        let expected_bytes = SessionFile::decode_content(expected_content, expected_encoding)
            .map_err(|e| store_error(format!("Invalid expected content encoding: {}", e)))?;
        let content_bytes = SessionFile::decode_content(content, encoding)
            .map_err(|e| store_error(format!("Invalid content encoding: {}", e)))?;

        // Fetch metadata only (no content blob) — content equality is enforced
        // atomically in SQL by update_session_file_if_content_matches below.
        let existing = self
            .db
            .get_session_file_info(session_id, path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to check existing file: {}", e);
                store_error("Failed to write file")
            })?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        if existing.is_directory || existing.is_readonly {
            return Ok(None);
        }

        {
            use crate::domains::session_files::limits::check_write_quota;
            check_write_quota(
                &self.db,
                session_id,
                content_bytes.len() as i64,
                existing.size_bytes,
                &self.quota,
            )
            .await
            .map_err(|e| store_error(e.to_string()))?;
        }

        let row = self
            .db
            .update_session_file_if_content_matches(
                session_id,
                path,
                expected_bytes,
                UpdateSessionFile {
                    content: Some(content_bytes),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to conditionally update file: {}", e);
                store_error("Failed to write file")
            })?;

        Ok(row.map(|row| {
            let (content, encoding) = if let Some(bytes) = row.content {
                SessionFile::encode_content(&bytes)
            } else {
                (String::new(), "text".to_string())
            };

            SessionFile {
                id: row.id,
                session_id: row.session_id.uuid(),
                path: row.path.clone(),
                name: name_from_path(&row.path),
                content: Some(content),
                encoding,
                is_directory: row.is_directory,
                is_readonly: row.is_readonly,
                size_bytes: row.size_bytes,
                created_at: row.created_at,
                updated_at: row.updated_at,
            }
        }))
    }

    async fn delete_file(&self, session_id: Uuid, path: &str, recursive: bool) -> Result<bool> {
        if let Some(registry) = &self.virtual_registry
            && registry.is_virtual_path(&session_id, path)
        {
            return Err(store_error(format!(
                "Cannot delete readonly file: {}",
                path
            )));
        }
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

        let mut entries: Vec<FileInfo> = rows
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
            .collect();

        // Merge virtual entries (virtual wins on name conflict)
        if let Some(registry) = &self.virtual_registry {
            let virtual_entries = registry.list_directory(&session_id, path);
            let now = chrono::Utc::now();
            for vf in virtual_entries {
                let name = name_from_path(&vf.path);
                entries.retain(|e| e.name != name);
                entries.push(FileInfo {
                    id: uuid::Uuid::nil(),
                    session_id,
                    path: vf.path,
                    name,
                    is_directory: vf.is_directory,
                    is_readonly: true,
                    size_bytes: vf.size_bytes,
                    created_at: now,
                    updated_at: now,
                });
            }
            entries.sort_by(|a, b| {
                b.is_directory
                    .cmp(&a.is_directory)
                    .then_with(|| a.path.cmp(&b.path))
            });
        }

        Ok(entries)
    }

    async fn stat_file(&self, session_id: Uuid, path: &str) -> Result<Option<FileStat>> {
        // Check virtual mounts first
        if let Some(registry) = &self.virtual_registry
            && let Some(vf) = registry.read_file(&session_id, path)
        {
            return Ok(Some(FileStat {
                path: vf.path.clone(),
                name: name_from_path(&vf.path),
                is_directory: vf.is_directory,
                is_readonly: true,
                size_bytes: vf.content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }));
        }

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
        let results = crate::domains::session_files::grep_session_files(
            &self.db,
            session_id,
            pattern,
            path_pattern,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to grep files: {}", e);
            store_error(format!("Failed to grep files: {}", e))
        })?;

        let mut matches: Vec<GrepMatch> = results.into_iter().flat_map(|r| r.matches).collect();

        // Also search virtual mounts with the shared canonical-path glob semantics.
        if let Some(registry) = &self.virtual_registry
            && let Ok(regex) = regex::Regex::new(pattern)
        {
            let path_matcher = path_pattern
                .map(everruns_core::session_path::GrepPathPattern::new)
                .transpose()
                .unwrap_or(None);
            let virtual_matches = registry.grep(&session_id, &regex, None, None, 512 * 1024);
            matches.extend(
                virtual_matches
                    .into_iter()
                    .filter(|vm| {
                        path_matcher
                            .as_ref()
                            .is_none_or(|matcher| matcher.is_match(&vm.path))
                    })
                    .map(|vm| GrepMatch {
                        path: vm.path,
                        line_number: vm.line_number,
                        line: vm.line,
                    }),
            );
        }

        Ok(matches)
    }

    async fn grep_files_with_options(
        &self,
        session_id: Uuid,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        crate::domains::session_files::service::grep_session_files_with_options(
            &self.db,
            self.virtual_registry.as_deref(),
            session_id,
            pattern,
            options,
        )
        .await
        .map_err(|error| store_error(format!("Failed to grep files: {error}")))
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
        session_id: Option<Uuid>,
        server_prefix: &str,
    ) -> Result<McpServerInfo> {
        let mut runtime_agent_id = None;
        if let Some(session_id) = session_id
            && let Some(session) = self.get_stored_session(org_id, session_id).await?
            && let Some(harness) = self
                .get_harness_impl(org_id, session.harness_id.uuid())
                .await?
        {
            runtime_agent_id = session.agent_id;
            let mut agent = match session.agent_id {
                Some(agent_id) => self.get_agent(org_id, agent_id.uuid()).await?,
                None => None,
            };
            if let (Some(agent), Some(version_id)) = (agent.as_mut(), session.agent_version_id)
                && let Some(version_row) = self.db.get_agent_version(org_id, version_id).await?
            {
                let version = crate::domains::agents::queries::row_to_agent_version(version_row);
                *agent = crate::domains::agents::queries::version_to_agent(agent, &version);
            }

            if let Some(resolved) = resolve_scoped_mcp_server_with_capabilities(
                &harness,
                agent.as_ref(),
                &session,
                server_prefix,
                &self.capability_registry,
            ) {
                let secret_bindings =
                    crate::domains::agents::credentials::resolve_runtime_secret_bindings(
                        self.db.as_ref(),
                        self.encryption.as_deref(),
                        org_id,
                        runtime_agent_id,
                        &resolved.name,
                        &resolved.url,
                    )
                    .await
                    .map_err(|e| store_error(format!("Failed to resolve MCP credentials: {e}")))?;
                return Ok(McpServerInfo {
                    id: resolved.id,
                    name: resolved.name,
                    url: resolved.url,
                    api_key: resolved.api_key,
                    headers: resolved.headers,
                    auth_mode: resolved.auth_mode,
                    protocol_mode: resolved.protocol_mode,
                    oauth_provider_id: resolved.oauth_provider_id,
                    secret_bindings,
                });
            }
        }

        let resolved = self
            .mcp_server_service
            .resolve_by_prefix(&Caller::internal(org_id), server_prefix)
            .await
            .map_err(|e| {
                tracing::error!("Failed to resolve MCP server: {}", e);
                store_error(format!("Failed to get MCP server: {}", e))
            })?
            .ok_or_else(|| store_error(format!("MCP server not found: {}", server_prefix)))?;

        let secret_bindings = crate::domains::agents::credentials::resolve_runtime_secret_bindings(
            self.db.as_ref(),
            self.encryption.as_deref(),
            org_id,
            runtime_agent_id,
            &resolved.name,
            &resolved.url,
        )
        .await
        .map_err(|e| store_error(format!("Failed to resolve MCP credentials: {e}")))?;

        Ok(McpServerInfo {
            id: resolved.id,
            name: resolved.name,
            url: resolved.url,
            api_key: resolved.api_key,
            headers: resolved.headers,
            auth_mode: resolved.auth_mode,
            protocol_mode: resolved.protocol_mode,
            oauth_provider_id: resolved.oauth_provider_id,
            secret_bindings,
        })
    }

    // =========================================================================
    // Turn Context (batch operation)
    // =========================================================================

    async fn load_turn_context(&self, org_id: i64, session_id: Uuid) -> Result<TurnContext> {
        // Load the stored record: the platform-only fields (agent version
        // pinning, scoped-MCP wiring) are consumed here, at the loading seam,
        // and only the projected execution view leaves in the TurnContext.
        let session = self
            .get_stored_session(org_id, session_id)
            .await?
            .ok_or_else(|| store_error("Session not found"))?;

        // session.agent_id from get_session() is the internal UUID (raw DB FK).
        // Use get_agent() which queries by internal id.
        let (agent, org_mcp_tool_definitions) = if let Some(agent_id) = session.agent_id {
            let agent_row = self.db.get_agent(org_id, agent_id).await.map_err(|e| {
                tracing::error!("Failed to get agent: {}", e);
                store_error("Failed to get agent")
            })?;

            if let Some(row) = agent_row {
                let capability_rows = self
                    .db
                    .get_agent_capabilities(row.id.uuid())
                    .await
                    .unwrap_or_default();

                let mcp_tools = self
                    .build_mcp_tool_definitions_with_capabilities(org_id, &capability_rows)
                    .await
                    .unwrap_or_default();

                let hydrated_capabilities = self
                    .hydrate_capability_rows(org_id, capability_rows)
                    .await
                    .unwrap_or_default();
                let mut agent = Self::row_to_agent(row, hydrated_capabilities);
                if let Some(version_id) = session.agent_version_id
                    && let Some(version_row) = self
                        .db
                        .get_agent_version(org_id, version_id)
                        .await
                        .map_err(|e| {
                        tracing::error!("Failed to get agent version: {}", e);
                        store_error("Failed to get agent version")
                    })?
                {
                    let version =
                        crate::domains::agents::queries::row_to_agent_version(version_row);
                    agent = crate::domains::agents::queries::version_to_agent(&agent, &version);
                }

                (Some(agent), mcp_tools)
            } else {
                (None, vec![])
            }
        } else {
            (None, vec![])
        };

        // Load harness
        let harness = self
            .get_harness_impl(org_id, session.harness_id.uuid())
            .await?;

        let local_mcp_tool_definitions = if let Some(ref harness) = harness {
            let effective = merge_effective_scoped_mcp_servers_with_capabilities(
                harness,
                agent.as_ref(),
                &session,
                &self.capability_registry,
            );

            if let Err(error) = validate_scoped_mcp_servers(&effective) {
                tracing::warn!(error = %error, "Invalid scoped MCP server config, skipping");
                vec![]
            } else {
                let egress = self.egress_service.clone().unwrap_or_else(|| {
                    Arc::new(everruns_host::DirectEgressService::for_runtime_traffic_from_env())
                });
                build_scoped_mcp_tool_definitions(
                    &effective,
                    Some(session.id),
                    self.connection_resolver.as_ref(),
                    egress.as_ref(),
                )
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(error = %error, "Failed to build scoped MCP tool definitions");
                    vec![]
                })
            }
        } else {
            vec![]
        };

        let mut mcp_tool_definitions = org_mcp_tool_definitions;
        mcp_tool_definitions.extend(local_mcp_tool_definitions);
        let binding_metadata = crate::domains::agents::credentials::list_secret_binding_metadata(
            self.db.as_ref(),
            org_id,
            session.agent_id,
        )
        .await
        .map_err(|e| store_error(format!("Failed to load MCP credential metadata: {e}")))?;
        everruns_core::apply_mcp_secret_binding_schemas(
            &mut mcp_tool_definitions,
            &binding_metadata,
        );

        // Load messages through the same capability-aware windowing used by
        // reason atoms. Long sessions should fetch a bounded prompt candidate
        // set (for example infinity_context's head+tail window), not the whole
        // event history.
        let messages = self
            .load_turn_messages(&session, agent.as_ref(), harness.as_ref())
            .await?;

        // Load model (session > agent > harness > default)
        let model = if let Some(model_id) = session.model_id {
            self.get_model_spec(org_id, model_id.uuid()).await?
        } else if let Some(ref a) = agent {
            if let Some(model_id) = a.default_model_id {
                self.get_model_spec(org_id, model_id.uuid()).await?
            } else if let Some(ref h) = harness {
                if let Some(model_id) = h.default_model_id {
                    self.get_model_spec(org_id, model_id.uuid()).await?
                } else {
                    self.get_default_model_spec(org_id).await?
                }
            } else {
                self.get_default_model_spec(org_id).await?
            }
        } else if let Some(ref h) = harness {
            if let Some(model_id) = h.default_model_id {
                self.get_model_spec(org_id, model_id.uuid()).await?
            } else {
                self.get_default_model_spec(org_id).await?
            }
        } else {
            self.get_default_model_spec(org_id).await?
        };

        Ok(TurnContext {
            agent,
            session: session.execution_session(),
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
        self.driver_registry.clone()
    }

    fn sqldb_store(
        &self,
    ) -> std::sync::Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore> {
        self.sqldb_store.clone()
    }

    fn sandbox_checkpoint_store(
        &self,
    ) -> Option<Arc<dyn everruns_platform::sandbox_checkpoint::SandboxCheckpointStore>> {
        self.db.pool().map(|pool| {
            Arc::new(crate::storage::PgSandboxCheckpointStore::new(pool.clone()))
                as Arc<dyn everruns_platform::sandbox_checkpoint::SandboxCheckpointStore>
        })
    }

    fn compaction_checkpoint_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::CompactionCheckpointStore>> {
        if let Some(encryption) = self.encryption.clone() {
            return Some(Arc::new(
                crate::storage::DbCompactionCheckpointStore::with_proactive_attempt_tracker(
                    self.db.clone(),
                    encryption,
                    self.proactive_compaction_attempts.clone(),
                ),
            ));
        }
        self.db.is_dev_mode().then(|| {
            self.in_memory_compaction_checkpoint_store.clone()
                as Arc<dyn everruns_core::CompactionCheckpointStore>
        })
    }

    fn storage_store(&self) -> Arc<dyn everruns_core::session_services::SessionStorageStore> {
        self.storage_store
            .clone()
            .expect("DirectWorkerAdapters: storage_store not set (call with_storage_store)")
    }

    fn knowledge_store(&self) -> Option<Arc<dyn everruns_platform::KnowledgeStore>> {
        Some(Arc::new(
            crate::knowledge_store::StorageBackendKnowledgeStore::new(self.db.clone()),
        ))
    }

    fn image_artifact_store(&self, org_id: i64) -> Arc<dyn ImageArtifactStore> {
        Arc::new(DirectImageArtifactStore {
            db: self.db.clone(),
            org_id,
        })
    }

    fn provider_credential_store(&self, org_id: i64) -> Arc<dyn ProviderCredentialStore> {
        Arc::new(DirectProviderCredentialStore {
            provider_resolver: self.provider_resolver.clone(),
            org_id,
        })
    }

    fn utility_llm_service(&self) -> Option<Arc<dyn UtilityLlmService>> {
        self.utility_llm_service.clone()
    }

    fn egress_service(&self) -> Option<Arc<dyn EgressService>> {
        self.egress_service.clone()
    }

    fn connection_resolver(
        &self,
    ) -> Arc<dyn everruns_core::connection_services::UserConnectionResolver> {
        self.connection_resolver.clone().expect(
            "DirectWorkerAdapters: connection_resolver not set (call with_connection_resolver)",
        )
    }

    fn leased_resource_store(
        &self,
    ) -> Arc<dyn everruns_core::session_services::LeasedResourceStore> {
        let mut store = crate::storage::DbLeasedResourceStore::new(self.db.clone());
        if let Some(registry) = self.session_resource_registry() {
            store = store.with_registry(registry);
        }
        Arc::new(store)
    }

    fn session_resource_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_services::SessionResourceRegistry>> {
        Some(Arc::new(crate::storage::DbSessionResourceRegistry::new(
            self.db.clone(),
        )))
    }

    fn session_task_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_task::SessionTaskRegistry>> {
        let waker = Arc::new(DirectSessionTaskWaker {
            db: self.db.clone(),
            event_service: self.event_service.clone(),
            runner: self.runner.clone(),
        });
        let mut registry = crate::storage::DbSessionTaskRegistry::new(self.db.clone())
            .with_event_emitter(self.event_service.clone())
            .with_waker(waker);
        if let Some(egress) = &self.egress_service {
            let notifier = Arc::new(DirectTaskWebhookNotifier {
                db: self.db.clone(),
                egress_service: egress.clone(),
            });
            registry = registry.with_transition_observer(notifier);
        }
        Some(Arc::new(registry))
    }

    fn schedule_store(
        &self,
        org_id: i64,
    ) -> Arc<dyn everruns_core::session_services::SessionScheduleStore> {
        Arc::new(crate::storage::DbSessionScheduleStore::new(
            self.db.clone(),
            org_id,
        ))
    }

    fn budget_checker(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn BudgetChecker>> {
        self.budget_service.as_ref().map(|budget_service| {
            Arc::new(DirectBudgetChecker {
                budget_service: budget_service.clone(),
                org_id,
                agent_id: agent_id.map(|id| id.to_string()),
            }) as Arc<dyn BudgetChecker>
        })
    }

    fn payment_authority(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn PaymentAuthority>> {
        Some(Arc::new(
            crate::domains::payments::ServerPaymentAuthority::new(
                self.db.clone(),
                self.encryption.clone(),
                org_id,
                agent_id,
            ),
        ))
    }

    fn session_creation_authority(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<dyn SessionCreationAuthority>> {
        Some(Arc::new(DirectPlatformStore::new(
            org_id,
            session_id,
            self.db.clone(),
            DirectPlatformStoreDeps {
                event_service: self.event_service.clone(),
                runner: self.runner.clone(),
                capability_registry: self.capability_registry.clone(),
                connector_registry: self.connector_registry.clone(),
                encryption: self.encryption.clone(),
                workflow_store: self.workflow_store.clone(),
                permission_resolver: self.permission_resolver.clone(),
                egress_service: self.egress_service.clone(),
            },
        )))
    }

    fn outbound_tool_rate_limiter(
        &self,
        _org_id: i64,
    ) -> Option<Arc<dyn everruns_core::tool_execution::OutboundToolRateLimiter>> {
        self.org_rate_limiter
            .as_ref()
            .map(|l| l.clone() as Arc<dyn everruns_core::tool_execution::OutboundToolRateLimiter>)
    }

    fn durable_tool_result_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::durability::DurableToolResultStore>> {
        self.db.pool().map(|pool| {
            Arc::new(crate::storage::PgDurableToolResultStore::new(pool.clone()))
                as Arc<dyn everruns_core::durability::DurableToolResultStore>
        })
    }

    fn subagent_spawn_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::delegation_services::SubagentSpawnStore>> {
        self.db.pool().map(|pool| {
            Arc::new(crate::storage::PgSubagentSpawnStore::new(pool.clone()))
                as Arc<dyn everruns_core::delegation_services::SubagentSpawnStore>
        })
    }

    async fn invoke_scheduled_app_channel(
        &self,
        org_id: i64,
        app_id: &str,
        channel_id: &str,
    ) -> Result<serde_json::Value> {
        let runner = self
            .runner
            .clone()
            .ok_or_else(|| store_error("Agent runner not configured"))?;
        let session_service = if let Some(registry) = self.virtual_registry.clone() {
            SessionService::with_registry(self.db.clone(), self.capability_registry.clone())
                .with_virtual_registry(registry)
        } else {
            SessionService::with_registry(self.db.clone(), self.capability_registry.clone())
        };
        let message_service = MessageService::new(
            self.db.clone(),
            runner,
            false,
            self.event_service.event_delivery().clone(),
        );

        let result = crate::domains::apps::invoke_scheduled_app_channel(
            &self.db,
            self.encryption.as_ref(),
            &session_service,
            &message_service,
            org_id,
            app_id,
            channel_id,
        )
        .await
        .map_err(|error| store_error(format!("Failed to invoke scheduled app channel: {error}")))?;

        Ok(serde_json::json!({
            "session_id": result.session_id.to_string(),
            "created_session": result.created_session,
        }))
    }

    async fn invoke_agent_trigger(
        &self,
        org_id: i64,
        agent_id: &str,
        trigger_id: &str,
    ) -> Result<serde_json::Value> {
        let runner = self
            .runner
            .clone()
            .ok_or_else(|| store_error("Agent runner not configured"))?;
        let session_service = if let Some(registry) = self.virtual_registry.clone() {
            SessionService::with_registry(self.db.clone(), self.capability_registry.clone())
                .with_virtual_registry(registry)
        } else {
            SessionService::with_registry(self.db.clone(), self.capability_registry.clone())
        };
        let message_service = MessageService::new(
            self.db.clone(),
            runner,
            false,
            self.event_service.event_delivery().clone(),
        );

        let result = crate::domains::agent_triggers::invoke_agent_trigger(
            &self.db,
            &session_service,
            &message_service,
            org_id,
            agent_id,
            trigger_id,
        )
        .await
        .map_err(|error| store_error(format!("Failed to invoke agent trigger: {error}")))?;

        Ok(serde_json::json!({
            "session_id": result.session_id.to_string(),
            "created_session": result.created_session,
        }))
    }

    fn platform_store(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Arc<dyn everruns_platform::PlatformStore> {
        Arc::new(DirectPlatformStore::new(
            org_id,
            session_id,
            self.db.clone(),
            DirectPlatformStoreDeps {
                event_service: self.event_service.clone(),
                runner: self.runner.clone(),
                capability_registry: self.capability_registry.clone(),
                connector_registry: self.connector_registry.clone(),
                encryption: self.encryption.clone(),
                workflow_store: self.workflow_store.clone(),
                permission_resolver: self.permission_resolver.clone(),
                egress_service: self.egress_service.clone(),
            },
        ))
    }

    fn knowledge_index_search(
        &self,
        _org_id: i64,
    ) -> Option<Arc<dyn everruns_platform::vector_store::KnowledgeIndexSearch>> {
        // The service is org-scoped per call via the `org_id` passed to
        // `search`, so a single instance is reused across orgs.
        let vector_store = self.vector_store.clone()?;
        Some(Arc::new(
            crate::domains::knowledge_indexes::KnowledgeIndexSearchService::new(
                self.db.clone(),
                self.provider_resolver.clone(),
                Arc::new(self.driver_registry.clone()),
                vector_store,
            ),
        ))
    }

    async fn claim_due_leased_resources(
        &self,
        limit: u32,
        stale_after_seconds: u32,
    ) -> Result<Vec<everruns_core::LeasedResource>> {
        let rows = self
            .db
            .claim_due_leased_resources(limit as i32, stale_after_seconds as i32)
            .await
            .map_err(|e| store_error(format!("Failed to claim leased resources: {e}")))?;
        rows.iter()
            .map(crate::storage::leased_resource_row_to_domain)
            .collect()
    }

    async fn mark_leased_resource_released(
        &self,
        resource_id: everruns_provider::typed_id::LeasedResourceId,
        expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        Ok(self
            .db
            .mark_leased_resource_released(resource_id, expected_cleanup_started_at)
            .await
            .map_err(|e| store_error(format!("Failed to mark leased resource released: {e}")))?
            .is_some())
    }

    async fn mark_leased_resource_cleanup_failed(
        &self,
        resource_id: everruns_provider::typed_id::LeasedResourceId,
        expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
        retry_after_seconds: u32,
        error: &str,
    ) -> Result<bool> {
        Ok(self
            .db
            .mark_leased_resource_cleanup_failed(
                resource_id,
                expected_cleanup_started_at,
                retry_after_seconds as i32,
                error,
            )
            .await
            .map_err(|e| {
                store_error(format!(
                    "Failed to mark leased resource cleanup failed: {e}"
                ))
            })?
            .is_some())
    }

    async fn list_orphaned_session_task_ids(
        &self,
        stale_after: chrono::Duration,
        limit: i64,
    ) -> Result<Vec<(everruns_provider::typed_id::SessionId, String)>> {
        self.db
            .list_orphaned_session_task_ids(stale_after, limit)
            .await
            .map_err(|e| store_error(format!("Failed to list orphaned session tasks: {e}")))
    }

    fn reaper_session_task_registry(
        &self,
    ) -> Arc<dyn everruns_core::session_task::SessionTaskRegistry> {
        // Attach waker so reaped tasks can wake sessions per wake_policy.
        let waker = Arc::new(DirectSessionTaskWaker {
            db: self.db.clone(),
            event_service: self.event_service.clone(),
            runner: self.runner.clone(),
        });
        let mut registry = crate::storage::DbSessionTaskRegistry::new(self.db.clone())
            .with_event_emitter(self.event_service.clone())
            .with_waker(waker);
        if let Some(egress) = &self.egress_service {
            let notifier = Arc::new(DirectTaskWebhookNotifier {
                db: self.db.clone(),
                egress_service: egress.clone(),
            });
            registry = registry.with_transition_observer(notifier);
        }
        Arc::new(registry)
    }

    async fn prune_terminal_session_tasks(
        &self,
        ttl: chrono::Duration,
        limit: i64,
    ) -> Result<usize> {
        self.db
            .prune_terminal_session_tasks_with_artifacts(ttl, limit)
            .await
            .map_err(|e| store_error(format!("Failed to prune terminal session tasks: {e}")))
    }
}

impl DirectWorkerAdapters {
    /// Get a harness by ID (direct DB access)
    async fn get_harness_impl(&self, org_id: i64, harness_id: Uuid) -> Result<Option<Harness>> {
        let mut visited = std::collections::HashSet::new();
        let mut chain = Vec::new();
        let mut cursor = Some(HarnessId::from_uuid(harness_id));

        while let Some(current_harness_id) = cursor {
            if !visited.insert(current_harness_id) {
                return Err(store_error("Harness inheritance cycle detected"));
            }

            let row = self
                .db
                .get_harness(org_id, current_harness_id)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to get harness: {}", e);
                    store_error("Failed to get harness")
                })?;
            let Some(row) = row else {
                if chain.is_empty() {
                    return Ok(None);
                }
                return Err(store_error("Parent harness not found"));
            };

            let capabilities = self
                .db
                .get_harness_capabilities(current_harness_id.uuid())
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|cap| AgentCapabilityConfig::with_config(cap.capability_id, cap.config))
                .collect();
            let capabilities =
                crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
                    &self.db,
                    org_id,
                    capabilities,
                )
                .await
                .unwrap_or_default();

            cursor = row.parent_harness_id;
            chain.push(Harness {
                id: row.id,
                name: row.name,
                display_name: row.display_name,
                description: row.description,
                system_prompt: row.system_prompt,
                parent_harness_id: row.parent_harness_id,
                default_model_id: row.default_model_id,
                tags: row.tags,
                capabilities,
                mcp_servers: serde_json::from_value(row.mcp_servers).unwrap_or_default(),
                initial_files: serde_json::from_value(row.initial_files).unwrap_or_default(),
                network_access: row
                    .network_access
                    .and_then(|v| serde_json::from_value(v).ok()),
                // No per-harness config column (EVE-598).
                parallel_tool_calls: None,
                embedder_metadata: serde_json::from_value(row.embedder_metadata)
                    .unwrap_or_default(),
                is_built_in: row.is_built_in,
                status: match row.status.as_str() {
                    "active" => HarnessStatus::Active,
                    "archived" => HarnessStatus::Archived,
                    "deleted" => HarnessStatus::Deleted,
                    _ => HarnessStatus::Active,
                },
                created_at: row.created_at,
                updated_at: row.updated_at,
                archived_at: row.archived_at,
                deleted_at: row.deleted_at,
            });
        }

        let Some(mut effective) = chain.pop() else {
            return Ok(None);
        };
        while let Some(layer) = chain.pop() {
            effective = merge_harness(&effective, &layer);
        }

        Ok(Some(effective))
    }

    /// Build an Agent from a DB row and pre-loaded capability rows.
    ///
    /// Used by both `get_agent` (standalone) and `load_turn_context`
    /// (where capabilities are loaded once and shared across consumers).
    /// Convert an AgentRow + capability rows into an Agent domain object.
    fn row_to_agent(r: AgentRow, capabilities: Vec<AgentCapabilityConfig>) -> Agent {
        Agent {
            public_id: r
                .public_id
                .parse()
                .unwrap_or_else(|_| AgentId::from_uuid(r.id.uuid())),
            internal_id: r.id.uuid(),
            name: r.name,
            display_name: r.display_name,
            description: r.description,
            system_prompt: r.system_prompt,
            default_model_id: r.default_model_id,
            harness_id: r.harness_id,
            default_version_id: r.default_version_id,
            forked_from_agent_id: r.forked_from_agent_id,
            forked_from_version_id: r.forked_from_version_id,
            root_agent_id: r.root_agent_id,
            tags: r.tags,
            capabilities,
            initial_files: serde_json::from_value(r.initial_files).unwrap_or_default(),
            mcp_servers: serde_json::from_value(r.mcp_servers).unwrap_or_default(),
            network_access: r
                .network_access
                .and_then(|v| serde_json::from_value(v).ok()),
            max_iterations: max_iterations::from_db(r.max_iterations),
            parallel_tool_calls: r.parallel_tool_calls,
            tools: serde_json::from_value(r.tools).unwrap_or_default(),
            status: match r.status.as_str() {
                "active" => AgentStatus::Active,
                "archived" => AgentStatus::Archived,
                "deleted" => AgentStatus::Deleted,
                _ => AgentStatus::Active,
            },
            created_at: r.created_at,
            updated_at: r.updated_at,
            archived_at: r.archived_at,
            deleted_at: r.deleted_at,
            usage: None,
        }
    }

    async fn hydrate_capability_rows(
        &self,
        org_id: i64,
        capability_rows: Vec<AgentCapabilityRow>,
    ) -> Result<Vec<AgentCapabilityConfig>> {
        let capabilities = capability_rows
            .into_iter()
            .map(|c| AgentCapabilityConfig::with_config(c.capability_id, c.config))
            .collect();
        crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
            &self.db,
            org_id,
            capabilities,
        )
        .await
        .map_err(|error| store_error(format!("Failed to hydrate capabilities: {error}")))
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
        use everruns_core::mcp_server::mcp_tool_name;
        use everruns_mcp::parse_mcp_capability_id;
        use everruns_provider::tool_types::{BuiltinTool, DeferrablePolicy, ToolPolicy};

        let mut mcp_tools = Vec::new();

        for cap_row in capability_rows {
            let cap_id = &cap_row.capability_id;
            let server_id = match parse_mcp_capability_id(cap_id) {
                Some(id) => id,
                None => continue,
            };

            let tools = match self
                .mcp_server_service
                .get_tools(&Caller::internal(org_id), server_id, false)
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

            let server_name = match self
                .mcp_server_service
                .get(&Caller::internal(org_id), server_id)
                .await
            {
                Ok(Some(s)) => s.name,
                _ => {
                    tracing::warn!(server_id = %server_id, "MCP server not found, skipping");
                    continue;
                }
            };

            if !everruns_core::mcp_server::is_valid_mcp_server_name(&server_name) {
                tracing::warn!(server_id = %server_id, "MCP tools omitted: ambiguous server prefix");
                continue;
            }
            for tool in tools {
                let prefixed_name = mcp_tool_name(&server_name, &tool.name);
                let description = tool
                    .description
                    .unwrap_or_else(|| format!("Tool from MCP server: {}", server_name));

                mcp_tools.push(
                    ToolDefinition::Builtin(BuiltinTool {
                        name: prefixed_name,
                        display_name: None,
                        description,
                        parameters: tool.input_schema,
                        policy: ToolPolicy::Auto,
                        category: None,
                        deferrable: DeferrablePolicy::default(),
                        hints: everruns_provider::tool_types::ToolHints::default()
                            .with_open_world(true),
                        full_parameters: None,
                    })
                    .with_capability_attribution(cap_id.clone(), Some(server_name.clone())),
                );
            }
        }

        Ok(mcp_tools)
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

fn string_to_provider_type(s: &str) -> DriverId {
    // FromStr is infallible: unknown ids become External providers, preserving
    // the id so embedder-defined providers resolve correctly.
    s.to_lowercase().parse().unwrap_or_else(|_| unreachable!())
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
                phase: None,
                phase_source: None,
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
// DirectSessionTaskWaker — inject wake messages into sessions from the worker
// =============================================================================

/// Wake the owning session by injecting a synthetic user message via the same
/// path as `platform_send_message` in the gRPC service. Uses an internal
/// caller so the message is created without a user context.
struct DirectSessionTaskWaker {
    db: Arc<crate::storage::StorageBackend>,
    event_service: Arc<crate::services::EventService>,
    runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
}

#[async_trait::async_trait]
impl crate::storage::session_task_store::SessionTaskWaker for DirectSessionTaskWaker {
    async fn wake(
        &self,
        session_id: everruns_provider::typed_id::SessionId,
        text: &str,
    ) -> anyhow::Result<()> {
        // Fetch session without org-scope to get harness_id and org_id.
        let session = self
            .db
            .get_session_unscoped(session_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to look up session for wake: {e}"))?;
        let Some(session) = session else {
            return Ok(());
        };
        let Some(harness_id) = session.harness_id else {
            tracing::debug!(
                session_id = %session_id,
                "SessionTaskWaker: session has no harness_id; skipping wake"
            );
            return Ok(());
        };

        let message_id = everruns_provider::typed_id::MessageId::new();
        let now = chrono::Utc::now();
        let core_message = everruns_core::Message {
            id: message_id,
            role: everruns_core::MessageRole::User,
            content: vec![everruns_core::ContentPart::text(text)],
            phase: None,
            phase_source: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: now,
        };

        self.event_service
            .emit(everruns_core::EventRequest::new(
                session_id,
                everruns_core::events::EventContext::empty(),
                everruns_core::events::InputMessageData::new(core_message),
            ))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to emit wake message event: {e}"))?;

        if let Some(runner) = &self.runner {
            let runner = runner.clone();
            let agent_id = session.agent_id;
            let org_id = session.org_id;
            tokio::spawn(async move {
                if let Err(e) = runner
                    .start_run(org_id, session_id, harness_id, agent_id, message_id, None)
                    .await
                {
                    tracing::warn!(
                        session_id = %session_id,
                        "SessionTaskWaker: failed to start turn workflow: {e}"
                    );
                }
            });
        }

        Ok(())
    }
}

// =============================================================================
// DirectTaskWebhookNotifier — fire outbound HTTP webhooks on terminal tasks
// =============================================================================

struct DirectTaskWebhookNotifier {
    db: Arc<crate::storage::StorageBackend>,
    egress_service: Arc<dyn EgressService>,
}

/// One resolved delivery target for a webhook notification. `label` is only for
/// logging (a public id or a spec-embedded marker) and never affects delivery.
struct WebhookTarget {
    label: String,
    url: String,
    secret: Option<String>,
}

/// Parse spec-embedded push configs (EVE-682) that match `event`.
///
/// Spawn-time configs live in `task.spec["push_configs"]` as an array of
/// `{ url, secret?, event_filter? }`. `event_filter` defaults to
/// `["terminal"]`, matching org-webhook behavior. URLs were SSRF-validated at
/// spawn time and delivery pins DNS, so no re-validation is needed here.
fn spec_push_config_targets(
    spec: &serde_json::Value,
    event: crate::storage::session_task_store::TaskTransition,
) -> Vec<WebhookTarget> {
    let Some(entries) = spec.get("push_configs").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    for entry in entries {
        let Some(url) = entry.get("url").and_then(|v| v.as_str()) else {
            continue;
        };
        let matches = match entry.get("event_filter").and_then(|v| v.as_array()) {
            Some(filters) => filters
                .iter()
                .filter_map(|f| f.as_str())
                .any(|f| f == event.filter_value()),
            // Absent filter defaults to terminal-only.
            None => event.filter_value() == "terminal",
        };
        if !matches {
            continue;
        }
        targets.push(WebhookTarget {
            label: "spec".to_string(),
            url: url.to_string(),
            secret: entry
                .get("secret")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        });
    }
    targets
}

#[async_trait::async_trait]
impl crate::storage::session_task_store::TaskTransitionObserver for DirectTaskWebhookNotifier {
    async fn on_transition(
        &self,
        task: &everruns_core::session_task::SessionTask,
        event: crate::storage::session_task_store::TaskTransition,
    ) -> anyhow::Result<()> {
        use crate::storage::session_task_store::TaskTransition;

        // Resolve org_id from session (unscoped lookup — no harness required).
        // Server-internal dispatch only: this is never a user-facing read, so
        // the unscoped lookup does not widen any caller's tenant scope.
        let session = self
            .db
            .get_session_unscoped(task.session_id)
            .await
            .map_err(|e| anyhow::anyhow!("webhook notifier: session lookup failed: {e}"))?;
        let Some(session) = session else {
            return Ok(());
        };

        let mut targets: Vec<WebhookTarget> = Vec::new();

        // Org webhooks are terminal-only and unchanged (EVE-579): they never
        // fire on non-terminal events.
        if event == TaskTransition::Terminal {
            let webhooks = self
                .db
                .list_enabled_org_task_webhooks(session.org_id)
                .await
                .map_err(|e| anyhow::anyhow!("webhook notifier: webhook lookup failed: {e}"))?;
            targets.extend(webhooks.into_iter().map(|w| WebhookTarget {
                label: w.public_id,
                url: w.url,
                secret: w.secret,
            }));
        }

        // Per-task push configs (EVE-682): DB-persisted (endpoint-created) plus
        // spec-embedded (spawn-time) configs share this delivery path. Both are
        // filtered by whether their event_filter includes this event.
        let configs = self
            .db
            .list_task_push_configs(task.session_id, &task.id)
            .await
            .map_err(|e| anyhow::anyhow!("webhook notifier: push-config lookup failed: {e}"))?;
        targets.extend(
            configs
                .into_iter()
                .filter(|c| c.event_filter.iter().any(|f| f == event.filter_value()))
                .map(|c| WebhookTarget {
                    label: c.public_id,
                    url: c.url,
                    secret: c.secret,
                }),
        );
        targets.extend(spec_push_config_targets(&task.spec, event));

        if targets.is_empty() {
            return Ok(());
        }

        let payload = serde_json::json!({
            "event": event.event_name(),
            "task": {
                "id": task.id,
                "display_name": task.display_name,
                "kind": task.kind,
                "state": task.state.to_string(),
                "session_id": task.session_id,
                "summary": task.summary,
                "result_path": task.result_path,
            }
        });
        let body = serde_json::to_vec(&payload)?;

        for target in targets {
            let req = build_task_webhook_request(&target.url, &body, target.secret.as_deref());

            if let Err(e) = self.egress_service.send(req).await {
                tracing::warn!(
                    webhook = %target.label,
                    url = %target.url,
                    task_id = %task.id,
                    event = ?event,
                    "Task webhook delivery failed (best-effort): {e}"
                );
            }
        }

        Ok(())
    }
}

/// Build the egress request for a task-webhook delivery.
///
/// TM-API-020 (EVE-625): task-webhook URLs are org-configured and SSRF-validated
/// only at create time. Delivery must pin DNS to the IPs resolved during the
/// request-time SSRF check, otherwise a DNS rebind between create and delivery
/// can point the previously-validated host at a private IP / cloud metadata
/// endpoint. `require_dns_pinning()` closes that rebinding window.
fn build_task_webhook_request(url: &str, body: &[u8], secret: Option<&str>) -> EgressRequest {
    let mut req = EgressRequest::new("POST", url, EgressRequestKind::Integration)
        .header("Content-Type", "application/json")
        .body(body.to_vec())
        .timeout_ms(10_000)
        .require_dns_pinning();

    if let Some(secret) = secret {
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
        mac.update(body);
        let sig = hex::encode(mac.finalize().into_bytes());
        req = req.header("X-Everruns-Signature", format!("sha256={sig}"));
    }

    req
}

#[cfg(test)]
mod task_webhook_request_tests {
    use super::build_task_webhook_request;

    #[test]
    fn task_webhook_request_pins_dns_and_signs() {
        // EVE-625: delivery must pin DNS to defeat create-time->delivery DNS
        // rebinding SSRF.
        let req = build_task_webhook_request(
            "https://hooks.example.com/notify",
            br#"{"event":"task.terminal"}"#,
            Some("s3cret"),
        );
        assert!(
            req.dns_pinning_required,
            "task webhook delivery must require DNS pinning"
        );
        assert!(
            req.headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("X-Everruns-Signature")
                    && v.starts_with("sha256=")),
            "signed webhook must carry an HMAC signature header"
        );

        // Unsigned webhook still pins DNS and omits the signature header.
        let unsigned = build_task_webhook_request("https://hooks.example.com/notify", b"{}", None);
        assert!(unsigned.dns_pinning_required);
        assert!(
            !unsigned
                .headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("X-Everruns-Signature"))
        );
    }

    use super::spec_push_config_targets;
    use crate::storage::session_task_store::TaskTransition;

    #[test]
    fn spec_push_configs_filter_by_event() {
        // EVE-682: spawn-time configs embedded in the task spec deliver only for
        // events their event_filter includes; an absent filter defaults to
        // terminal-only, matching org webhook behavior.
        let spec = serde_json::json!({
            "push_configs": [
                { "url": "https://a.example.com", "secret": "s", "event_filter": ["message"] },
                { "url": "https://b.example.com", "event_filter": ["terminal", "awaiting_input"] },
                { "url": "https://c.example.com" },
            ]
        });

        let terminal = spec_push_config_targets(&spec, TaskTransition::Terminal);
        let terminal_urls: Vec<&str> = terminal.iter().map(|t| t.url.as_str()).collect();
        assert_eq!(
            terminal_urls,
            vec!["https://b.example.com", "https://c.example.com"],
            "terminal must include the explicit terminal filter and the default"
        );

        let message = spec_push_config_targets(&spec, TaskTransition::Message);
        let message_urls: Vec<&str> = message.iter().map(|t| t.url.as_str()).collect();
        assert_eq!(message_urls, vec!["https://a.example.com"]);
        assert_eq!(
            message[0].secret.as_deref(),
            Some("s"),
            "secret must be carried through for signing"
        );

        let awaiting = spec_push_config_targets(&spec, TaskTransition::AwaitingInput);
        let awaiting_urls: Vec<&str> = awaiting.iter().map(|t| t.url.as_str()).collect();
        assert_eq!(awaiting_urls, vec!["https://b.example.com"]);

        // No push_configs key → no targets.
        assert!(
            spec_push_config_targets(&serde_json::json!({}), TaskTransition::Terminal).is_empty()
        );
    }
}

// =============================================================================
// DirectPlatformStore - PlatformStore implementation for in-process worker
// =============================================================================

/// Direct PlatformStore backed by StorageBackend + EventService + AgentRunner.
// THREAT[TM-AGENT-017]: All ops org-scoped via org_id field
struct DirectPlatformStoreDeps {
    event_service: Arc<EventService>,
    runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
    capability_registry: CapabilityRegistry,
    connector_registry: everruns_platform::connector::ConnectorRegistry,
    encryption: Option<Arc<EncryptionService>>,
    workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    permission_resolver: Arc<dyn PermissionResolver>,
    egress_service: Option<Arc<dyn EgressService>>,
}

pub struct DirectPlatformStore {
    org_id: i64,
    session_id: SessionId,
    db: Arc<StorageBackend>,
    runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
    capability_service: Arc<crate::services::CapabilityService>,
    session_service: Arc<SessionService>,
    message_service: Option<Arc<MessageService>>,
    event_service: Arc<EventService>,
    encryption: Option<Arc<EncryptionService>>,
    workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    permission_resolver: Arc<dyn PermissionResolver>,
    connector_registry: everruns_platform::connector::ConnectorRegistry,
    resolved_caller: Arc<OnceCell<Caller>>,
}

impl DirectPlatformStore {
    fn new(
        org_id: i64,
        session_id: SessionId,
        db: Arc<StorageBackend>,
        deps: DirectPlatformStoreDeps,
    ) -> Self {
        let mut capability_service = crate::services::CapabilityService::with_registry(
            db.clone(),
            deps.encryption.clone(),
            deps.capability_registry.clone(),
        );
        if let Some(egress) = &deps.egress_service {
            capability_service = capability_service.with_mcp_egress_service(egress.clone());
        }
        let capability_service = Arc::new(capability_service);
        let session_service = Arc::new(SessionService::with_registry(
            db.clone(),
            deps.capability_registry.clone(),
        ));
        let message_service = deps.runner.as_ref().map(|runner| {
            Arc::new(MessageService::new(
                db.clone(),
                runner.clone(),
                false,
                deps.event_service.event_delivery().clone(),
            ))
        });
        Self {
            org_id,
            session_id,
            db,
            runner: deps.runner,
            capability_service,
            session_service,
            message_service,
            event_service: deps.event_service,
            encryption: deps.encryption,
            workflow_store: deps.workflow_store,
            permission_resolver: deps.permission_resolver,
            connector_registry: deps.connector_registry,
            resolved_caller: Arc::new(OnceCell::new()),
        }
    }

    fn base_url_from_env() -> String {
        everruns_core::config::env_string_any(
            &["PUBLIC_APP_URL", "FRONTEND_URL", "APP_URL"],
            "http://localhost:9300",
        )
    }

    fn capability_refs_to_configs(capabilities: &[String]) -> Vec<serde_json::Value> {
        capabilities
            .iter()
            .map(|capability| serde_json::json!({ "ref": capability, "config": {} }))
            .collect()
    }

    async fn resolve_caller(&self) -> everruns_provider::error::Result<Caller> {
        self.resolved_caller
            .get_or_try_init(|| async {
                // Platform tools must execute as the session owner. Do not replace
                // this with Caller::internal to "fix" permission failures.
                let session = self
                    .db
                    .get_session(self.org_id, self.session_id)
                    .await
                    .map_err(|e| store_error(format!("Failed to load session owner: {e}")))?
                    .ok_or_else(|| {
                        store_error("Session not found for platform tool authorization")
                    })?;

                let user_id = session.resolved_owner_user_id.ok_or_else(|| {
                    store_error(
                        "Platform tool authorization requires a user-owned session with a resolved owner",
                    )
                })?;

                crate::auth::caller_resolution::caller_for_user(&self.db, self.org_id, user_id)
                    .await
                    .map_err(|e| {
                        store_error(format!("Failed to resolve platform tool caller: {e}"))
                    })
            })
            .await
            .cloned()
    }

    async fn command_ctx(&self) -> everruns_provider::error::Result<crate::domains::common::Ctx> {
        let caller = self.resolve_caller().await?;
        let feature_flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            self.org_id,
            &everruns_platform::FeatureFlags::current(),
        )
        .await
        .map_err(|error| {
            store_error(format!("Failed to resolve platform feature flags: {error}"))
        })?;
        let mut ctx = crate::domains::common::Ctx::new(
            caller,
            self.db.clone(),
            self.capability_service.clone(),
            self.encryption.clone(),
            self.permission_resolver.clone(),
        )
        .with_feature_flags(feature_flags)
        .with_connector_registry(self.connector_registry.clone())
        .with_workflow_store(self.workflow_store.clone())
        .with_session_service(self.session_service.clone())
        // wait_for_idle probes terminal turn events through list_events; without
        // the event service every subagent wait fails in the in-process adapter.
        .with_event_service(self.event_service.clone());

        if let Some(message_service) = &self.message_service {
            ctx = ctx.with_message_service(message_service.clone());
        }
        if let Some(runner) = &self.runner {
            ctx = ctx.with_runner(runner.clone());
        }

        Ok(ctx)
    }

    async fn execute_domain_command<T>(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> everruns_provider::error::Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let ctx = self.command_ctx().await?;
        let json = crate::domains::common::dispatch(name, params, &ctx)
            .await
            .map_err(|e| store_error(format!("Command {name} failed: {e}")))?;
        serde_json::from_str(&json)
            .map_err(|e| store_error(format!("Failed to decode {name} response: {e}")))
    }

    async fn execute_domain_lookup<T>(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> everruns_provider::error::Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        let ctx = self.command_ctx().await?;
        match crate::domains::common::dispatch(name, params, &ctx).await {
            Ok(json) => serde_json::from_str(&json)
                .map(Some)
                .map_err(|e| store_error(format!("Failed to decode {name} response: {e}"))),
            Err(crate::domains::common::CommandError {
                kind: crate::domains::common::CommandErrorKind::NotFound(_),
                ..
            }) => Ok(None),
            Err(error) => Err(store_error(format!("Command {name} failed: {error}"))),
        }
    }

    async fn invoke_platform_command_surface(
        &self,
        operation: crate::services::platform_command_surface::Operation,
        arguments: serde_json::Value,
    ) -> everruns_provider::error::Result<String> {
        let base_url = Self::base_url_from_env();
        let context = crate::api::mcp_endpoint::catalog::CatalogContext {
            domain_ctx: self.command_ctx().await?,
            link_builder: crate::api::common::UrlBuilder::new(&base_url, &base_url),
        };
        crate::services::platform_command_surface::invoke(operation, &arguments, context)
            .await
            .map_err(AgentLoopError::tool)
    }

    async fn latest_terminal_turn_status(&self, session_id: SessionId) -> Result<Option<String>> {
        const TURN_COMPLETED: &str = "turn.completed";
        const TURN_FAILED: &str = "turn.failed";
        const TURN_CANCELLED: &str = "turn.cancelled";
        const TURN_SEALED: &str = "turn.sealed";

        let response: serde_json::Value = self
            .execute_domain_command(
                "list_events",
                serde_json::json!({
                    "session_id": session_id.to_string(),
                    "types": [TURN_COMPLETED, TURN_FAILED, TURN_CANCELLED, TURN_SEALED],
                    "limit": 1,
                    "order_desc": true,
                }),
            )
            .await?;

        let Some(event_type) = response
            .get("data")
            .and_then(|data| data.as_array())
            .and_then(|events| events.first())
            .and_then(|event| event.get("type"))
            .and_then(|event_type| event_type.as_str())
        else {
            return Ok(None);
        };

        let status = match event_type {
            TURN_COMPLETED => Some("completed"),
            TURN_FAILED => Some("failed"),
            TURN_CANCELLED => Some("cancelled"),
            // A sealed turn is terminal but distinct from a failure: surface it
            // as "sealed" so the parent agent can decide what to do next.
            TURN_SEALED => Some("sealed"),
            _ => None,
        };
        Ok(status.map(str::to_string))
    }
}

#[async_trait]
impl SessionCreationAuthority for DirectPlatformStore {
    async fn authorize_session_creation(
        &self,
        session_id: SessionId,
    ) -> everruns_provider::error::Result<SessionId> {
        if session_id != self.session_id {
            return Err(AgentLoopError::tool(
                "session-creation authority is scoped to the current session",
            ));
        }
        let caller = self.resolve_caller().await?;
        crate::domains::sessions::SESSION_MANAGE
            .evaluate_with(self.permission_resolver.as_ref(), &caller)
            .map_err(|error| AgentLoopError::tool(error.message))?;
        let session = self
            .db
            .get_session(self.org_id, session_id)
            .await
            .map_err(|error| store_error(format!("Failed to load session budget root: {error}")))?
            .ok_or_else(|| store_error("Session not found for session-creation authority"))?;
        Ok(session.root_session_id.unwrap_or(session.id))
    }
}

#[async_trait]
impl everruns_platform::PlatformStore for DirectPlatformStore {
    async fn platform_discover(
        &self,
        arguments: serde_json::Value,
    ) -> everruns_provider::error::Result<String> {
        self.invoke_platform_command_surface(
            crate::services::platform_command_surface::Operation::Discover,
            arguments,
        )
        .await
    }

    async fn platform_query(
        &self,
        arguments: serde_json::Value,
    ) -> everruns_provider::error::Result<String> {
        self.invoke_platform_command_surface(
            crate::services::platform_command_surface::Operation::Query,
            arguments,
        )
        .await
    }

    async fn platform_execute(
        &self,
        arguments: serde_json::Value,
    ) -> everruns_provider::error::Result<String> {
        self.invoke_platform_command_surface(
            crate::services::platform_command_surface::Operation::Execute,
            arguments,
        )
        .await
    }

    // =========================================================================
    // Harness Operations
    // =========================================================================

    async fn list_harnesses(&self) -> everruns_provider::error::Result<Vec<Harness>> {
        self.execute_domain_command("list_harnesses", serde_json::json!({}))
            .await
    }

    async fn get_harness(
        &self,
        id: HarnessId,
    ) -> everruns_provider::error::Result<Option<Harness>> {
        self.execute_domain_lookup("get_harness", serde_json::json!({ "id": id.to_string() }))
            .await
    }

    async fn create_harness(
        &self,
        name: &str,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
        parent_harness_id: Option<HarnessId>,
        capabilities: &[String],
    ) -> everruns_provider::error::Result<Harness> {
        self.execute_domain_command(
            "create_harness",
            serde_json::json!({
                "name": name,
                "display_name": display_name,
                "description": description,
                // Omit when absent so the harness contributes no base prompt.
                "system_prompt": system_prompt,
                "parent_harness_id": parent_harness_id.map(|id| id.to_string()),
                "tags": ["managed"],
                "initial_files": [],
                "mcp_servers": {},
                "capabilities": Self::capability_refs_to_configs(capabilities),
            }),
        )
        .await
    }

    async fn update_harness(
        &self,
        id: HarnessId,
        name: Option<&str>,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
        parent_harness_id: Option<Option<HarnessId>>,
    ) -> everruns_provider::error::Result<Harness> {
        let mut params = serde_json::Map::from_iter([(
            "id".to_string(),
            serde_json::Value::String(id.to_string()),
        )]);
        if let Some(name) = name {
            params.insert(
                "name".to_string(),
                serde_json::Value::String(name.to_string()),
            );
        }
        if let Some(display_name) = display_name {
            params.insert(
                "display_name".to_string(),
                serde_json::Value::String(display_name.to_string()),
            );
        }
        if let Some(description) = description {
            params.insert(
                "description".to_string(),
                serde_json::Value::String(description.to_string()),
            );
        }
        if let Some(system_prompt) = system_prompt {
            params.insert(
                "system_prompt".to_string(),
                serde_json::Value::String(system_prompt.to_string()),
            );
        }
        match parent_harness_id {
            Some(Some(parent_id)) => {
                params.insert(
                    "parent_harness_id".to_string(),
                    serde_json::Value::String(parent_id.to_string()),
                );
            }
            Some(None) => {
                params.insert("parent_harness_id".to_string(), serde_json::Value::Null);
            }
            None => {}
        }

        self.execute_domain_command("update_harness", serde_json::Value::Object(params))
            .await
    }

    async fn delete_harness(&self, id: HarnessId) -> everruns_provider::error::Result<()> {
        let _: serde_json::Value = self
            .execute_domain_command(
                "delete_harness",
                serde_json::json!({ "id": id.to_string() }),
            )
            .await?;
        Ok(())
    }

    async fn copy_harness(
        &self,
        id: HarnessId,
        new_name: Option<&str>,
    ) -> everruns_provider::error::Result<Harness> {
        let harness: Harness = self
            .execute_domain_command("copy_harness", serde_json::json!({ "id": id.to_string() }))
            .await?;

        if let Some(new_name) = new_name {
            self.update_harness(harness.id, Some(new_name), None, None, None, None)
                .await
        } else {
            Ok(harness)
        }
    }

    // =========================================================================
    // Agent Operations
    // =========================================================================

    async fn list_agents(&self) -> everruns_provider::error::Result<Vec<Agent>> {
        let page: CommandPage<Agent> = self
            .execute_domain_command(
                "list_agents",
                serde_json::json!({ "offset": 0, "limit": 1000 }),
            )
            .await?;
        Ok(page.data)
    }

    async fn get_agent_by_id(
        &self,
        id: AgentId,
    ) -> everruns_provider::error::Result<Option<Agent>> {
        self.execute_domain_lookup("get_agent", serde_json::json!({ "id": id.to_string() }))
            .await
    }

    async fn create_agent(
        &self,
        name: &str,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: &str,
        capabilities: &[String],
    ) -> everruns_provider::error::Result<Agent> {
        self.execute_domain_command(
            "create_agent",
            serde_json::json!({
                "name": name,
                "display_name": display_name,
                "description": description,
                "system_prompt": system_prompt,
                "tags": ["managed"],
                "initial_files": [],
                "tools": [],
                "mcp_servers": {},
                "capabilities": Self::capability_refs_to_configs(capabilities),
            }),
        )
        .await
    }

    async fn update_agent(
        &self,
        id: AgentId,
        name: Option<&str>,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
    ) -> everruns_provider::error::Result<Agent> {
        let mut params = serde_json::Map::from_iter([(
            "id".to_string(),
            serde_json::Value::String(id.to_string()),
        )]);
        if let Some(name) = name {
            params.insert(
                "name".to_string(),
                serde_json::Value::String(name.to_string()),
            );
        }
        if let Some(display_name) = display_name {
            params.insert(
                "display_name".to_string(),
                serde_json::Value::String(display_name.to_string()),
            );
        }
        if let Some(description) = description {
            params.insert(
                "description".to_string(),
                serde_json::Value::String(description.to_string()),
            );
        }
        if let Some(system_prompt) = system_prompt {
            params.insert(
                "system_prompt".to_string(),
                serde_json::Value::String(system_prompt.to_string()),
            );
        }

        self.execute_domain_command("update_agent", serde_json::Value::Object(params))
            .await
    }

    async fn delete_agent(&self, id: AgentId) -> everruns_provider::error::Result<()> {
        let _: serde_json::Value = self
            .execute_domain_command("delete_agent", serde_json::json!({ "id": id.to_string() }))
            .await?;
        Ok(())
    }

    // =========================================================================
    // App Operations
    // =========================================================================

    async fn list_apps(
        &self,
        search: Option<&str>,
        include_archived: bool,
    ) -> everruns_provider::error::Result<Vec<everruns_platform::App>> {
        let mut params = serde_json::Map::new();
        if let Some(search) = search {
            params.insert(
                "search".to_string(),
                serde_json::Value::String(search.to_string()),
            );
        }
        if include_archived {
            params.insert(
                "include_archived".to_string(),
                serde_json::Value::Bool(true),
            );
        }
        self.execute_domain_command("list_apps", serde_json::Value::Object(params))
            .await
    }

    async fn get_app(
        &self,
        id: everruns_provider::typed_id::AppId,
    ) -> everruns_provider::error::Result<Option<everruns_platform::App>> {
        self.execute_domain_lookup("get_app", serde_json::json!({ "id": id.to_string() }))
            .await
    }

    async fn create_app(
        &self,
        name: &str,
        description: Option<&str>,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        agent_identity_id: Option<everruns_provider::typed_id::AgentIdentityId>,
        channel_type: Option<everruns_platform::ChannelType>,
        channel_config: Option<&serde_json::Value>,
    ) -> everruns_provider::error::Result<everruns_platform::App> {
        let mut params = serde_json::Map::from_iter([
            (
                "name".to_string(),
                serde_json::Value::String(name.to_string()),
            ),
            (
                "harness_id".to_string(),
                serde_json::Value::String(harness_id.to_string()),
            ),
        ]);
        if let Some(description) = description {
            params.insert(
                "description".to_string(),
                serde_json::Value::String(description.to_string()),
            );
        }
        if let Some(agent_id) = agent_id {
            params.insert(
                "agent_id".to_string(),
                serde_json::Value::String(agent_id.to_string()),
            );
        }
        if let Some(agent_identity_id) = agent_identity_id {
            params.insert(
                "agent_identity_id".to_string(),
                serde_json::Value::String(agent_identity_id.to_string()),
            );
        }
        if let Some(channel_type) = channel_type {
            params.insert(
                "channel_type".to_string(),
                serde_json::Value::String(channel_type.to_string()),
            );
        }
        if let Some(channel_config) = channel_config {
            params.insert("channel_config".to_string(), channel_config.clone());
        }
        self.execute_domain_command("create_app", serde_json::Value::Object(params))
            .await
    }

    async fn update_app(
        &self,
        id: everruns_provider::typed_id::AppId,
        name: Option<&str>,
        description: Option<&str>,
        harness_id: Option<HarnessId>,
        agent_id: Option<AgentId>,
        agent_identity_id: Option<Option<everruns_provider::typed_id::AgentIdentityId>>,
    ) -> everruns_provider::error::Result<everruns_platform::App> {
        let mut params = serde_json::Map::from_iter([(
            "id".to_string(),
            serde_json::Value::String(id.to_string()),
        )]);
        if let Some(name) = name {
            params.insert(
                "name".to_string(),
                serde_json::Value::String(name.to_string()),
            );
        }
        if let Some(description) = description {
            params.insert(
                "description".to_string(),
                serde_json::Value::String(description.to_string()),
            );
        }
        if let Some(harness_id) = harness_id {
            params.insert(
                "harness_id".to_string(),
                serde_json::Value::String(harness_id.to_string()),
            );
        }
        if let Some(agent_id) = agent_id {
            params.insert(
                "agent_id".to_string(),
                serde_json::Value::String(agent_id.to_string()),
            );
        }
        match agent_identity_id {
            Some(Some(agent_identity_id)) => {
                params.insert(
                    "agent_identity_id".to_string(),
                    serde_json::Value::String(agent_identity_id.to_string()),
                );
            }
            Some(None) => {
                params.insert("agent_identity_id".to_string(), serde_json::Value::Null);
            }
            None => {}
        }
        self.execute_domain_command("update_app", serde_json::Value::Object(params))
            .await
    }

    async fn delete_app(
        &self,
        id: everruns_provider::typed_id::AppId,
    ) -> everruns_provider::error::Result<()> {
        let _: serde_json::Value = self
            .execute_domain_command("delete_app", serde_json::json!({ "id": id.to_string() }))
            .await?;
        Ok(())
    }

    async fn destroy_app(
        &self,
        id: everruns_provider::typed_id::AppId,
    ) -> everruns_provider::error::Result<()> {
        let _: serde_json::Value = self
            .execute_domain_command("destroy_app", serde_json::json!({ "id": id.to_string() }))
            .await?;
        Ok(())
    }

    async fn publish_app(
        &self,
        id: everruns_provider::typed_id::AppId,
    ) -> everruns_provider::error::Result<everruns_platform::App> {
        self.execute_domain_command("publish_app", serde_json::json!({ "id": id.to_string() }))
            .await
    }

    async fn unpublish_app(
        &self,
        id: everruns_provider::typed_id::AppId,
    ) -> everruns_provider::error::Result<everruns_platform::App> {
        self.execute_domain_command("unpublish_app", serde_json::json!({ "id": id.to_string() }))
            .await
    }

    async fn add_app_channel(
        &self,
        app_id: everruns_provider::typed_id::AppId,
        channel_type: everruns_platform::ChannelType,
        channel_config: Option<&serde_json::Value>,
        enabled: Option<bool>,
    ) -> everruns_provider::error::Result<everruns_platform::AppChannel> {
        let mut params = serde_json::Map::from_iter([
            (
                "app_id".to_string(),
                serde_json::Value::String(app_id.to_string()),
            ),
            (
                "channel_type".to_string(),
                serde_json::Value::String(channel_type.to_string()),
            ),
        ]);
        if let Some(channel_config) = channel_config {
            params.insert("channel_config".to_string(), channel_config.clone());
        }
        if let Some(enabled) = enabled {
            params.insert("enabled".to_string(), serde_json::Value::Bool(enabled));
        }
        self.execute_domain_command("add_app_channel", serde_json::Value::Object(params))
            .await
    }

    async fn update_app_channel(
        &self,
        app_id: everruns_provider::typed_id::AppId,
        channel_id: everruns_provider::typed_id::AppChannelId,
        channel_type: Option<everruns_platform::ChannelType>,
        channel_config: Option<&serde_json::Value>,
        enabled: Option<bool>,
    ) -> everruns_provider::error::Result<everruns_platform::AppChannel> {
        let mut params = serde_json::Map::from_iter([
            (
                "app_id".to_string(),
                serde_json::Value::String(app_id.to_string()),
            ),
            (
                "channel_id".to_string(),
                serde_json::Value::String(channel_id.to_string()),
            ),
        ]);
        if let Some(channel_type) = channel_type {
            params.insert(
                "channel_type".to_string(),
                serde_json::Value::String(channel_type.to_string()),
            );
        }
        if let Some(channel_config) = channel_config {
            params.insert("channel_config".to_string(), channel_config.clone());
        }
        if let Some(enabled) = enabled {
            params.insert("enabled".to_string(), serde_json::Value::Bool(enabled));
        }
        self.execute_domain_command("update_app_channel", serde_json::Value::Object(params))
            .await
    }

    async fn delete_app_channel(
        &self,
        app_id: everruns_provider::typed_id::AppId,
        channel_id: everruns_provider::typed_id::AppChannelId,
    ) -> everruns_provider::error::Result<()> {
        let _: serde_json::Value = self
            .execute_domain_command(
                "delete_app_channel",
                serde_json::json!({
                    "app_id": app_id.to_string(),
                    "channel_id": channel_id.to_string(),
                }),
            )
            .await?;
        Ok(())
    }

    // =========================================================================
    // Session Operations
    // =========================================================================

    async fn list_sessions(
        &self,
        limit: Option<usize>,
        agent_id: Option<AgentId>,
    ) -> everruns_provider::error::Result<Vec<Session>> {
        let page: CommandPage<Session> = self
            .execute_domain_command(
                "list_sessions",
                serde_json::json!({
                    "limit": limit.unwrap_or(20),
                    "agent_id": agent_id.map(|id| id.to_string()),
                }),
            )
            .await?;
        Ok(page.data)
    }

    async fn create_session(
        &self,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
        locale: Option<&str>,
        blueprint_id: Option<&str>,
        blueprint_config: Option<&serde_json::Value>,
        parent_session_id: Option<SessionId>,
    ) -> everruns_provider::error::Result<Session> {
        self.execute_domain_command(
            "create_session",
            serde_json::json!({
                "harness_id": harness_id.to_string(),
                "agent_id": agent_id.map(|id| id.to_string()),
                "title": title,
                "locale": locale,
                "tags": ["managed"],
                "capabilities": [],
                "tools": [],
                "mcp_servers": {},
                "initial_files": [],
                "blueprint_id": blueprint_id,
                "blueprint_config": blueprint_config,
                "parent_session_id": parent_session_id.map(|id| id.to_string()),
            }),
        )
        .await
    }

    async fn create_session_with_options(
        &self,
        request: everruns_platform::PlatformCreateSessionRequest,
    ) -> everruns_provider::error::Result<Session> {
        self.execute_domain_command(
            "create_session",
            serde_json::json!({
                "harness_id": request.harness_id.to_string(),
                "agent_id": request.agent_id.map(|id| id.to_string()),
                "title": request.title,
                "goal": request.goal,
                "locale": request.locale,
                "tags": ["managed"],
                "capabilities": [],
                "tools": [],
                "mcp_servers": {},
                "initial_files": [],
                "blueprint_id": request.blueprint_id,
                "blueprint_config": request.blueprint_config,
                "parent_session_id": request.parent_session_id.map(|id| id.to_string()),
                "forked_from_session_id": request.forked_from_session_id.map(|id| id.to_string()),
                "budget_root_session_id": request.budget_root_session_id.map(|id| id.to_string()),
                "seed": request.seed,
            }),
        )
        .await
    }

    async fn get_session_by_id(
        &self,
        id: SessionId,
    ) -> everruns_provider::error::Result<Option<Session>> {
        self.execute_domain_lookup(
            "get_session",
            serde_json::json!({ "session_id": id.to_string() }),
        )
        .await
    }

    async fn add_agent_session_participant(
        &self,
        session_id: SessionId,
        agent_id: AgentId,
    ) -> everruns_provider::error::Result<SessionParticipant> {
        self.execute_domain_command(
            "add_session_participant",
            serde_json::json!({
                "session_id": session_id.to_string(),
                "kind": "agent",
                "agent_id": agent_id.to_string(),
            }),
        )
        .await
    }

    async fn get_session_context_report(
        &self,
        id: SessionId,
    ) -> everruns_provider::error::Result<everruns_core::SessionContextReport> {
        self.execute_domain_command(
            "get_session_context_report",
            serde_json::json!({ "session_id": id.to_string() }),
        )
        .await
    }

    async fn delete_session(&self, id: SessionId) -> everruns_provider::error::Result<()> {
        let _: serde_json::Value = self
            .execute_domain_command(
                "delete_session",
                serde_json::json!({ "session_id": id.to_string() }),
            )
            .await?;
        Ok(())
    }

    // =========================================================================
    // Messaging
    // =========================================================================

    async fn send_message(
        &self,
        session_id: SessionId,
        content: &str,
    ) -> everruns_provider::error::Result<()> {
        let _: serde_json::Value = self
            .execute_domain_command(
                "create_message",
                serde_json::json!({
                    "session_id": session_id.to_string(),
                    "message": {
                        "content": [{ "type": "text", "text": content }],
                    },
                }),
            )
            .await?;
        Ok(())
    }

    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> everruns_provider::error::Result<Vec<everruns_platform::PlatformMessage>> {
        let mut messages: Vec<crate::api::messages::Message> = self
            .execute_domain_command(
                "list_messages",
                serde_json::json!({
                    "session_id": session_id.to_string(),
                    "limit": limit.unwrap_or(10),
                }),
            )
            .await?;
        messages.retain(|message| {
            matches!(
                message.role,
                crate::api::messages::MessageRole::User | crate::api::messages::MessageRole::Agent
            )
        });

        Ok(messages
            .into_iter()
            .filter_map(|message| {
                let content = message
                    .content
                    .iter()
                    .filter_map(|part| match part {
                        everruns_core::ContentPart::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if content.is_empty() {
                    return None;
                }
                Some(everruns_platform::PlatformMessage {
                    role: match message.role {
                        crate::api::messages::MessageRole::User => "user".to_string(),
                        _ => "agent".to_string(),
                    },
                    content,
                    created_at: message.created_at,
                })
            })
            .collect())
    }

    // =========================================================================
    // Turn Management
    // =========================================================================

    async fn wait_for_idle(
        &self,
        session_id: SessionId,
        timeout_secs: Option<u64>,
    ) -> everruns_provider::error::Result<String> {
        let timeout = std::time::Duration::from_secs(timeout_secs.unwrap_or(120));
        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_millis(500);

        loop {
            let session = self
                .get_session_by_id(session_id)
                .await?
                .ok_or_else(|| store_error("Session not found"))?;

            match session.status {
                SessionStatus::Idle => {
                    if let Some(status) = self.latest_terminal_turn_status(session_id).await? {
                        return Ok(status);
                    }
                    // Session status flips to idle independently from terminal
                    // turn-event persistence. Keep polling until the event lands
                    // so callers can distinguish successful and failed idle turns.
                }
                SessionStatus::Started => {
                    // Not yet active, keep waiting
                }
                SessionStatus::Active => {
                    // Turn in progress, keep waiting
                }
                SessionStatus::WaitingForToolResults => {
                    return Ok("waiting_for_tool_results".to_string());
                }
                SessionStatus::Paused => return Ok("paused".to_string()),
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
    ) -> everruns_provider::error::Result<Vec<everruns_core::CapabilityInfo>> {
        let mut params = serde_json::Map::new();
        params.insert("limit".to_string(), serde_json::json!(200));
        if let Some(search) = search {
            params.insert(
                "search".to_string(),
                serde_json::Value::String(search.to_string()),
            );
        }
        let page: CommandPage<everruns_core::CapabilityInfo> = self
            .execute_domain_command("list_capabilities", serde_json::Value::Object(params))
            .await?;
        Ok(page.data)
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
    use crate::storage::models::CreateHarnessRow;
    use everruns_platform::PlatformStore;

    #[test]
    fn string_to_provider_type_maps_gemini() {
        assert_eq!(string_to_provider_type("gemini").to_string(), "gemini");
    }

    #[test]
    fn string_to_provider_type_maps_unknown_to_external() {
        // Unknown ids resolve to External, preserving the id.
        assert_eq!(
            string_to_provider_type("custom-provider").to_string(),
            "custom-provider"
        );
    }

    // =========================================================================
    // Capability deduplication helpers (EVE-47)
    // =========================================================================

    /// Build a DirectWorkerAdapters with in-memory backends for unit tests.
    fn test_adapters() -> DirectWorkerAdapters {
        let db = Arc::new(crate::storage::StorageBackend::in_memory());
        let event_service = Arc::new(crate::services::EventService::new(
            db.clone(),
            crate::event_delivery::EventDelivery::in_memory(),
        ));
        let provider_resolver = Arc::new(crate::services::ProviderResolverService::new(
            db.clone(),
            None,
        ));
        let mcp_server_service = Arc::new(crate::domains::mcp_servers::McpServerService::new(
            db.clone(),
            None,
        ));
        let cap_registry = CapabilityRegistry::new();
        let driver_registry = everruns_worker::create_driver_registry();
        let sqldb_backend = Arc::new(crate::session_sqldb::InMemorySqlDbBackend::new());
        let sqldb_store: std::sync::Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore> =
            Arc::new(crate::session_sqldb::InMemorySqlDbStore::new(sqldb_backend));

        DirectWorkerAdapters::new(
            db,
            event_service,
            provider_resolver,
            mcp_server_service,
            cap_registry,
            driver_registry,
            sqldb_store,
        )
    }

    #[tokio::test]
    async fn get_agent_returns_none_for_missing_agent() {
        let adapters = test_adapters();
        let result = adapters
            .get_agent(everruns_core::DEFAULT_ORG_ID, Uuid::new_v4())
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_agent_resolves_by_public_id() {
        let adapters = test_adapters();
        let agent_id = seed_agent(&adapters.db).await;

        let agent = adapters
            .get_agent(everruns_core::DEFAULT_ORG_ID, agent_id)
            .await
            .unwrap()
            .expect("agent should exist");

        assert_eq!(agent.name, "test-agent");
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

    #[tokio::test]
    async fn grep_files_returns_bounded_merged_context() {
        let adapters = test_adapters();
        let session_id = Uuid::new_v4();
        seed_file(
            &adapters.db,
            session_id,
            "/events.log",
            "before\nError one\nbetween\nError two\nafter\n",
        )
        .await;

        let result = adapters
            .grep_files_with_options(
                session_id,
                "Error",
                &GrepOptions {
                    path_pattern: None,
                    before_context: 1,
                    after_context: 1,
                    offset: 0,
                    limit: 2,
                    max_bytes: everruns_core::GREP_MAX_RETURN_BYTES,
                },
            )
            .await
            .unwrap();

        assert_eq!(result.total_matches, 2);
        assert_eq!(result.returned_matches, 2);
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].match_line_numbers, vec![2, 4]);
        assert_eq!(result.blocks[0].lines.len(), 5);
    }

    async fn seed_harness_for_platform_store(
        db: &StorageBackend,
        org_id: i64,
        name: &str,
        is_built_in: bool,
    ) -> HarnessId {
        db.create_harness(
            org_id,
            CreateHarnessRow {
                name: name.to_string(),
                display_name: None,
                description: None,
                system_prompt: Some("test prompt".to_string()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in,
            },
        )
        .await
        .expect("seed harness")
        .id
    }

    async fn seed_platform_session(
        db: &StorageBackend,
        org_id: i64,
        harness_id: HarnessId,
        resolved_owner_user_id: Option<Uuid>,
    ) -> SessionId {
        use crate::storage::models::CreateSessionRow;

        db.create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id,
            app_id: None,
            harness_id: Some(harness_id),
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id,
            title: Some("platform-store-test".to_string()),
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
        })
        .await
        .expect("seed session")
        .id
    }

    async fn seed_platform_owner(db: &StorageBackend, org_id: i64, email: &str) -> Uuid {
        use crate::storage::models::CreateUserRow;

        let user = db
            .create_user(CreateUserRow {
                email: email.to_string(),
                name: "Platform Owner".to_string(),
                avatar_url: None,
                external_id: None,
                roles: vec![],
                password_hash: None,
                email_verified: true,
                auth_provider: Some("test".to_string()),
                auth_provider_id: None,
            })
            .await
            .expect("create owner user");
        db.ensure_membership(user.id, org_id, "owner")
            .await
            .expect("ensure owner membership");
        user.id
    }

    fn test_platform_store(
        adapters: &DirectWorkerAdapters,
        org_id: i64,
        session_id: SessionId,
    ) -> DirectPlatformStore {
        DirectPlatformStore::new(
            org_id,
            session_id,
            adapters.db.clone(),
            DirectPlatformStoreDeps {
                event_service: adapters.event_service.clone(),
                runner: None,
                capability_registry: adapters.capability_registry.clone(),
                connector_registry: adapters.connector_registry.clone(),
                encryption: None,
                workflow_store: None,
                permission_resolver: adapters.permission_resolver.clone(),
                egress_service: adapters.egress_service.clone(),
            },
        )
    }

    #[tokio::test]
    async fn platform_store_update_rejects_built_in_harness() {
        let adapters = test_adapters();
        let harness_id = seed_harness_for_platform_store(
            &adapters.db,
            everruns_core::DEFAULT_ORG_ID,
            "built-in",
            true,
        )
        .await;
        let owner_user_id = seed_platform_owner(
            &adapters.db,
            everruns_core::DEFAULT_ORG_ID,
            "built-in-update-owner@example.com",
        )
        .await;
        let session_id = seed_platform_session(
            &adapters.db,
            everruns_core::DEFAULT_ORG_ID,
            harness_id,
            Some(owner_user_id),
        )
        .await;
        let store = test_platform_store(&adapters, everruns_core::DEFAULT_ORG_ID, session_id);

        let err = store
            .update_harness(harness_id, Some("renamed"), None, None, None, None)
            .await
            .expect_err("built-in harness updates should be rejected");

        assert!(err.to_string().contains("built-in harness"));
    }

    #[tokio::test]
    async fn platform_store_delete_rejects_built_in_harness() {
        let adapters = test_adapters();
        let harness_id = seed_harness_for_platform_store(
            &adapters.db,
            everruns_core::DEFAULT_ORG_ID,
            "built-in",
            true,
        )
        .await;
        let owner_user_id = seed_platform_owner(
            &adapters.db,
            everruns_core::DEFAULT_ORG_ID,
            "built-in-delete-owner@example.com",
        )
        .await;
        let session_id = seed_platform_session(
            &adapters.db,
            everruns_core::DEFAULT_ORG_ID,
            harness_id,
            Some(owner_user_id),
        )
        .await;
        let store = test_platform_store(&adapters, everruns_core::DEFAULT_ORG_ID, session_id);

        let err = store
            .delete_harness(harness_id)
            .await
            .expect_err("built-in harness deletes should be rejected");

        assert!(err.to_string().contains("built-in harness"));
    }

    #[tokio::test]
    async fn platform_store_uses_session_owner_permissions() {
        use crate::storage::models::{CreateOrganizationRow, CreateUserRow};

        let adapters = test_adapters();
        let org = adapters
            .db
            .create_organization(CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000042".to_string(),
                name: "Platform Store Auth".to_string(),
                created_by: None,
            })
            .await
            .expect("create org");
        let user = adapters
            .db
            .create_user(CreateUserRow {
                email: "member@example.com".to_string(),
                name: "Member".to_string(),
                avatar_url: None,
                external_id: None,
                roles: vec![],
                password_hash: None,
                email_verified: true,
                auth_provider: Some("test".to_string()),
                auth_provider_id: None,
            })
            .await
            .expect("create user");
        adapters
            .db
            .ensure_membership(user.id, org.org_id, "member")
            .await
            .expect("ensure membership");

        let harness_id =
            seed_harness_for_platform_store(&adapters.db, org.org_id, "session-harness", false)
                .await;
        let session_id =
            seed_platform_session(&adapters.db, org.org_id, harness_id, Some(user.id)).await;
        let store = adapters.platform_store(org.org_id, session_id);

        let discovered = store
            .platform_discover(serde_json::json!({ "query": "models" }))
            .await
            .expect("members may inspect the command catalog");
        assert!(discovered.contains("list_models"));

        store
            .platform_query(serde_json::json!({ "commands": "list_models" }))
            .await
            .expect("members may run read-only platform queries");

        let script_error = store
            .platform_execute(serde_json::json!({
                "commands": "create_harness --name forbidden-script"
            }))
            .await
            .expect_err("member should not mutate through the command surface");
        assert!(
            script_error.to_string().contains("forbidden")
                || script_error.to_string().contains("Access denied"),
            "unexpected authorization error: {script_error}"
        );

        let err = store
            .create_harness("forbidden", None, None, Some("prompt"), None, &[])
            .await
            .expect_err("member should not manage harnesses via platform tools");

        assert!(
            err.to_string().contains("Access denied")
                || err.to_string().contains("Permission denied"),
            "unexpected error: {err}"
        );
    }

    struct DenySessionManageResolver;

    impl everruns_core::PermissionResolver for DenySessionManageResolver {
        fn has_permission(
            &self,
            caller: &everruns_core::Caller,
            permission: &everruns_core::Permission,
        ) -> bool {
            *permission != everruns_core::Permission::OrgSessionsManage
                && everruns_core::DefaultPermissionResolver.has_permission(caller, permission)
        }

        fn caller_permissions(
            &self,
            caller: &everruns_core::Caller,
        ) -> Vec<everruns_core::Permission> {
            everruns_core::DefaultPermissionResolver
                .caller_permissions(caller)
                .into_iter()
                .filter(|permission| *permission != everruns_core::Permission::OrgSessionsManage)
                .collect()
        }
    }

    #[tokio::test]
    async fn session_creation_authority_uses_owner_permission_and_returns_root() {
        let adapters = test_adapters();
        let org_id = everruns_core::DEFAULT_ORG_ID;
        let owner_id =
            seed_platform_owner(&adapters.db, org_id, "detached-authority-owner@example.com").await;
        let harness_id =
            seed_harness_for_platform_store(&adapters.db, org_id, "authority-harness", false).await;
        let root_id = seed_platform_session(&adapters.db, org_id, harness_id, Some(owner_id)).await;
        let authority = adapters
            .session_creation_authority(org_id, root_id)
            .expect("direct authority");
        assert_eq!(
            authority
                .authorize_session_creation(root_id)
                .await
                .expect("owner may create sessions"),
            root_id
        );

        let denied_adapters =
            test_adapters().with_permission_resolver(Arc::new(DenySessionManageResolver));
        let denied_owner = seed_platform_owner(
            &denied_adapters.db,
            org_id,
            "detached-authority-denied@example.com",
        )
        .await;
        let denied_harness = seed_harness_for_platform_store(
            &denied_adapters.db,
            org_id,
            "denied-authority-harness",
            false,
        )
        .await;
        let denied_session = seed_platform_session(
            &denied_adapters.db,
            org_id,
            denied_harness,
            Some(denied_owner),
        )
        .await;
        let denied = denied_adapters
            .session_creation_authority(org_id, denied_session)
            .expect("direct authority")
            .authorize_session_creation(denied_session)
            .await
            .expect_err("custom resolver denial must be honored");
        assert!(denied.to_string().contains("Access denied"));
    }

    /// Regression: the direct platform store's command ctx must carry the
    /// event service — wait_for_idle probes terminal turn events through the
    /// list_events command, and without it every subagent wait failed with
    /// "Event service not configured".
    #[tokio::test]
    async fn platform_store_wait_for_idle_reaches_event_service() {
        use everruns_core::DEFAULT_ORG_ID;

        let adapters = test_adapters();
        let user_id = seed_platform_owner(&adapters.db, DEFAULT_ORG_ID, "waiter@example.com").await;
        let harness_id =
            seed_harness_for_platform_store(&adapters.db, DEFAULT_ORG_ID, "wait-harness", false)
                .await;
        let session_id =
            seed_platform_session(&adapters.db, DEFAULT_ORG_ID, harness_id, Some(user_id)).await;
        let store = adapters.platform_store(DEFAULT_ORG_ID, session_id);

        // No terminal turn events exist, so a zero-timeout wait reports a
        // timeout — the point is it must NOT fail on a missing event service.
        let status = store
            .wait_for_idle(session_id, Some(0))
            .await
            .expect("wait_for_idle should reach the event service");
        assert!(status.starts_with("timeout"), "unexpected status: {status}");
    }

    // ---- helpers ----

    /// Seed a test agent, returning its UUID.
    ///
    /// Uses `create_agent_with_id` so public_id matches the internal UUID,
    /// consistent with normal agent creation via the API.
    async fn seed_agent(db: &StorageBackend) -> Uuid {
        use crate::storage::models::CreateAgentRow;
        let id = AgentId::new();
        let public_id = id.to_string();
        let create = CreateAgentRow {
            public_id,
            name: "test-agent".to_string(),
            display_name: Some("Test Agent".to_string()),
            description: None,
            system_prompt: String::new(),
            default_model_id: None,
            harness_id: everruns_provider::typed_id::HarnessId::from_uuid(uuid::Uuid::nil()),
            tags: vec![],
            initial_files: serde_json::Value::Array(vec![]),
            tools: serde_json::Value::Array(vec![]),
            mcp_servers: serde_json::json!({}),
            max_iterations: None,
            network_access: None,
            parallel_tool_calls: None,
            is_built_in: false,
        };
        db.create_agent_with_id(everruns_core::DEFAULT_ORG_ID, id, create)
            .await
            .expect("seed agent")
            .expect("seed agent should create");
        id.uuid()
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
        use crate::storage::models::CreateOrganizationRow;

        let adapters = test_adapters();
        let agent_id = seed_agent(&adapters.db).await;
        let org2 = adapters
            .db
            .create_organization(CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000999".to_string(),
                name: "Platform Store Org 2".to_string(),
                created_by: None,
            })
            .await
            .expect("create org 2");
        let harness_org1 = seed_harness_for_platform_store(
            &adapters.db,
            everruns_core::DEFAULT_ORG_ID,
            "org1-session-harness",
            false,
        )
        .await;
        let harness_org2 = seed_harness_for_platform_store(
            &adapters.db,
            org2.org_id,
            "org2-session-harness",
            false,
        )
        .await;
        let owner_org1 = seed_platform_owner(
            &adapters.db,
            everruns_core::DEFAULT_ORG_ID,
            "org1-owner@example.com",
        )
        .await;
        let owner_org2 =
            seed_platform_owner(&adapters.db, org2.org_id, "org2-owner@example.com").await;
        let session_org1 = seed_platform_session(
            &adapters.db,
            everruns_core::DEFAULT_ORG_ID,
            harness_org1,
            Some(owner_org1),
        )
        .await;
        let session_org2 =
            seed_platform_session(&adapters.db, org2.org_id, harness_org2, Some(owner_org2)).await;

        // Agent seeded in org 1 should be visible via org 1's platform store
        let store_org1 = adapters.platform_store(everruns_core::DEFAULT_ORG_ID, session_org1);
        let agent = store_org1
            .get_agent_by_id(everruns_provider::typed_id::AgentId::from_uuid(agent_id))
            .await
            .unwrap();
        assert!(agent.is_some(), "agent should be visible in org 1");

        // Same agent must NOT be visible via org 2's platform store
        let store_org2 = adapters.platform_store(org2.org_id, session_org2);
        let agent = store_org2
            .get_agent_by_id(everruns_provider::typed_id::AgentId::from_uuid(agent_id))
            .await
            .unwrap();
        assert!(agent.is_none(), "agent must NOT be visible in org 2");
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
                "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                &[],
            )
            .unwrap(),
        )
    }

    fn test_adapters_with_encryption() -> DirectWorkerAdapters {
        let encryption = test_encryption();
        let db = Arc::new(crate::storage::StorageBackend::in_memory());
        let event_service = Arc::new(crate::services::EventService::new(
            db.clone(),
            crate::event_delivery::EventDelivery::in_memory(),
        ));
        let provider_resolver = Arc::new(crate::services::ProviderResolverService::new(
            db.clone(),
            None,
        ));
        let mcp_server_service = Arc::new(crate::domains::mcp_servers::McpServerService::new(
            db.clone(),
            Some(encryption),
        ));
        let cap_registry = CapabilityRegistry::new();
        let driver_registry = everruns_worker::create_driver_registry();
        let sqldb_backend = Arc::new(crate::session_sqldb::InMemorySqlDbBackend::new());
        let sqldb_store: std::sync::Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore> =
            Arc::new(crate::session_sqldb::InMemorySqlDbStore::new(sqldb_backend));

        DirectWorkerAdapters::new(
            db,
            event_service,
            provider_resolver,
            mcp_server_service,
            cap_registry,
            driver_registry,
            sqldb_store,
        )
    }

    fn test_adapters_with_budget_service() -> DirectWorkerAdapters {
        let adapters = test_adapters();
        let budget_service = Arc::new(crate::domains::budgets::BudgetService::new(
            adapters.db.clone(),
        ));
        adapters.with_budget_service(budget_service)
    }

    #[tokio::test]
    async fn budget_checker_is_absent_without_service() {
        let adapters = test_adapters();
        assert!(
            adapters
                .budget_checker(everruns_core::DEFAULT_ORG_ID, None)
                .is_none()
        );
    }

    #[tokio::test]
    async fn budget_checker_returns_no_budgets_without_rows() {
        let adapters = test_adapters_with_budget_service();
        let checker = adapters
            .budget_checker(everruns_core::DEFAULT_ORG_ID, None)
            .expect("budget checker should be wired");

        let response = checker
            .check_budgets("session_test_budget")
            .await
            .expect("budget check should succeed");

        assert_eq!(response.status, "no_budgets");
        assert!(response.budgets.is_empty());
        assert!(response.hint.is_some());
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
            .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, None, "my_server")
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
            .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, None, "auth_server")
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
            .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, None, "no_enc_server")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_mcp_server_by_prefix_errors_when_server_not_found() {
        let adapters = test_adapters();
        let result = adapters
            .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, None, "nonexistent")
            .await;
        assert!(result.is_err());
    }

    // =========================================================================
    // Adapter contract test harness (EVE-61)
    //
    // Reusable macro-driven suite that defines behavioral contracts for
    // WorkerAdapters. Each test is parameterised by an adapter constructor,
    // so it runs against DirectWorkerAdapters today and can be wired to
    // GrpcWorkerAdapters once an in-process gRPC loopback is available.
    // =========================================================================

    macro_rules! adapter_contract_tests {
        ($mod_name:ident, $make_adapters:expr) => {
            mod $mod_name {
                use super::*;

                #[tokio::test]
                async fn grep_single_match() {
                    let (adapters, db) = $make_adapters;
                    let sid = Uuid::new_v4();
                    seed_file(&db, sid, "/hello.rs", "fn main() {\n    hello();\n}\n").await;
                    let results = adapters.grep_files(sid, "hello", None).await.unwrap();
                    assert_eq!(results.len(), 1);
                    assert_eq!(results[0].path, "/hello.rs");
                    assert_eq!(results[0].line_number, 2);
                    assert!(results[0].line.contains("hello"));
                }

                #[tokio::test]
                async fn grep_no_match_returns_empty() {
                    let (adapters, db) = $make_adapters;
                    let sid = Uuid::new_v4();
                    seed_file(&db, sid, "/code.rs", "let x = 1;\n").await;
                    let results = adapters
                        .grep_files(sid, "no_such_pattern", None)
                        .await
                        .unwrap();
                    assert!(results.is_empty());
                }

                #[tokio::test]
                async fn grep_multiple_files_and_lines() {
                    let (adapters, db) = $make_adapters;
                    let sid = Uuid::new_v4();
                    seed_file(&db, sid, "/a.txt", "ERR line1\nok\nERR line3\n").await;
                    seed_file(&db, sid, "/b.txt", "ok\nERR line2\n").await;
                    let results = adapters.grep_files(sid, "ERR", None).await.unwrap();
                    assert_eq!(results.len(), 3);
                    let a: Vec<_> = results.iter().filter(|m| m.path == "/a.txt").collect();
                    assert_eq!(a.len(), 2);
                    assert_eq!(a[0].line_number, 1);
                    assert_eq!(a[1].line_number, 3);
                    let b: Vec<_> = results.iter().filter(|m| m.path == "/b.txt").collect();
                    assert_eq!(b.len(), 1);
                    assert_eq!(b[0].line_number, 2);
                }

                #[tokio::test]
                async fn grep_regex_pattern() {
                    let (adapters, db) = $make_adapters;
                    let sid = Uuid::new_v4();
                    seed_file(&db, sid, "/nums.txt", "val 1\nval 22\nval 333\n").await;
                    let results = adapters.grep_files(sid, r"\d{2,}", None).await.unwrap();
                    assert_eq!(results.len(), 2);
                }

                #[tokio::test]
                async fn grep_invalid_regex_is_error() {
                    let (adapters, _db) = $make_adapters;
                    let sid = Uuid::new_v4();
                    assert!(adapters.grep_files(sid, "[bad", None).await.is_err());
                }

                #[tokio::test]
                async fn grep_empty_session_returns_empty() {
                    let (adapters, _db) = $make_adapters;
                    let sid = Uuid::new_v4();
                    let results = adapters.grep_files(sid, "anything", None).await.unwrap();
                    assert!(results.is_empty());
                }

                #[tokio::test]
                async fn write_then_read_file() {
                    let (adapters, _db) = $make_adapters;
                    let sid = Uuid::new_v4();
                    let written = adapters
                        .write_file(sid, "/test.txt", "content", "text")
                        .await
                        .unwrap();
                    assert_eq!(written.path, "/test.txt");
                    let read = adapters.read_file(sid, "/test.txt").await.unwrap();
                    assert!(read.is_some());
                    assert_eq!(read.unwrap().path, "/test.txt");
                }

                #[tokio::test]
                async fn read_nonexistent_file_returns_none() {
                    let (adapters, _db) = $make_adapters;
                    let sid = Uuid::new_v4();
                    assert!(
                        adapters
                            .read_file(sid, "/nope.txt")
                            .await
                            .unwrap()
                            .is_none()
                    );
                }

                #[tokio::test]
                async fn delete_file_returns_true() {
                    let (adapters, _db) = $make_adapters;
                    let sid = Uuid::new_v4();
                    adapters
                        .write_file(sid, "/del.txt", "bye", "text")
                        .await
                        .unwrap();
                    assert!(adapters.delete_file(sid, "/del.txt", false).await.unwrap());
                    assert!(adapters.read_file(sid, "/del.txt").await.unwrap().is_none());
                }

                #[tokio::test]
                async fn resolve_image_missing_returns_none() {
                    let (adapters, _db) = $make_adapters;
                    let result = adapters
                        .resolve_image(everruns_core::DEFAULT_ORG_ID, Uuid::new_v4())
                        .await
                        .unwrap();
                    assert!(result.is_none());
                }

                #[tokio::test]
                async fn resolve_images_batch_empty_ids() {
                    let (adapters, _db) = $make_adapters;
                    let result = adapters
                        .resolve_images_batch(everruns_core::DEFAULT_ORG_ID, &[])
                        .await
                        .unwrap();
                    assert!(result.is_empty());
                }

                #[tokio::test]
                async fn get_session_nonexistent_returns_none() {
                    let (adapters, _db) = $make_adapters;
                    let result = adapters
                        .get_session(everruns_core::DEFAULT_ORG_ID, Uuid::new_v4())
                        .await
                        .unwrap();
                    assert!(result.is_none());
                }

                #[tokio::test]
                async fn get_agent_nonexistent_returns_none() {
                    let (adapters, _db) = $make_adapters;
                    let result = adapters
                        .get_agent(everruns_core::DEFAULT_ORG_ID, Uuid::new_v4())
                        .await
                        .unwrap();
                    assert!(result.is_none());
                }

                #[tokio::test]
                async fn mcp_server_not_found_is_error() {
                    let (adapters, _db) = $make_adapters;
                    assert!(
                        adapters
                            .get_mcp_server_by_prefix(
                                everruns_core::DEFAULT_ORG_ID,
                                None,
                                "no_such_prefix"
                            )
                            .await
                            .is_err()
                    );
                }

                #[tokio::test]
                async fn mcp_server_no_api_key() {
                    let (adapters, db) = $make_adapters;
                    seed_mcp_server(&db, "Plain Server", None).await;
                    let info = adapters
                        .get_mcp_server_by_prefix(
                            everruns_core::DEFAULT_ORG_ID,
                            None,
                            "plain_server",
                        )
                        .await
                        .unwrap();
                    assert!(info.api_key.is_none());
                    assert_eq!(info.name, "Plain Server");
                }

                #[tokio::test]
                async fn mcp_server_with_encrypted_api_key() {
                    let adapters = test_adapters_with_encryption();
                    let encrypted = test_encryption().encrypt_string("sk-contract").unwrap();
                    seed_mcp_server(&adapters.db, "Enc Server", Some(encrypted)).await;
                    let info = adapters
                        .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, None, "enc_server")
                        .await
                        .unwrap();
                    assert_eq!(info.api_key.as_deref(), Some("sk-contract"));
                }
            }
        };
    }

    adapter_contract_tests!(direct_adapter_contract, {
        let a = test_adapters();
        let db = a.db.clone();
        (a, db)
    });

    // =========================================================================
    // Cross-org image resolution regression (EVE-56 / EVE-61)
    // =========================================================================

    async fn seed_image(db: &StorageBackend, org_id: i64) -> Uuid {
        use crate::storage::models::CreateImageRow;
        let row = db
            .create_image(
                org_id,
                CreateImageRow {
                    org_id,
                    filename: "test.png".to_string(),
                    content_type: "image/png".to_string(),
                    size_bytes: 4,
                    data: vec![0x89, 0x50, 0x4E, 0x47],
                    thumbnail_data: None,
                    thumbnail_content_type: None,
                    metadata: serde_json::json!({}),
                },
            )
            .await
            .expect("seed image");
        row.id.into()
    }

    #[tokio::test]
    async fn resolve_image_cross_org_isolation() {
        let adapters = test_adapters();
        let image_id = seed_image(&adapters.db, everruns_core::DEFAULT_ORG_ID).await;
        let found = adapters
            .resolve_image(everruns_core::DEFAULT_ORG_ID, image_id)
            .await
            .unwrap();
        assert!(found.is_some(), "image should be visible in owning org");
        let cross = adapters.resolve_image(999, image_id).await.unwrap();
        assert!(
            cross.is_none(),
            "image must NOT be visible in different org"
        );
    }

    #[tokio::test]
    async fn resolve_images_batch_cross_org_isolation() {
        let adapters = test_adapters();
        let img1 = seed_image(&adapters.db, everruns_core::DEFAULT_ORG_ID).await;
        let img2 = seed_image(&adapters.db, everruns_core::DEFAULT_ORG_ID).await;
        let batch = adapters
            .resolve_images_batch(everruns_core::DEFAULT_ORG_ID, &[img1, img2])
            .await
            .unwrap();
        assert_eq!(batch.len(), 2);
        let cross = adapters
            .resolve_images_batch(999, &[img1, img2])
            .await
            .unwrap();
        assert!(cross.is_empty(), "images must not leak across orgs");
    }

    // =========================================================================
    // MCP auth-required execution coverage (EVE-55 / EVE-61)
    // =========================================================================

    #[tokio::test]
    async fn mcp_server_encrypted_key_decrypts_correctly() {
        let adapters = test_adapters_with_encryption();
        let enc = test_encryption();
        let key = "sk-mcp-auth-test-12345";
        let encrypted = enc.encrypt_string(key).unwrap();
        seed_mcp_server(&adapters.db, "Auth Required MCP", Some(encrypted)).await;
        let info = adapters
            .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, None, "auth_required_mcp")
            .await
            .unwrap();
        assert_eq!(info.api_key.as_deref(), Some(key));
        assert_eq!(info.url, "https://example.com/mcp");
    }

    #[tokio::test]
    async fn mcp_server_encrypted_key_without_encryption_service_fails() {
        let adapters = test_adapters();
        let enc = test_encryption();
        let encrypted = enc.encrypt_string("sk-should-fail").unwrap();
        seed_mcp_server(&adapters.db, "Fail MCP", Some(encrypted)).await;
        let result = adapters
            .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, None, "fail_mcp")
            .await;
        assert!(
            result.is_err(),
            "decryption without encryption service must fail"
        );
    }

    #[tokio::test]
    async fn mcp_server_wrong_org_not_found() {
        let adapters = test_adapters();
        seed_mcp_server(&adapters.db, "Org Scoped MCP", None).await;
        assert!(
            adapters
                .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, None, "org_scoped_mcp")
                .await
                .is_ok()
        );
        assert!(
            adapters
                .get_mcp_server_by_prefix(999, None, "org_scoped_mcp")
                .await
                .is_err(),
            "MCP server must not be visible in wrong org"
        );
    }

    // =========================================================================
    // Cross-org isolation regression tests (EVE-59)
    // =========================================================================

    /// Regression: get_session must populate organization_id from org_id,
    /// not hardcode DEFAULT_ORG_PUBLIC_ID.
    #[tokio::test]
    async fn get_session_carries_org_public_id() {
        use crate::storage::models::CreateSessionRow;

        let adapters = test_adapters();
        let agent_id = seed_agent(&adapters.db).await;

        let row = adapters
            .db
            .create_session(CreateSessionRow {
                source: everruns_platform::SessionSource::Api,
                workspace_id: None,
                org_id: everruns_core::DEFAULT_ORG_ID,
                app_id: None,
                agent_id: Some(AgentId::from_uuid(agent_id)),
                agent_version_id: None,
                agent_config_hash: None,
                agent_identity_id: None,
                harness_id: Some(HarnessId::from_seed(1)),
                owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                title: None,
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::Value::Array(vec![]),
                tools: serde_json::Value::Array(vec![]),
                mcp_servers: serde_json::json!({}),
                system_prompt: None,
                initial_files: serde_json::Value::Array(vec![]),
                hints: None,
                max_iterations: None,
                parallel_tool_calls: None,
                blueprint_id: None,
                blueprint_config: None,
                network_access: None,
                parent_session_id: None,
                budget_root_session_id: None,
            })
            .await
            .expect("create session");

        let session = adapters
            .get_session(everruns_core::DEFAULT_ORG_ID, row.id.uuid())
            .await
            .unwrap()
            .expect("session should exist");

        assert_eq!(
            session.organization_id,
            everruns_core::DEFAULT_ORG_PUBLIC_ID,
            "session must carry the correct org public_id"
        );
    }

    // =========================================================================
    // Encrypted DB key model sync regression (EVE-57 / EVE-61)
    // =========================================================================

    #[tokio::test]
    async fn get_model_spec_returns_none_for_missing() {
        let adapters = test_adapters();
        let result = adapters
            .get_model_spec(everruns_core::DEFAULT_ORG_ID, Uuid::new_v4())
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_default_model_spec_returns_none_when_no_providers() {
        let adapters = test_adapters();
        let result = adapters
            .get_default_model_spec(everruns_core::DEFAULT_ORG_ID)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    // =========================================================================
    // Usage attribution org scoping (EVE-56 / EVE-61)
    // =========================================================================

    #[tokio::test]
    async fn platform_store_agent_count_isolated_per_org() {
        use crate::storage::models::CreateOrganizationRow;

        let adapters = test_adapters();
        seed_agent(&adapters.db).await;
        let org2 = adapters
            .db
            .create_organization(CreateOrganizationRow {
                public_id: "org_00000000000000000000000000000998".to_string(),
                name: "Platform Store Agent Count Org 2".to_string(),
                created_by: None,
            })
            .await
            .expect("create org 2");
        let harness_org1 = seed_harness_for_platform_store(
            &adapters.db,
            everruns_core::DEFAULT_ORG_ID,
            "agent-count-org1",
            false,
        )
        .await;
        let harness_org2 =
            seed_harness_for_platform_store(&adapters.db, org2.org_id, "agent-count-org2", false)
                .await;
        let owner_org1 = seed_platform_owner(
            &adapters.db,
            everruns_core::DEFAULT_ORG_ID,
            "agent-count-org1-owner@example.com",
        )
        .await;
        let owner_org2 = seed_platform_owner(
            &adapters.db,
            org2.org_id,
            "agent-count-org2-owner@example.com",
        )
        .await;
        let session_org1 = seed_platform_session(
            &adapters.db,
            everruns_core::DEFAULT_ORG_ID,
            harness_org1,
            Some(owner_org1),
        )
        .await;
        let session_org2 =
            seed_platform_session(&adapters.db, org2.org_id, harness_org2, Some(owner_org2)).await;

        let store_org1 = adapters.platform_store(everruns_core::DEFAULT_ORG_ID, session_org1);
        let agents_org1 = store_org1.list_agents().await.unwrap();
        assert!(!agents_org1.is_empty(), "default org should have agents");
        let store_org2 = adapters.platform_store(org2.org_id, session_org2);
        let agents_org2 = store_org2.list_agents().await.unwrap();
        assert!(agents_org2.is_empty(), "second org should have no agents");
    }
}
