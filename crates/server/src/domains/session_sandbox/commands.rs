use super::queries as q;
use super::types::{
    GetSessionSandboxResponse, ManageSessionSandboxResponse, SessionSandboxAction,
    SessionSandboxStatusValue,
};
use crate::domains::common::*;
use everruns_platform::session_sandbox::{
    create_session_sandbox_provider, delete_session_sandbox, ensure_session_sandbox_running,
    load_session_sandbox_state, pause_session_sandbox,
};
use serde::Deserialize;
use utoipa::ToSchema;

fn status_response(
    configured: bool,
    exists: bool,
    provider: Option<String>,
    state: Option<&everruns_platform::session_sandbox::SessionSandboxState>,
    status: Option<everruns_platform::session_sandbox::SessionSandboxStatusResponse>,
) -> GetSessionSandboxResponse {
    let session_status = status
        .as_ref()
        .map(|status| SessionSandboxStatusValue::from(status.session_status));
    GetSessionSandboxResponse {
        configured,
        exists,
        provider,
        session_status,
        external_id: status.as_ref().map(|status| status.external_id.clone()),
        display_name: status
            .as_ref()
            .and_then(|status| status.display_name.clone()),
        workspace_path: status
            .as_ref()
            .and_then(|status| status.workspace_path.clone()),
        metadata: status.map(|status| status.metadata),
        init_completed_at: state.and_then(|state| state.init_completed_at.clone()),
        last_init_error: state.and_then(|state| state.last_init_error.clone()),
        created_at: state.map(|state| state.created_at.clone()),
        updated_at: state.map(|state| state.updated_at.clone()),
    }
}

fn manage_response_from_state(
    action: SessionSandboxAction,
    state: &everruns_platform::session_sandbox::SessionSandboxState,
) -> ManageSessionSandboxResponse {
    ManageSessionSandboxResponse {
        action,
        exists: true,
        deleted: false,
        provider: Some(state.provider.clone()),
        session_status: Some(SessionSandboxStatusValue::from(state.status)),
        external_id: Some(state.instance.external_id.clone()),
        display_name: state.instance.display_name.clone(),
        workspace_path: state.instance.workspace_path.clone(),
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetSessionSandbox {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for GetSessionSandbox {
    type Output = GetSessionSandboxResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session_sandbox",
            category: "session_sandbox",
            description: "Inspect the managed sandbox owned by a session.",
            method: "GET",
            path: "/v1/sessions/{session_id}/sandbox",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<GetSessionSandboxResponse, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session_access(ctx, session_id).await?;

        let service = q::session_sandbox_service(ctx)?;
        let Some(config) = service
            .config_for_session(session_id)
            .await
            .map_err(classify_anyhow)?
        else {
            return Ok(status_response(false, false, None, None, None));
        };

        let tool_context = service.tool_context(session_id);
        let Some(state) = load_session_sandbox_state(&tool_context)
            .await
            .map_err(q::map_tool_error)?
        else {
            return Ok(status_response(
                true,
                false,
                Some(config.provider),
                None,
                None,
            ));
        };

        let provider = create_session_sandbox_provider(&config.provider).ok_or_else(|| {
            CommandError::bad_request(format!(
                "Session sandbox provider '{}' is not registered",
                config.provider
            ))
        })?;
        let status = provider
            .status(&tool_context, &config, &state)
            .await
            .map_err(q::map_tool_error)?;

        Ok(status_response(
            true,
            true,
            Some(status.provider.clone()),
            Some(&state),
            Some(status),
        ))
    }
}

inventory::submit! { CommandDescriptor::of::<GetSessionSandbox>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ManageSessionSandbox {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub action: SessionSandboxAction,
}

impl Command for ManageSessionSandbox {
    type Output = ManageSessionSandboxResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "manage_session_sandbox",
            category: "session_sandbox",
            description: "Pause, resume, or delete the managed sandbox for a session.",
            method: "POST",
            path: "/v1/sessions/{session_id}/sandbox",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<ManageSessionSandboxResponse, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session_access(ctx, session_id).await?;

        let service = q::session_sandbox_service(ctx)?;
        let config = service
            .config_for_session(session_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| {
                CommandError::bad_request("Session sandbox is not configured for this session")
            })?;
        let tool_context = service.tool_context(session_id);

        match self.action {
            SessionSandboxAction::Pause => {
                let state = pause_session_sandbox(&tool_context, &config)
                    .await
                    .map_err(q::map_tool_error)?;
                Ok(match state {
                    Some(state) => manage_response_from_state(self.action, &state),
                    None => ManageSessionSandboxResponse {
                        action: self.action,
                        exists: false,
                        deleted: false,
                        provider: Some(config.provider),
                        session_status: None,
                        external_id: None,
                        display_name: None,
                        workspace_path: None,
                    },
                })
            }
            SessionSandboxAction::Resume => {
                let state = ensure_session_sandbox_running(&tool_context, &config)
                    .await
                    .map_err(q::map_tool_error)?;
                Ok(manage_response_from_state(self.action, &state))
            }
            SessionSandboxAction::Delete => {
                let deleted = delete_session_sandbox(&tool_context, &config)
                    .await
                    .map_err(q::map_tool_error)?;
                Ok(ManageSessionSandboxResponse {
                    action: self.action,
                    exists: false,
                    deleted,
                    provider: Some(config.provider),
                    session_status: None,
                    external_id: None,
                    display_name: None,
                    workspace_path: None,
                })
            }
        }
    }
}

inventory::submit! { CommandDescriptor::of::<ManageSessionSandbox>() }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::common::Ctx;
    use crate::domains::session_sandbox::SessionSandboxService;
    use crate::domains::sessions::SessionService;
    use crate::storage::{CreateHarnessRow, CreateSessionRow, StorageBackend};
    use everruns_core::{Caller, DEFAULT_ORG_ID, InitialFile, SessionStorageStore};
    use everruns_platform::session_sandbox::{
        SessionSandboxConfig, SessionSandboxExecRequest, SessionSandboxExecResponse,
        SessionSandboxInstance, SessionSandboxProvider, SessionSandboxReadFileResponse,
        SessionSandboxStatusResponse, SessionSandboxWriteFileResponse,
    };
    use serde_json::json;
    use std::sync::Arc;

    struct TestSessionSandboxProvider;

    inventory::submit! {
        everruns_platform::session_sandbox::SessionSandboxProviderPlugin {
            factory: || Box::new(TestSessionSandboxProvider),
        }
    }

    #[async_trait::async_trait]
    impl SessionSandboxProvider for TestSessionSandboxProvider {
        fn id(&self) -> &str {
            "test-session-sandbox"
        }

        async fn create(
            &self,
            _context: &everruns_core::ToolContext,
            _config: &SessionSandboxConfig,
        ) -> Result<SessionSandboxInstance, everruns_core::ToolExecutionResult> {
            Ok(SessionSandboxInstance {
                external_id: "sb_test".to_string(),
                display_name: Some("Test Sandbox".to_string()),
                workspace_path: Some("/workspace".to_string()),
                provider_state: json!({}),
                metadata: json!({"source": "test"}),
            })
        }

        async fn resume(
            &self,
            _context: &everruns_core::ToolContext,
            _config: &SessionSandboxConfig,
            instance: &SessionSandboxInstance,
        ) -> Result<SessionSandboxInstance, everruns_core::ToolExecutionResult> {
            Ok(instance.clone())
        }

        async fn pause(
            &self,
            _context: &everruns_core::ToolContext,
            _config: &SessionSandboxConfig,
            instance: &SessionSandboxInstance,
        ) -> Result<SessionSandboxInstance, everruns_core::ToolExecutionResult> {
            Ok(instance.clone())
        }

        async fn delete(
            &self,
            _context: &everruns_core::ToolContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
        ) -> Result<(), everruns_core::ToolExecutionResult> {
            Ok(())
        }

        async fn exec(
            &self,
            _context: &everruns_core::ToolContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
            _request: &SessionSandboxExecRequest,
        ) -> Result<SessionSandboxExecResponse, everruns_core::ToolExecutionResult> {
            Ok(SessionSandboxExecResponse {
                exit_code: 0,
                stdout: "ok".to_string(),
                stderr: String::new(),
                success: true,
                truncated: false,
                total_lines: 1,
                raw_output: Some("ok".to_string()),
                hint: None,
            })
        }

        async fn read_file(
            &self,
            _context: &everruns_core::ToolContext,
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
            _context: &everruns_core::ToolContext,
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
            _context: &everruns_core::ToolContext,
            _config: &SessionSandboxConfig,
            state: &everruns_platform::session_sandbox::SessionSandboxState,
        ) -> Result<SessionSandboxStatusResponse, everruns_core::ToolExecutionResult> {
            Ok(SessionSandboxStatusResponse {
                provider: state.provider.clone(),
                session_status: state.status,
                external_id: state.instance.external_id.clone(),
                display_name: state.instance.display_name.clone(),
                workspace_path: state.instance.workspace_path.clone(),
                metadata: json!({"source": "test"}),
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
                system_prompt: Some("test".to_string()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::to_value(Vec::<InitialFile>::new()).unwrap(),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in: false,
            },
        )
        .await
        .unwrap()
        .id
    }

    async fn create_test_session(db: &Arc<StorageBackend>) -> everruns_core::typed_id::SessionId {
        let harness_id = create_test_harness(db.as_ref()).await;
        db.set_harness_capabilities(
            harness_id.uuid(),
            vec![(
                "session_sandbox".to_string(),
                0,
                json!({
                    "provider": "test-session-sandbox",
                    "auto_start": true,
                    "idle_pause_after_seconds": 60
                }),
            )],
        )
        .await
        .unwrap();

        db.create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: Some(harness_id),
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some("test".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: json!([]),
            tools: json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: json!([]),
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
        .unwrap()
        .id
    }

    fn test_ctx(db: Arc<StorageBackend>, sandbox_service: Arc<SessionSandboxService>) -> Ctx {
        Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db.clone(), None)
            .with_session_service(Arc::new(SessionService::new(db)))
            .with_session_sandbox_service(sandbox_service)
    }

    #[tokio::test]
    async fn get_session_sandbox_reports_unstarted_configured_state() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_test_session(&db).await;
        let service = Arc::new(SessionSandboxService::new(
            db.clone(),
            test_storage_store(&db),
            None,
        ));
        let ctx = test_ctx(db, service);

        let response = GetSessionSandbox {
            session_id: session_id.to_string(),
        }
        .execute(&ctx)
        .await
        .unwrap();

        assert!(response.configured);
        assert!(!response.exists);
        assert_eq!(response.provider.as_deref(), Some("test-session-sandbox"));
    }

    #[tokio::test]
    async fn manage_session_sandbox_resume_then_delete() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_test_session(&db).await;
        let service = Arc::new(SessionSandboxService::new(
            db.clone(),
            test_storage_store(&db),
            None,
        ));
        let ctx = test_ctx(db, service);

        let resumed = ManageSessionSandbox {
            session_id: session_id.to_string(),
            action: SessionSandboxAction::Resume,
        }
        .execute(&ctx)
        .await
        .unwrap();
        assert!(resumed.exists);
        assert_eq!(
            resumed.session_status,
            Some(SessionSandboxStatusValue::Running)
        );
        assert_eq!(resumed.external_id.as_deref(), Some("sb_test"));

        let deleted = ManageSessionSandbox {
            session_id: session_id.to_string(),
            action: SessionSandboxAction::Delete,
        }
        .execute(&ctx)
        .await
        .unwrap();
        assert!(!deleted.exists);
        assert!(deleted.deleted);
    }
}
