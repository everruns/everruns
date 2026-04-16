// Session-owned managed sandbox lifecycle service.
//
// Design:
// - Server owns auto-start and idle-pause orchestration; providers stay focused
//   on sandbox operations.
// - This is intentionally best-effort and in-process for the experimental flag.
// - Capability config is resolved from effective harness + agent + session layers,
//   matching normal capability merge behavior.

use crate::domains::harnesses::queries::resolve_effective as resolve_effective_harness;
use crate::org_init::BASE_HARNESS_ID;
use crate::storage::{DbLeasedResourceStore, DbSessionResourceRegistry, StorageBackend};
use anyhow::Context;
use async_trait::async_trait;
use everruns_core::session_sandbox::{
    SessionSandboxConfig, ensure_session_sandbox_running, pause_session_sandbox,
    session_sandbox_config_from_capabilities,
};
use everruns_core::typed_id::{AgentId, SessionId};
use everruns_core::{
    AgentCapabilityConfig, Event, EventData, EventListener, LeasedResourceStore,
    SessionResourceRegistry, SessionStorageStore, ToolContext, UserConnectionResolver,
    merge_capabilities,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct SessionSandboxService {
    db: Arc<StorageBackend>,
    storage_store: Arc<dyn SessionStorageStore>,
    leased_resource_store: Arc<dyn LeasedResourceStore>,
    session_resource_registry: Arc<dyn SessionResourceRegistry>,
    connection_resolver: Option<Arc<dyn UserConnectionResolver>>,
    idle_tasks: Arc<Mutex<HashMap<SessionId, tokio::task::JoinHandle<()>>>>,
}

impl SessionSandboxService {
    pub fn new(
        db: Arc<StorageBackend>,
        storage_store: Arc<dyn SessionStorageStore>,
        connection_resolver: Option<Arc<dyn UserConnectionResolver>>,
    ) -> Self {
        let session_resource_registry: Arc<dyn SessionResourceRegistry> =
            Arc::new(DbSessionResourceRegistry::new(db.clone()));
        let leased_resource_store: Arc<dyn LeasedResourceStore> = Arc::new(
            DbLeasedResourceStore::new(db.clone()).with_registry(session_resource_registry.clone()),
        );

        Self {
            db,
            storage_store,
            leased_resource_store,
            session_resource_registry,
            connection_resolver,
            idle_tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn auto_start_for_capabilities(
        &self,
        session_id: SessionId,
        effective_capabilities: &[AgentCapabilityConfig],
    ) {
        let config = match session_sandbox_config_from_capabilities(effective_capabilities) {
            Ok(Some(config)) if config.auto_start => config,
            Ok(_) => return,
            Err(err) => {
                tracing::warn!(session_id = %session_id, error = %err, "Invalid session_sandbox config");
                return;
            }
        };

        if let Err(err) = self.ensure_started(session_id, &config).await {
            tracing::warn!(
                session_id = %session_id,
                error = ?err,
                "Failed to auto-start session sandbox"
            );
        }
    }

    pub async fn ensure_started_for_session(&self, session_id: SessionId) {
        let Some(config) = self.config_for_session(session_id).await.ok().flatten() else {
            return;
        };
        if !config.auto_start {
            return;
        }
        if let Err(err) = self.ensure_started(session_id, &config).await {
            tracing::warn!(
                session_id = %session_id,
                error = ?err,
                "Failed to auto-start session sandbox"
            );
        }
    }

    pub async fn cancel_idle_pause(&self, session_id: SessionId) {
        if let Some(handle) = self.idle_tasks.lock().await.remove(&session_id) {
            handle.abort();
        }
    }

    pub async fn schedule_idle_pause(&self, session_id: SessionId) {
        self.cancel_idle_pause(session_id).await;

        let Some(config) = self.config_for_session(session_id).await.ok().flatten() else {
            return;
        };

        let timeout = config.idle_pause_after_seconds;
        let service = self.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(timeout)).await;
            if let Err(err) = service.pause_for_session(session_id, &config).await {
                tracing::warn!(
                    session_id = %session_id,
                    error = ?err,
                    "Failed to auto-pause session sandbox"
                );
            }
            service.idle_tasks.lock().await.remove(&session_id);
        });
        self.idle_tasks.lock().await.insert(session_id, handle);
    }

    async fn ensure_started(
        &self,
        session_id: SessionId,
        config: &SessionSandboxConfig,
    ) -> Result<(), everruns_core::ToolExecutionResult> {
        let context = self.tool_context(session_id);
        ensure_session_sandbox_running(&context, config).await?;
        Ok(())
    }

    async fn pause_for_session(
        &self,
        session_id: SessionId,
        config: &SessionSandboxConfig,
    ) -> Result<(), everruns_core::ToolExecutionResult> {
        let context = self.tool_context(session_id);
        pause_session_sandbox(&context, config).await?;
        Ok(())
    }

    async fn config_for_session(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<SessionSandboxConfig>> {
        let Some(org_id) = self
            .db
            .get_session_organization_id(session_id)
            .await
            .context("failed to resolve session org")?
        else {
            return Ok(None);
        };
        let Some(session_row) = self
            .db
            .get_session(org_id, session_id)
            .await
            .context("failed to load session")?
        else {
            return Ok(None);
        };

        let harness_id = session_row
            .harness_id
            .unwrap_or_else(|| everruns_core::HarnessId::from_uuid(BASE_HARNESS_ID));
        let Some(harness) = resolve_effective_harness(self.db.as_ref(), org_id, harness_id)
            .await
            .context("failed to resolve effective harness")?
        else {
            return Ok(None);
        };
        let agent_capabilities = if let Some(agent_id) = session_row.agent_id {
            self.agent_capabilities(agent_id).await?
        } else {
            Vec::new()
        };
        let session_capabilities: Vec<AgentCapabilityConfig> =
            serde_json::from_value(session_row.capabilities)
                .context("failed to parse session capabilities")?;

        let merged = merge_capabilities(&harness.capabilities, &agent_capabilities);
        let merged = merge_capabilities(&merged, &session_capabilities);
        session_sandbox_config_from_capabilities(&merged).map_err(|err| anyhow::anyhow!(err))
    }

    async fn agent_capabilities(
        &self,
        agent_id: AgentId,
    ) -> anyhow::Result<Vec<AgentCapabilityConfig>> {
        self.db
            .get_agent_capabilities(agent_id.uuid())
            .await
            .context("failed to load agent capabilities")
            .map(|rows| {
                rows.into_iter()
                    .map(|row| AgentCapabilityConfig::with_config(row.capability_id, row.config))
                    .collect()
            })
    }

    fn tool_context(&self, session_id: SessionId) -> ToolContext {
        let mut context = ToolContext::new(session_id);
        context.storage_store = Some(self.storage_store.clone());
        context.leased_resource_store = Some(self.leased_resource_store.clone());
        context.session_resource_registry = Some(self.session_resource_registry.clone());
        context.connection_resolver = self.connection_resolver.clone();
        context
    }
}

pub struct SessionSandboxEventListener {
    service: Arc<SessionSandboxService>,
}

impl SessionSandboxEventListener {
    pub fn new(service: Arc<SessionSandboxService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl EventListener for SessionSandboxEventListener {
    async fn on_event(&self, event: &Event) {
        match &event.data {
            EventData::SessionActivated(_) => {
                self.service.cancel_idle_pause(event.session_id).await;
            }
            EventData::SessionIdled(_) => {
                self.service.schedule_idle_pause(event.session_id).await;
            }
            _ => {}
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            everruns_core::SESSION_ACTIVATED,
            everruns_core::SESSION_IDLED,
        ])
    }

    fn name(&self) -> &'static str {
        "SessionSandboxEventListener"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateHarnessRow, CreateSessionRow, StorageBackend};
    use everruns_core::session_sandbox::{
        SessionSandboxExecRequest, SessionSandboxExecResponse, SessionSandboxInstance,
        SessionSandboxProvider, SessionSandboxReadFileResponse, SessionSandboxState,
        SessionSandboxStatus, SessionSandboxStatusResponse, SessionSandboxWriteFileResponse,
        load_session_sandbox_state,
    };
    use everruns_core::{DEFAULT_ORG_ID, InitialFile, SessionStorageStore};
    use serde_json::json;

    struct TestSessionSandboxProvider;

    inventory::submit! {
        everruns_core::SessionSandboxProviderPlugin {
            factory: || Box::new(TestSessionSandboxProvider),
        }
    }

    #[async_trait]
    impl SessionSandboxProvider for TestSessionSandboxProvider {
        fn id(&self) -> &str {
            "test-session-sandbox"
        }

        async fn create(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
        ) -> Result<SessionSandboxInstance, everruns_core::ToolExecutionResult> {
            Ok(SessionSandboxInstance {
                external_id: "sb_test".to_string(),
                display_name: Some("Test Sandbox".to_string()),
                workspace_path: Some("/workspace".to_string()),
                provider_state: json!({}),
                metadata: json!({}),
            })
        }

        async fn resume(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            instance: &SessionSandboxInstance,
        ) -> Result<SessionSandboxInstance, everruns_core::ToolExecutionResult> {
            Ok(instance.clone())
        }

        async fn pause(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            instance: &SessionSandboxInstance,
        ) -> Result<SessionSandboxInstance, everruns_core::ToolExecutionResult> {
            Ok(instance.clone())
        }

        async fn delete(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
        ) -> Result<(), everruns_core::ToolExecutionResult> {
            Ok(())
        }

        async fn exec(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
            _request: &SessionSandboxExecRequest,
        ) -> Result<SessionSandboxExecResponse, everruns_core::ToolExecutionResult> {
            Ok(SessionSandboxExecResponse {
                exit_code: 0,
                output: "ok".to_string(),
                raw_output: Some("ok".to_string()),
                hint: None,
            })
        }

        async fn read_file(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
            path: &str,
        ) -> Result<SessionSandboxReadFileResponse, everruns_core::ToolExecutionResult> {
            Ok(SessionSandboxReadFileResponse {
                path: path.to_string(),
                content: "data".to_string(),
                encoding: "text".to_string(),
            })
        }

        async fn write_file(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
            path: &str,
            content: &str,
        ) -> Result<SessionSandboxWriteFileResponse, everruns_core::ToolExecutionResult> {
            Ok(SessionSandboxWriteFileResponse {
                path: path.to_string(),
                bytes_written: content.len(),
            })
        }

        async fn status(
            &self,
            _context: &ToolContext,
            _config: &SessionSandboxConfig,
            state: &SessionSandboxState,
        ) -> Result<SessionSandboxStatusResponse, everruns_core::ToolExecutionResult> {
            Ok(SessionSandboxStatusResponse {
                provider: state.provider.clone(),
                session_status: state.status,
                external_id: state.instance.external_id.clone(),
                display_name: state.instance.display_name.clone(),
                workspace_path: state.instance.workspace_path.clone(),
                metadata: json!({}),
            })
        }
    }

    fn test_storage_store(db: &Arc<StorageBackend>) -> Arc<dyn SessionStorageStore> {
        match db.as_ref() {
            StorageBackend::InMemory(mem_db) => mem_db.clone(),
            StorageBackend::Postgres(_) => unreachable!(),
        }
    }

    async fn create_test_harness(db: &StorageBackend) -> everruns_core::HarnessId {
        db.create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: "sandbox-harness".to_string(),
                display_name: Some("Sandbox Harness".to_string()),
                description: Some("test".to_string()),
                system_prompt: "test".to_string(),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::to_value(Vec::<InitialFile>::new()).unwrap(),
                network_access: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn auto_start_creates_session_sandbox_state() {
        let db = Arc::new(StorageBackend::in_memory());
        let harness_id = create_test_harness(db.as_ref()).await;
        db.set_harness_capabilities(
            harness_id.uuid(),
            vec![(
                "session_sandbox".to_string(),
                0,
                json!({
                    "provider": "test-session-sandbox",
                    "auto_start": true,
                    "idle_pause_after_seconds": 1
                }),
            )],
        )
        .await
        .unwrap();

        let session = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: Some(harness_id),
                agent_id: None,
                agent_identity_id: None,
                title: Some("test".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: json!([]),
                tools: json!([]),
                system_prompt: None,
                initial_files: json!([]),
                hints: None,
                network_access: None,
                max_iterations: None,
                blueprint_id: None,
                blueprint_config: None,
            })
            .await
            .unwrap();

        let service = SessionSandboxService::new(db.clone(), test_storage_store(&db), None);
        service.ensure_started_for_session(session.id).await;

        let state = load_session_sandbox_state(&service.tool_context(session.id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.provider, "test-session-sandbox");
        assert_eq!(state.status, SessionSandboxStatus::Running);
    }

    #[tokio::test]
    async fn idle_pause_marks_state_paused() {
        let db = Arc::new(StorageBackend::in_memory());
        let harness_id = create_test_harness(db.as_ref()).await;
        db.set_harness_capabilities(
            harness_id.uuid(),
            vec![(
                "session_sandbox".to_string(),
                0,
                json!({
                    "provider": "test-session-sandbox",
                    "auto_start": true,
                    "idle_pause_after_seconds": 1
                }),
            )],
        )
        .await
        .unwrap();
        let session = db
            .create_session(CreateSessionRow {
                org_id: DEFAULT_ORG_ID,
                harness_id: Some(harness_id),
                agent_id: None,
                agent_identity_id: None,
                title: Some("test".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: json!([]),
                tools: json!([]),
                system_prompt: None,
                initial_files: json!([]),
                hints: None,
                network_access: None,
                max_iterations: None,
                blueprint_id: None,
                blueprint_config: None,
            })
            .await
            .unwrap();

        let service = SessionSandboxService::new(db.clone(), test_storage_store(&db), None);
        service.ensure_started_for_session(session.id).await;
        service.schedule_idle_pause(session.id).await;
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

        let state = load_session_sandbox_state(&service.tool_context(session.id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.status, SessionSandboxStatus::Paused);
    }
}
