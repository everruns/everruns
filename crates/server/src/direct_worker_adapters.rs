mod command_context;

mod definition_reads;
mod message_projection;

// Direct implementation of WorkerAdapters for in-process worker
//
// Decision: Uses StorageBackend, domains, and infra helpers directly (no gRPC)
// Decision: Used by in-process worker in DEV_MODE
//
// Implements GrpcWorkerAdapters' interface using storage, domains, and infra directly.
use crate::domains::budgets::BudgetService;
use crate::domains::mcp_servers::McpServerService;
use crate::domains::mcp_servers::scoped_mcp::{
    build_materialized_scoped_mcp_tool_definitions,
    merge_effective_scoped_mcp_servers_with_capabilities,
    resolve_scoped_mcp_server_with_capabilities, validate_effective_mcp_servers,
};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::kernel_imports::{
    Caller, EgressRequest, EgressRequestKind, EgressService, RuntimeMessage, UtilityLlmService,
    contracts::driver_registry::DriverRegistry, contracts::provider::DriverId,
    contracts::tool_types::ToolDefinition, resolve_runtime_capabilities,
};
use crate::kernel_imports::{
    connection_services::ProviderCredentialStore, contracts::model_spec::ModelSpec,
    delegation_services::SessionCreationAuthority, file_services::ResolvedFile,
    image_services::CreateStoredImage, image_services::ImageArtifactStore,
    image_services::ResolvedImage, image_services::StoredImage, image_services::StoredImageInfo,
    tool_execution::BudgetChecker, tool_execution::PaymentAuthority,
};
use crate::max_iterations;
use crate::records::Harness;
use crate::records::{Agent, AgentStatus};
use crate::records::{Session, SessionStatus};
use crate::services::{EventService, ProviderResolverService};
use crate::storage::models::{AgentCapabilityRow, AgentRow, UpdateSession};
use crate::storage::{EncryptionService, StorageBackend};
use async_trait::async_trait;
use everruns_contracts::CapabilityRef as AgentCapabilityConfig;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::budget::{BudgetSummary, BudgetToolResponse};
use everruns_core::capabilities::{CapabilityRegistry, collect_message_filters_only};
use everruns_core::connection_services::ProviderCredentials;
use everruns_core::events::{Event, EventRequest};
use everruns_core::message_retriever::MessageRetriever;
use everruns_core::permissions::PermissionResolver;
use everruns_core::session_file::{
    FileInfo, FileStat, GrepMatch, GrepOptions, GrepSearchResult, SessionFile,
};
use everruns_durable::WorkflowEventStore;
use everruns_worker::mcp_executor::McpServerInfo;
use everruns_worker::worker_adapters::{TurnContext, WorkerAdapters};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;
// Helper to create store errors
pub(crate) fn store_error(msg: impl Into<String>) -> AgentLoopError {
    AgentLoopError::store(msg)
}
/// Extract file name from path
pub(crate) fn name_from_path(path: &str) -> String {
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
        image_id: everruns_contracts::typed_id::ImageId,
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
        image_id: everruns_contracts::typed_id::ImageId,
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
    input_message_id: Option<Uuid>,
    pub(crate) db: Arc<StorageBackend>,
    event_service: Arc<EventService>,
    budget_service: Option<Arc<BudgetService>>,
    provider_resolver: Arc<ProviderResolverService>,
    mcp_server_service: Arc<McpServerService>,
    capability_registry: CapabilityRegistry,
    connector_registry: everruns_contracts::connector::ConnectorRegistry,
    driver_registry: DriverRegistry,
    utility_llm_service: Option<Arc<dyn UtilityLlmService>>,
    decisions: Option<Arc<dyn everruns_core::DecisionsService>>,
    egress_service: Option<Arc<dyn EgressService>>,
    sqldb_store: std::sync::Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore>,
    storage_store: Option<Arc<dyn everruns_core::session_services::SessionStorageStore>>,
    connection_resolver:
        Option<Arc<dyn everruns_core::connection_services::UserConnectionResolver>>,
    /// Platform vector store for Knowledge Index retrieval (`search_index`).
    vector_store: Option<Arc<dyn everruns_contracts::vector_store::VectorStore>>,
    runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
    encryption: Option<Arc<EncryptionService>>,
    in_memory_compaction_checkpoint_store:
        Option<Arc<everruns_core::host::InMemoryCompactionCheckpointStore>>,
    proactive_compaction_attempts: Arc<everruns_core::ProactiveCompactionAttemptTracker>,
    workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    slack_provisioner: Option<Arc<dyn crate::records::slack_provisioning::SlackAppProvisioner>>,
    permission_resolver: Arc<dyn PermissionResolver>,
    pub(crate) virtual_registry:
        Option<Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>>,
    org_rate_limiter: Option<Arc<crate::auth::rate_limit::OrgRateLimiter>>,
    pub(crate) quota: crate::domains::session_files::limits::QuotaLimits,
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
        sqldb_store: std::sync::Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore>,
    ) -> Self {
        Self {
            input_message_id: None,
            db,
            event_service,
            budget_service: None,
            provider_resolver,
            mcp_server_service,
            capability_registry,
            connector_registry: everruns_contracts::connector::ConnectorRegistry::new(),
            driver_registry,
            utility_llm_service: None,
            decisions: None,
            egress_service: None,
            sqldb_store,
            storage_store: None,
            connection_resolver: None,
            vector_store: None,
            runner: None,
            encryption: None,
            in_memory_compaction_checkpoint_store: None,
            proactive_compaction_attempts: Arc::default(),
            workflow_store: None,
            slack_provisioner: None,
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

        let Some(mut r) = row else {
            return Ok(None);
        };
        if let Some(message) = self.input_message_id {
            if !self
                .db
                .runtime_invocation_exists(r.id, message)
                .await
                .map_err(|e| store_error(e.to_string()))?
            {
                return Err(store_error("Unknown invocation"));
            }
            let responder = self
                .db
                .runtime_invocation_responder(r.id, message)
                .await
                .map_err(|e| store_error(e.to_string()))?
                .map(AgentId::from_uuid);

            if r.agent_id != responder {
                r.agent_version_id = None;
            }

            r.agent_id = responder;
        }
        if r.locale.is_none() {
            let profile = match self.input_message_id {
                Some(message) => self
                    .db
                    .runtime_invocation_subject(r.id, message)
                    .await
                    .map_err(|e| store_error(e.to_string()))?,
                None => None,
            };
            if let Some(id) = profile.or(r.virtual_user_id)
                && let Some(profile) = self
                    .db
                    .get_virtual_user(org_id, id)
                    .await
                    .map_err(|e| store_error(e.to_string()))?
            {
                r.locale = profile.locale;
            }
        }
        let capabilities = serde_json::from_value(r.capabilities).unwrap_or_default();
        let capabilities =
            crate::domains::capabilities::queries::hydrate_declarative_capability_configs(
                &self.db,
                org_id,
                capabilities,
            )
            .await?;

        Ok(Some({
            // Parse capabilities from JSON
            Session {
                source: crate::records::SessionSource::from(r.source.as_str()),
                run_summary: r.run_summary.clone(),
                activity: crate::records::SessionActivity::derive(
                    &crate::records::SessionStatus::from(r.status.as_str()),
                    r.last_turn_status.as_deref(),
                ),
                id: r.id,
                organization_id: everruns_core::org_public_id_from_internal(org_id),
                workspace_id: everruns_contracts::typed_id::WorkspaceId::from_uuid(r.workspace_id),
                harness_id: r.harness_id.unwrap_or_else(|| HarnessId::from_seed(1)),
                agent_id: r.agent_id,
                agent_version_id: r.agent_version_id,
                virtual_user_id: r.virtual_user_id,
                playground_user_id: r.playground_user_id,
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
    ) -> Result<Vec<RuntimeMessage>> {
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
        registry: everruns_contracts::connector::ConnectorRegistry,
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

    pub fn with_dev_mode_in_memory_compaction_checkpoints(mut self) -> Self {
        self.in_memory_compaction_checkpoint_store = Some(Arc::default());
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
        vector_store: Arc<dyn everruns_contracts::vector_store::VectorStore>,
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

    pub fn with_slack_provisioner(
        mut self,
        provisioner: Option<Arc<dyn crate::records::slack_provisioning::SlackAppProvisioner>>,
    ) -> Self {
        self.slack_provisioner = provisioner;
        self
    }

    pub fn with_decisions(mut self, service: Arc<dyn everruns_core::DecisionsService>) -> Self {
        self.decisions = Some(service);
        self
    }

    pub fn with_egress_service(mut self, service: Arc<dyn EgressService>) -> Self {
        self.egress_service = Some(service);
        self
    }

    /// Ensure a directory exists, creating it and parents if needed
    pub(crate) async fn ensure_directory_exists(&self, session_id: Uuid, path: &str) -> Result<()> {
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

    async fn get_harness(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<everruns_core::HarnessDefinition>> {
        self.get_harness_impl(org_id, id)
            .await?
            .map(|h| h.execution_definition())
            .transpose()
    }
    async fn get_agent(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<everruns_core::AgentDefinition>> {
        self.get_agent_record(org_id, id)
            .await?
            .map(|a| a.execution_definition())
            .transpose()
    }
    async fn get_agent_blocker(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<everruns_core::DependencyBlocker>> {
        Ok(match self.get_agent_record(org_id, id).await? {
            Some(a) => a.dependency_blocker(),
            None => Some(everruns_core::DependencyBlocker::AgentDeleted),
        })
    }
    async fn get_harness_blocker(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<everruns_core::DependencyBlocker>> {
        Ok(match self.get_harness_impl(org_id, id).await? {
            Some(h) => h.dependency_blocker(),
            None => Some(everruns_core::DependencyBlocker::HarnessDeleted),
        })
    }

    async fn resolve_agent_read(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<(
        Result<Option<everruns_core::AgentDefinition>>,
        Option<everruns_core::DependencyBlocker>,
    )> {
        // One stored read supplies both lifecycle validation and portable setup.
        let Some(agent) = self.get_agent_record(org_id, id).await? else {
            return Ok((
                Ok(None),
                Some(everruns_core::DependencyBlocker::AgentDeleted),
            ));
        };
        Ok((
            agent.execution_definition().map(Some),
            agent.dependency_blocker(),
        ))
    }

    async fn resolve_harness_read(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<(
        Result<Option<everruns_core::HarnessDefinition>>,
        Option<everruns_core::DependencyBlocker>,
    )> {
        // Resolve inheritance before projecting, exactly as the ordinary loader does.
        let Some(harness) = self.get_harness_impl(org_id, id).await? else {
            return Ok((
                Ok(None),
                Some(everruns_core::DependencyBlocker::HarnessDeleted),
            ));
        };
        Ok((
            harness.execution_definition().map(Some),
            harness.dependency_blocker(),
        ))
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
        let id = SessionId::from_uuid(session_id);
        let update = UpdateSession {
            status: Some(status.to_string()),
            ..Default::default()
        };
        self.db
            .update_session(org_id, id, update)
            .await
            .inspect_err(|e| tracing::error!("Failed to update session status: {e}"))
            .map_err(|_| store_error("Failed to update session status"))?;
        if status == "waiting_for_tool_results" {
            let store = self.workflow_store.as_deref();
            crate::tool_result_timeout::arm_parked_turn(&self.db, store, org_id, id).await;
        }
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

    async fn get_message(
        &self,
        session_id: Uuid,
        message_id: Uuid,
    ) -> Result<Option<RuntimeMessage>> {
        let messages = self.load_messages(session_id).await?;
        Ok(messages.into_iter().find(|m| m.id == message_id))
    }

    async fn load_messages(&self, session_id: Uuid) -> Result<Vec<RuntimeMessage>> {
        let events = self
            .event_service
            .list_message_events(session_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to load message events: {}", e);
                store_error("Failed to load messages")
            })?;

        let messages: Vec<RuntimeMessage> = events
            .into_iter()
            .filter_map(message_projection::event_to_message)
            .collect();
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

    async fn get_provider_config_for_session(
        &self,
        org_id: i64,
        provider: &everruns_contracts::ProviderKey,
        session: SessionId,
    ) -> Result<Option<everruns_contracts::driver_registry::ProviderConfig>> {
        let resolved = self
            .provider_resolver
            .resolve_runtime_provider_config_for_session(
                org_id,
                provider.as_str(),
                Some(session.uuid()),
            )
            .await
            .map_err(|_| {
                store_error("Provider is unavailable for this session's runtime identity")
            })?;
        Ok(resolved.map(|value| {
            let mut config = everruns_contracts::driver_registry::ProviderConfig::for_provider(
                provider.clone(),
                string_to_provider_type(&value.provider_type),
            );
            config.api_key = value.api_key;
            config.base_url = value.base_url;
            config.request_options = value.request_options;
            config
        }))
    }

    async fn get_provider_config(
        &self,
        org_id: i64,
        provider: &everruns_contracts::runtime_provider::ProviderKey,
    ) -> Result<Option<everruns_contracts::driver_registry::ProviderConfig>> {
        let resolved = self
            .provider_resolver
            .resolve_runtime_provider_config(org_id, provider.as_str())
            .await
            .map_err(|error| {
                tracing::error!(%error, "Failed to resolve provider");
                store_error("Failed to resolve provider")
            })?;
        Ok(resolved.map(|value| {
            let mut config = everruns_contracts::driver_registry::ProviderConfig::for_provider(
                provider.clone(),
                string_to_provider_type(&value.provider_type),
            );
            config.api_key = value.api_key;
            config.base_url = value.base_url;
            config.request_options = value.request_options;
            config
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

    async fn resolve_files_batch(
        &self,
        org_id: i64,
        file_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, ResolvedFile>> {
        let mut result = HashMap::new();
        for &file_id in file_ids {
            let file_row = self.db.get_file(org_id, file_id).await.map_err(|e| {
                tracing::error!("Failed to get file: {}", e);
                store_error("Failed to get file")
            })?;
            if let Some(row) = file_row {
                // Convert file data to base64
                use base64::Engine;
                let base64_data = base64::engine::general_purpose::STANDARD.encode(&row.data);
                result.insert(
                    file_id,
                    ResolvedFile {
                        base64: base64_data,
                        media_type: row.content_type,
                        filename: row.filename,
                    },
                );
            }
        }
        Ok(result)
    }

    // =========================================================================
    // Session File Operations
    // =========================================================================
    //
    // Bodies live in `direct_worker_adapters_files`; a trait impl cannot span
    // modules, so these forward.

    async fn read_file(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFile>> {
        DirectWorkerAdapters::read_file(self, org_id, session_id, path).await
    }

    async fn write_file(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        DirectWorkerAdapters::write_file(self, org_id, session_id, path, content, encoding).await
    }

    async fn write_file_if_content_matches(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        DirectWorkerAdapters::write_file_if_content_matches(
            self,
            org_id,
            session_id,
            path,
            expected_content,
            expected_encoding,
            content,
            encoding,
        )
        .await
    }

    async fn delete_file(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        DirectWorkerAdapters::delete_file(self, org_id, session_id, path, recursive).await
    }

    async fn list_directory(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
    ) -> Result<Vec<FileInfo>> {
        DirectWorkerAdapters::list_directory(self, org_id, session_id, path).await
    }

    async fn stat_file(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<FileStat>> {
        DirectWorkerAdapters::stat_file(self, org_id, session_id, path).await
    }

    async fn grep_files(
        &self,
        org_id: i64,
        session_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        DirectWorkerAdapters::grep_files(self, org_id, session_id, pattern, path_pattern).await
    }

    async fn grep_files_with_options(
        &self,
        org_id: i64,
        session_id: Uuid,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        DirectWorkerAdapters::grep_files_with_options(self, org_id, session_id, pattern, options)
            .await
    }

    async fn create_directory(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
    ) -> Result<FileInfo> {
        DirectWorkerAdapters::create_directory(self, org_id, session_id, path).await
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
                Some(agent_id) => self.get_agent_record(org_id, agent_id.uuid()).await?,
                None => None,
            };
            if let (Some(agent), Some(version_id)) = (agent.as_mut(), session.agent_version_id)
                && let Some(version_row) = self.db.get_agent_version(org_id, version_id).await?
            {
                let version = crate::domains::agents::queries::row_to_agent_version(version_row);
                *agent = crate::domains::agents::queries::version_to_agent(agent, &version);
            }

            if let Some(resolved) = resolve_scoped_mcp_server_with_capabilities(
                &self.mcp_server_service,
                org_id,
                &harness,
                agent.as_ref(),
                &session,
                server_prefix,
                &self.capability_registry,
            )
            .await
            .map_err(|e| store_error(format!("Failed to resolve scoped MCP server: {e}")))?
            {
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
                return Ok(
                    crate::direct_worker_adapters_mcp::resolved_mcp_server_to_worker_info(
                        resolved,
                        secret_bindings,
                    ),
                );
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

        Ok(
            crate::direct_worker_adapters_mcp::resolved_mcp_server_to_worker_info(
                resolved,
                secret_bindings,
            ),
        )
    }

    // =========================================================================
    // Turn Context (batch operation)
    // =========================================================================

    async fn load_turn_context_for_execution(
        &self,
        org_id: i64,
        session_id: Uuid,
        input_message_id: Uuid,
    ) -> Result<TurnContext> {
        let mut bound = self.clone();
        bound.input_message_id = Some(input_message_id);
        if let Some(resolver) = &self.connection_resolver {
            bound.connection_resolver = resolver
                .for_execution(input_message_id)
                .or_else(|| Some(resolver.clone()));
        }
        bound.load_turn_context(org_id, session_id).await
    }
    async fn get_mcp_server_for_execution(
        &self,
        org_id: i64,
        session_id: Uuid,
        server_prefix: &str,
        input_message_id: Uuid,
    ) -> Result<McpServerInfo> {
        let mut bound = self.clone();
        bound.input_message_id = Some(input_message_id);
        bound
            .get_mcp_server_by_prefix(org_id, Some(session_id), server_prefix)
            .await
    }
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
                    .await?;
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

            if let Err(error) = validate_effective_mcp_servers(&effective) {
                tracing::warn!(error = %error, "Invalid scoped MCP server config, skipping");
                vec![]
            } else {
                let egress = self.egress_service.clone().unwrap_or_else(|| {
                    Arc::new(
                        everruns_core::host::DirectEgressService::for_runtime_traffic_from_env(),
                    )
                });
                match build_materialized_scoped_mcp_tool_definitions(
                    &self.db,
                    org_id,
                    &effective,
                    Some(session.id),
                    self.connection_resolver.as_ref(),
                    egress.as_ref(),
                )
                .await
                {
                    Ok(definitions) => definitions,
                    Err(error) => {
                        tracing::warn!(error = %error, "Failed to build scoped MCP tool definitions");
                        vec![]
                    }
                }
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
            agent: agent.map(|a| a.execution_definition()).transpose()?,
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
        _org_id: i64,
    ) -> std::sync::Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore> {
        // In-process: the store talks to the database directly, so the org is
        // already enforced by the commands and queries that reach it.
        self.sqldb_store.clone()
    }

    fn slack_action_invoker(
        &self,
        org_id: i64,
        session_id: everruns_contracts::typed_id::SessionId,
    ) -> Option<Arc<dyn everruns_capabilities::slack_action::SlackActionInvoker>> {
        Some(crate::slack_actions::in_process_invoker(
            &self.db,
            self.encryption.as_ref(),
            org_id,
            session_id,
        ))
    }

    fn sandbox_persistence_store(
        &self,
    ) -> Option<Arc<dyn everruns_capabilities::sandbox_state::SandboxPersistenceStore>> {
        self.db.pool().map(|pool| {
            Arc::new(crate::storage::PgSandboxCheckpointStore::new(pool.clone()))
                as Arc<dyn everruns_capabilities::sandbox_state::SandboxPersistenceStore>
        })
    }

    fn native_async_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::native_async_store::NativeAsyncStore>> {
        crate::storage::PgNativeAsyncStore::shared(&self.db, self.encryption.as_ref())
    }
    fn agents_api_store(&self) -> Option<Arc<dyn everruns_core::agents_api_store::AgentsApiStore>> {
        crate::storage::PgAgentsApiStore::shared(&self.db, self.encryption.as_ref())
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
        self.in_memory_compaction_checkpoint_store
            .clone()
            .map(|store| store as Arc<dyn everruns_core::CompactionCheckpointStore>)
    }

    fn storage_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn everruns_core::session_services::SessionStorageStore> {
        self.storage_store_unscoped()
    }

    fn storage_store_unscoped(
        &self,
    ) -> Arc<dyn everruns_core::session_services::SessionStorageStore> {
        self.storage_store
            .clone()
            .expect("DirectWorkerAdapters: storage_store not set (call with_storage_store)")
    }

    fn knowledge_store(&self) -> Option<Arc<dyn everruns_capabilities::KnowledgeStore>> {
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

    fn decisions(&self) -> Option<Arc<dyn everruns_core::DecisionsService>> {
        self.decisions.clone()
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
                slack_provisioner: self.slack_provisioner.clone(),
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

    async fn invoke_scheduled_channel(
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

        let result = crate::domains::agent_channels::invoke_scheduled_legacy_alias_channel(
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
    ) -> Arc<dyn everruns_capabilities::PlatformStore> {
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
                slack_provisioner: self.slack_provisioner.clone(),
                permission_resolver: self.permission_resolver.clone(),
                egress_service: self.egress_service.clone(),
            },
        ))
    }

    fn knowledge_index_search(
        &self,
        _org_id: i64,
    ) -> Option<Arc<dyn everruns_contracts::vector_store::KnowledgeIndexSearch>> {
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
        resource_id: everruns_contracts::typed_id::LeasedResourceId,
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
        resource_id: everruns_contracts::typed_id::LeasedResourceId,
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
    ) -> Result<Vec<(everruns_contracts::typed_id::SessionId, String)>> {
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
        use everruns_contracts::tool_types::{BuiltinTool, DeferrablePolicy, ToolPolicy};
        use everruns_core::mcp::parse_mcp_capability_id;
        use everruns_core::mcp_server::mcp_tool_name;

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
                        hints: everruns_contracts::tool_types::ToolHints::default()
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
        session_id: everruns_contracts::typed_id::SessionId,
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

        let message_id = everruns_contracts::typed_id::MessageId::new();
        let now = chrono::Utc::now();
        let core_message = everruns_core::RuntimeMessage {
            id: message_id,
            role: everruns_core::RuntimeMessageRole::User,
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
#[path = "direct_worker_adapters/task_webhook_request_tests.rs"]
mod task_webhook_request_tests;

// =============================================================================
// DirectPlatformStore - PlatformStore implementation for in-process worker
// =============================================================================

/// Direct PlatformStore backed by StorageBackend + EventService + AgentRunner.
// THREAT[TM-AGENT-017]: All ops org-scoped via org_id field
struct DirectPlatformStoreDeps {
    event_service: Arc<EventService>,
    runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
    capability_registry: CapabilityRegistry,
    connector_registry: everruns_contracts::connector::ConnectorRegistry,
    encryption: Option<Arc<EncryptionService>>,
    workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    slack_provisioner: Option<Arc<dyn crate::records::slack_provisioning::SlackAppProvisioner>>,
    permission_resolver: Arc<dyn PermissionResolver>,
    egress_service: Option<Arc<dyn EgressService>>,
}

#[derive(Clone)]
pub struct DirectPlatformStore {
    input_message_id: Option<Uuid>,
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
    slack_provisioner: Option<Arc<dyn crate::records::slack_provisioning::SlackAppProvisioner>>,
    permission_resolver: Arc<dyn PermissionResolver>,
    connector_registry: everruns_contracts::connector::ConnectorRegistry,
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
            input_message_id: None,
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
            slack_provisioner: deps.slack_provisioner,
            permission_resolver: deps.permission_resolver,
            connector_registry: deps.connector_registry,
        }
    }

    fn base_url_from_env() -> String {
        everruns_core::config::env_string_any(
            &["PUBLIC_APP_URL", "FRONTEND_URL", "APP_URL"],
            "http://localhost:9300",
        )
    }

    async fn resolve_caller(&self) -> everruns_contracts::error::Result<Caller> {
        let message = self
            .input_message_id
            .ok_or_else(|| store_error("Management-authorized invocation required"))?;
        let user = self
            .db
            .runtime_invocation_management_user(self.session_id, message)
            .await
            .map_err(|e| store_error(e.to_string()))?
            .ok_or_else(|| store_error("Management-authorized invocation required"))?;
        crate::auth::caller_resolution::caller_for_user(&self.db, self.org_id, user)
            .await
            .map_err(|e| store_error(e.to_string()))
    }

    async fn execute_domain_command<T>(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> everruns_contracts::error::Result<T>
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

    async fn execute_runtime_command<T>(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> everruns_contracts::error::Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let ctx = self.command_ctx().await?;
        let json = crate::services::runtime_command_view::dispatch_runtime_view(name, params, &ctx)
            .await
            .map_err(|e| store_error(format!("Command {name} failed: {e}")))?;
        serde_json::from_str(&json)
            .map_err(|e| store_error(format!("Failed to decode {name} response: {e}")))
    }

    async fn execute_runtime_lookup<T>(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> everruns_contracts::error::Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        let ctx = self.command_ctx().await?;
        match crate::services::runtime_command_view::dispatch_runtime_view(name, params, &ctx).await
        {
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
    ) -> everruns_contracts::error::Result<String> {
        let (base_url, ctx) = (Self::base_url_from_env(), self.command_ctx().await?);
        let context = crate::api::mcp_endpoint::catalog::CatalogContext {
            domain_ctx: crate::domains::change_history::on_platform(ctx, self.session_id).await,
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
    fn for_execution(&self, id: Uuid) -> Option<Arc<dyn SessionCreationAuthority>> {
        let mut bound = self.clone();
        bound.input_message_id = Some(id);
        Some(Arc::new(bound))
    }

    async fn authorize_session_creation(
        &self,
        session_id: SessionId,
    ) -> everruns_contracts::error::Result<SessionId> {
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

#[path = "direct_worker_adapters/platform_store.rs"]
mod platform_store;

#[cfg(test)]
#[path = "direct_worker_adapters/tests.rs"]
mod tests;

mod decision_models;
