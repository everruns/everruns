use super::queries as q;
use super::types::{
    ManageSessionSandboxResponse, SessionRepositoriesResponse, SessionSandboxAction,
    SessionSandboxStatusValue,
};
use crate::domains::common::*;
use everruns_capabilities::session_sandbox::{
    delete_session_sandbox, ensure_session_sandbox_running, pause_session_sandbox,
};
use serde::Deserialize;
use serde_json::json;
use utoipa::ToSchema;

fn manage_response_from_state(
    action: SessionSandboxAction,
    state: &everruns_capabilities::session_sandbox::SessionSandboxState,
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

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ManageSessionSandbox {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub action: SessionSandboxAction,
}

#[command(
    name = "manage_session_sandbox",
    category = "session_sandbox",
    description = "Pause, resume, or delete the managed sandbox for a session.",
    method = "POST",
    path = "/v1/sessions/{session_id}/sandbox",
    policy = crate::domains::sessions::SESSION_MANAGE,
    cli = CliRoute::new(&["sessions", "sandbox"], "manage").with_examples(&[CliExample::new("Pause a session's sandbox to stop paying for idle compute", "everruns sessions sandbox manage --session-id session_01h9 --action pause --reason 'Idle overnight'")]),
)]
impl Command for ManageSessionSandbox {
    type Output = ManageSessionSandboxResponse;

    async fn execute(self, ctx: &Ctx) -> Result<ManageSessionSandboxResponse, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session_access(ctx, session_id).await?;

        let service = q::session_sandbox_service(ctx)?;
        let config = service
            .config_for_session(session_id)
            .await?
            .ok_or_else(|| {
                CommandError::bad_request("Session sandbox is not configured for this session")
            })?;
        let tool_context = service.tool_context(session_id).await;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::common::Ctx;
    use crate::domains::session_sandbox::SessionSandboxService;
    use crate::domains::sessions::SessionService;
    use crate::storage::{CreateHarnessRow, CreateSessionRow, StorageBackend};
    use everruns_capabilities::session_sandbox::{
        SessionSandboxConfig, SessionSandboxExecRequest, SessionSandboxExecResponse,
        SessionSandboxInstance, SessionSandboxProvider, SessionSandboxReadFileResponse,
        SessionSandboxStatusResponse, SessionSandboxWriteFileResponse,
    };
    use everruns_core::{
        Caller, DEFAULT_ORG_ID, InitialFile, session_services::SessionStorageStore,
    };
    use serde_json::json;
    use std::sync::Arc;

    struct TestSessionSandboxProvider;

    inventory::submit! {
        everruns_capabilities::session_sandbox::SessionSandboxProviderPlugin {
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
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
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
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
            instance: &SessionSandboxInstance,
        ) -> Result<SessionSandboxInstance, everruns_core::ToolExecutionResult> {
            Ok(instance.clone())
        }

        async fn pause(
            &self,
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
            instance: &SessionSandboxInstance,
        ) -> Result<SessionSandboxInstance, everruns_core::ToolExecutionResult> {
            Ok(instance.clone())
        }

        async fn delete(
            &self,
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
        ) -> Result<(), everruns_core::ToolExecutionResult> {
            Ok(())
        }

        async fn exec(
            &self,
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
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
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
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
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
            _instance: &SessionSandboxInstance,
            path: &str,
            content: &[u8],
        ) -> Result<SessionSandboxWriteFileResponse, everruns_core::ToolExecutionResult> {
            Ok(SessionSandboxWriteFileResponse {
                path: path.to_string(),
                bytes_written: content.len(),
            })
        }

        async fn status(
            &self,
            _context: &dyn everruns_contracts::session_sandbox::SessionSandboxContext,
            _config: &SessionSandboxConfig,
            state: &everruns_capabilities::session_sandbox::SessionSandboxState,
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
        // Sandbox state carries secrets, so the store must encrypt.
        let encryption = crate::storage::EncryptionService::new(
            "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            &[],
        )
        .expect("test encryption key");
        Arc::new(crate::storage::DbSessionStorageStore::new(
            db.database().clone(),
            encryption,
        ))
    }

    async fn create_test_harness(db: &StorageBackend) -> everruns_contracts::typed_id::HarnessId {
        db.create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                localizations: Default::default(),
                name: "sandbox-harness".to_string(),
                display_name: Some("Sandbox Harness".to_string()),
                icon: None,
                description: Some("test".to_string()),
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
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

    async fn create_test_session(
        db: &Arc<StorageBackend>,
    ) -> everruns_contracts::typed_id::SessionId {
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
            mcp_creator: None,
            playground_user_id: None,
            source: crate::domains::sessions::record::SessionSource::Api,
            workspace_id: None,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            channel_id: None,
            trigger_id: None,
            harness_id: Some(harness_id),
            agent_id: None,
            agent_revision: None,
            virtual_user_id: None,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
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
    async fn manage_session_sandbox_resume_then_delete() {
        let db = Arc::new(StorageBackend::test_database());
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

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetSessionRepositories {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

/// API name of a stored sandbox state; the fleet view uses the same names.
fn sandbox_state_name(observed: &str) -> &'static str {
    match observed {
        "ready" => "running",
        "paused" => "paused",
        "lost" => "lost",
        "provisioning" => "starting",
        "failed" => "failed",
        "absent" => "not_started",
        _ => "deleted",
    }
}

#[command(
    name = "get_session_repositories",
    category = "session_sandbox",
    description = "List the repositories checked out in a session's sandbox and what the session changed in each.",
    method = "GET",
    path = "/v1/sessions/{session_id}/repositories",
    policy = crate::domains::sessions::SESSION_VIEW,
    positional = "session_id",
    cli = CliRoute::new(&["sessions", "repositories"], "get").with_args(&[CliArg::new("session_id").at(1)]).with_examples(&[CliExample::new("See what a coding session changed in each repository", "everruns sessions repositories get session_01h9")]),
)]
impl Command for GetSessionRepositories {
    type Output = SessionRepositoriesResponse;

    async fn execute(self, ctx: &Ctx) -> Result<SessionRepositoriesResponse, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session_access(ctx, session_id).await?;

        let empty = |sandbox_state: &str| SessionRepositoriesResponse {
            sandbox_state: sandbox_state.to_string(),
            work_branch: None,
            repositories: Vec::new(),
        };
        let Some(capabilities) =
            crate::domains::sandbox_templates::queries::effective_session_capabilities(
                &ctx.db, session_id,
            )
            .await?
        else {
            return Err(CommandError::not_found("Session not found"));
        };
        let Some(repositories) = capabilities.iter().find(|capability| {
            capability.capability_id() == everruns_integrations::github::REPOSITORIES_CAPABILITY_ID
        }) else {
            return Ok(empty("none"));
        };
        // Decision: a page view never starts or resumes a sandbox. Only a
        // running one is read; anything else is reported as it is.
        let state = match ctx.db.get_primary_sandbox(session_id).await? {
            Some(record) => sandbox_state_name(&record.observed_state),
            None => "not_started",
        };
        if state != "running" {
            return Ok(empty(state));
        }

        let service = q::session_sandbox_service(ctx)?;
        let mut tool_context = service.tool_context(session_id).await;
        everruns_capabilities::capabilities::install_sandbox_workspace(
            &mut tool_context.extensions,
            &capabilities,
        );
        // Unguessable to the sandbox, so a checkout cannot forge a section.
        let marker = uuid::Uuid::new_v4().simple().to_string();
        let changes = everruns_integrations::github::read_repository_changes(
            &tool_context,
            repositories.config_value(),
            &marker,
        )
        .await
        .map_err(CommandError::bad_request)?;
        let mut response: SessionRepositoriesResponse = serde_json::from_value(json!({
            "sandbox_state": state,
            "work_branch": changes["work_branch"],
            "repositories": changes["repositories"],
        }))
        .map_err(|error| CommandError::internal(anyhow::anyhow!(error)))?;
        response.repositories.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(response)
    }
}

#[cfg(test)]
mod repositories_tests {
    use super::*;

    #[test]
    fn only_a_ready_sandbox_reads_as_running() {
        assert_eq!(sandbox_state_name("ready"), "running");
        for other in [
            "paused",
            "lost",
            "provisioning",
            "failed",
            "absent",
            "deleted",
        ] {
            assert_ne!(sandbox_state_name(other), "running");
        }
    }

    #[test]
    fn the_sandbox_report_maps_onto_the_response() {
        let response: SessionRepositoriesResponse = serde_json::from_value(json!({
            "sandbox_state": "running",
            "work_branch": "everruns/bdaf0ce2",
            "repositories": [{
                "repo": "acme/app", "path": "/workspace/app", "access": "write",
                "branch": "everruns/bdaf0ce2", "on_work_branch": true, "head_sha": "abc",
                "base_branch": "main", "uncommitted_files": 1,
                "commits_ahead_of_base": 1, "commits_not_pushed": null,
                "has_agents_md": false,
                "files": [{"path": "a.rs", "additions": 2, "deletions": null, "committed": true}],
                "diff": "diff --git", "diff_truncated": false
            }]
        }))
        .unwrap();
        assert_eq!(response.repositories[0].files[0].additions, Some(2));
        assert_eq!(response.repositories[0].commits_not_pushed, None);
    }
}
