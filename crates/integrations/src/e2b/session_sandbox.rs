//! E2B implementation of the provider-neutral managed Session Sandbox.

use std::time::Duration;

use everruns_contracts::runtime::SessionFile;
use everruns_contracts::runtime::exec_tool_result::ExecToolResultPayload;
use everruns_contracts::session_sandbox::{
    SessionSandboxConfig, SessionSandboxContext, SessionSandboxExecRequest,
    SessionSandboxExecResponse, SessionSandboxInstance, SessionSandboxLease,
    SessionSandboxProvider, SessionSandboxReadFileResponse, SessionSandboxState,
    SessionSandboxStatus, SessionSandboxStatusResponse, SessionSandboxWriteFileResponse,
};
use everruns_contracts::tools::ToolExecutionResult;
use serde_json::{Value, json};
use tracing::warn;

use super::client::E2BClient;
use super::state::{
    E2B_SANDBOX_LEASE_DURATION_SECONDS, E2BSandboxDetail, SandboxState, build_state,
};
use super::{E2B_DEFAULT_TEMPLATE, E2B_DEFAULT_TIMEOUT_SECS, E2B_DEFAULT_WORKSPACE_PATH};

const MAX_READ_FILE_BYTES: usize = 5 * 1024 * 1024;
const LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5 * 60);
const ENVD_ACCESS_TOKEN_SECRET: &str = "e2b_sandbox:__managed_primary_envd_token";

pub struct E2BSessionSandboxProvider;

fn is_not_found(error: &str) -> bool {
    error.contains("404 Not Found") || error.contains("(404)") || error.contains("404:")
}

async fn provider_state(
    context: &dyn SessionSandboxContext,
    instance: &SessionSandboxInstance,
) -> Result<SandboxState, ToolExecutionResult> {
    let mut state: SandboxState =
        serde_json::from_value(instance.provider_state.clone()).map_err(|error| {
            ToolExecutionResult::internal_error_msg(format!("Corrupt E2B sandbox state: {error}"))
        })?;
    state.envd_access_token = context
        .get_provider_secret(ENVD_ACCESS_TOKEN_SECRET)
        .await?;
    Ok(state)
}

fn provider_config_string<'a>(config: &'a SessionSandboxConfig, key: &str) -> Option<&'a str> {
    config
        .provider_config
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn timeout_seconds(config: &SessionSandboxConfig) -> Result<u64, ToolExecutionResult> {
    let timeout = config
        .provider_config
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(E2B_DEFAULT_TIMEOUT_SECS);
    if (1..=86_400).contains(&timeout) {
        Ok(timeout)
    } else {
        Err(ToolExecutionResult::tool_error(
            "session_sandbox E2B timeout_seconds must be between 1 and 86400",
        ))
    }
}

async fn client(
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
) -> Result<E2BClient, ToolExecutionResult> {
    let key = context
        .sandbox_connection_token("e2b", &config.credential)
        .await?
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| ToolExecutionResult::connection_required("e2b"))?;
    Ok(E2BClient::new(key))
}

async fn refresh_lease(
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
    instance: &SessionSandboxInstance,
) -> Result<(), ToolExecutionResult> {
    context
        .refresh_lease(SessionSandboxLease {
            provider: "e2b".to_string(),
            external_id: instance.external_id.clone(),
            display_name: instance.display_name.clone(),
            duration_seconds: E2B_SANDBOX_LEASE_DURATION_SECONDS,
            credential: config.credential.clone(),
            metadata: json!({
                "workspace_path": instance.workspace_path,
                "managed": true,
            }),
        })
        .await
}

async fn instance_from_detail(
    context: &dyn SessionSandboxContext,
    mut detail: E2BSandboxDetail,
    timeout: u64,
    workspace_path: String,
    display_name: Option<String>,
) -> Result<SessionSandboxInstance, ToolExecutionResult> {
    // THREAT[TM-E2B-002]: the envd token grants direct process/file access.
    // Keep it in encrypted session-secret storage, never provider_state.
    if let Some(token) = detail.envd_access_token.take() {
        context
            .set_provider_secret(ENVD_ACCESS_TOKEN_SECRET, &token)
            .await?;
    }
    let mut state = build_state(&detail, timeout);
    state.workspace_path = workspace_path.clone();
    let provider_state = serde_json::to_value(&state).map_err(|error| {
        ToolExecutionResult::internal_error_msg(format!(
            "Failed to save E2B sandbox state: {error}"
        ))
    })?;
    Ok(SessionSandboxInstance {
        external_id: detail.sandbox_id,
        display_name,
        workspace_path: Some(workspace_path),
        provider_state,
        metadata: json!({ "remote_state": detail.state }),
    })
}

#[async_trait::async_trait]
impl SessionSandboxProvider for E2BSessionSandboxProvider {
    fn id(&self) -> &str {
        "e2b"
    }

    async fn create(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let client = client(context, config).await?;
        let timeout = timeout_seconds(config)?;
        let template = provider_config_string(config, "template").unwrap_or(E2B_DEFAULT_TEMPLATE);
        let workspace_path = provider_config_string(config, "workspace_path")
            .unwrap_or(E2B_DEFAULT_WORKSPACE_PATH)
            .to_string();
        let display_name = provider_config_string(config, "title")
            .map(str::to_string)
            .or_else(|| Some(format!("Session Sandbox {}", context.session_id())));
        let mut labels = context.resource_labels().await;
        if let Some(name) = &display_name {
            labels.insert("everruns.title".to_string(), json!(name));
        }
        let created = client
            .create_sandbox(template, timeout, Value::Object(labels), json!({}))
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        let detail = super::tools::detail_from_create(created);
        let instance =
            instance_from_detail(context, detail, timeout, workspace_path, display_name).await?;
        refresh_lease(context, config, &instance).await?;
        Ok(instance)
    }

    async fn resume(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let client = client(context, config).await?;
        let timeout = timeout_seconds(config)?;
        let detail = match client.get_sandbox(&instance.external_id).await {
            Ok(detail) if detail.state.eq_ignore_ascii_case("running") => detail,
            Ok(_) => super::tools::detail_from_create(
                client
                    .resume_sandbox(&instance.external_id, timeout)
                    .await
                    .map_err(ToolExecutionResult::tool_error)?,
            ),
            Err(error) => return Err(ToolExecutionResult::tool_error(error)),
        };
        let resumed = instance_from_detail(
            context,
            detail,
            timeout,
            instance
                .workspace_path
                .clone()
                .unwrap_or_else(|| E2B_DEFAULT_WORKSPACE_PATH.to_string()),
            instance.display_name.clone(),
        )
        .await?;
        refresh_lease(context, config, &resumed).await?;
        Ok(resumed)
    }

    async fn pause(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        client(context, config)
            .await?
            .pause_sandbox(&instance.external_id)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        refresh_lease(context, config, instance).await?;
        Ok(SessionSandboxInstance {
            metadata: json!({ "remote_state": "paused" }),
            ..instance.clone()
        })
    }

    async fn delete(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<(), ToolExecutionResult> {
        match client(context, config)
            .await?
            .delete_sandbox(&instance.external_id)
            .await
        {
            Ok(()) => {}
            Err(error) if is_not_found(&error) => {}
            Err(error) => return Err(ToolExecutionResult::tool_error(error)),
        }
        context.release_lease("e2b", &instance.external_id).await?;
        context
            .delete_provider_secret(ENVD_ACCESS_TOKEN_SECRET)
            .await
    }

    async fn exec(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        request: &SessionSandboxExecRequest,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        let client = client(context, config).await?;
        let state = provider_state(context, instance).await?;
        let heartbeat_context = context.clone_context();
        let heartbeat_config = config.clone();
        let heartbeat_instance = instance.clone();
        let heartbeat = tokio::spawn(async move {
            loop {
                tokio::time::sleep(LEASE_HEARTBEAT_INTERVAL).await;
                if let Err(error) = refresh_lease(
                    heartbeat_context.as_ref(),
                    &heartbeat_config,
                    &heartbeat_instance,
                )
                .await
                {
                    warn!(?error, "E2B Session Sandbox heartbeat failed");
                }
            }
        });
        let result = client
            .exec(
                &state,
                &request.command,
                request.cwd.as_deref(),
                request.timeout_ms,
            )
            .await;
        heartbeat.abort();
        let result = result.map_err(ToolExecutionResult::tool_error)?;
        refresh_lease(context, config, instance).await?;
        let payload = ExecToolResultPayload::new(
            &result.stdout,
            &result.stderr,
            result.exit_code,
            &request.output_mode,
        );
        Ok(SessionSandboxExecResponse {
            exit_code: payload.exit_code,
            stdout: payload.stdout,
            stderr: payload.stderr,
            success: payload.success,
            truncated: payload.truncated,
            total_lines: payload.total_lines,
            raw_output: Some(payload.raw_output),
            hint: result.error,
        })
    }

    async fn read_file(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
    ) -> Result<SessionSandboxReadFileResponse, ToolExecutionResult> {
        let client = client(context, config).await?;
        let state = provider_state(context, instance).await?;
        let bytes = client
            .read_file_bytes(&state, path)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        if bytes.len() > MAX_READ_FILE_BYTES {
            return Err(ToolExecutionResult::tool_error(format!(
                "E2B file is {} bytes; the limit is {MAX_READ_FILE_BYTES}",
                bytes.len()
            )));
        }
        refresh_lease(context, config, instance).await?;
        let (content, encoding) = SessionFile::encode_content(&bytes);
        Ok(SessionSandboxReadFileResponse {
            path: path.to_string(),
            content,
            encoding,
        })
    }

    async fn write_file(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
        content: &[u8],
    ) -> Result<SessionSandboxWriteFileResponse, ToolExecutionResult> {
        let client = client(context, config).await?;
        let state = provider_state(context, instance).await?;
        client
            .write_file_bytes(&state, path, content)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        refresh_lease(context, config, instance).await?;
        Ok(SessionSandboxWriteFileResponse {
            path: path.to_string(),
            bytes_written: content.len(),
        })
    }

    async fn status(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        state: &SessionSandboxState,
    ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult> {
        let detail = match client(context, config)
            .await?
            .get_sandbox(&state.instance.external_id)
            .await
        {
            Ok(detail) => detail,
            Err(error) if is_not_found(&error) => {
                return Ok(SessionSandboxStatusResponse {
                    provider: "e2b".to_string(),
                    session_status: SessionSandboxStatus::Lost,
                    external_id: state.instance.external_id.clone(),
                    display_name: state.instance.display_name.clone(),
                    workspace_path: state.instance.workspace_path.clone(),
                    metadata: json!({ "remote_state": "lost" }),
                });
            }
            Err(error) => return Err(ToolExecutionResult::tool_error(error)),
        };
        let session_status = if detail.state.eq_ignore_ascii_case("running") {
            SessionSandboxStatus::Running
        } else {
            SessionSandboxStatus::Paused
        };
        Ok(SessionSandboxStatusResponse {
            provider: "e2b".to_string(),
            session_status,
            external_id: state.instance.external_id.clone(),
            display_name: state.instance.display_name.clone(),
            workspace_path: state.instance.workspace_path.clone(),
            metadata: json!({ "remote_state": detail.state }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use async_trait::async_trait;
    use everruns_contracts::session_sandbox::create_session_sandbox_provider;
    use everruns_contracts::typed_id::SessionId;
    use tokio::sync::Mutex;

    #[derive(Clone)]
    struct RecordingContext {
        session_id: SessionId,
        secret: Arc<Mutex<Option<String>>>,
    }

    #[async_trait]
    impl SessionSandboxContext for RecordingContext {
        fn session_id(&self) -> SessionId {
            self.session_id
        }

        fn clone_context(&self) -> Arc<dyn SessionSandboxContext> {
            Arc::new(self.clone())
        }

        async fn connection_token(&self, _: &str) -> Result<Option<String>, ToolExecutionResult> {
            Ok(None)
        }

        async fn sandbox_connection_token(
            &self,
            _: &str,
            _: &everruns_contracts::session_sandbox::SessionSandboxCredential,
        ) -> Result<Option<String>, ToolExecutionResult> {
            Ok(Some("api-key".to_string()))
        }

        async fn resource_labels(&self) -> serde_json::Map<String, Value> {
            serde_json::Map::new()
        }

        async fn get_provider_secret(
            &self,
            _: &str,
        ) -> Result<Option<String>, ToolExecutionResult> {
            Ok(self.secret.lock().await.clone())
        }

        async fn set_provider_secret(
            &self,
            _: &str,
            value: &str,
        ) -> Result<(), ToolExecutionResult> {
            *self.secret.lock().await = Some(value.to_string());
            Ok(())
        }

        async fn delete_provider_secret(&self, _: &str) -> Result<(), ToolExecutionResult> {
            *self.secret.lock().await = None;
            Ok(())
        }

        async fn refresh_lease(&self, _: SessionSandboxLease) -> Result<(), ToolExecutionResult> {
            Ok(())
        }

        async fn release_lease(&self, _: &str, _: &str) -> Result<(), ToolExecutionResult> {
            Ok(())
        }
    }

    #[test]
    fn provider_is_registered() {
        assert!(create_session_sandbox_provider("e2b").is_some());
    }

    #[test]
    fn timeout_is_bounded() {
        let mut config = SessionSandboxConfig::default();
        config.provider_config = json!({ "timeout_seconds": 0 });
        assert!(timeout_seconds(&config).is_err());
        config.provider_config = json!({ "timeout_seconds": 3600 });
        assert_eq!(timeout_seconds(&config).unwrap(), 3600);
    }

    #[tokio::test]
    async fn envd_token_is_kept_out_of_provider_state() {
        let context = RecordingContext {
            session_id: SessionId::new(),
            secret: Arc::new(Mutex::new(None)),
        };
        let instance = instance_from_detail(
            &context,
            E2BSandboxDetail {
                client_id: "client".to_string(),
                cpu_count: 2,
                disk_size_mb: 1024,
                end_at: String::new(),
                envd_version: "1".to_string(),
                memory_mb: 512,
                sandbox_id: "sandbox".to_string(),
                started_at: "2026-10-07T00:00:00Z".to_string(),
                state: "running".to_string(),
                template_id: "base".to_string(),
                alias: None,
                domain: Some("e2b.app".to_string()),
                envd_access_token: Some("secret-envd-token".to_string()),
                metadata: json!({}),
            },
            3600,
            "/home/user".to_string(),
            None,
        )
        .await
        .unwrap();

        assert_eq!(
            context.secret.lock().await.as_deref(),
            Some("secret-envd-token")
        );
        assert!(
            !instance
                .provider_state
                .to_string()
                .contains("secret-envd-token")
        );
        assert!(
            instance.provider_state["envd_access_token"].is_null(),
            "the non-secret sandbox snapshot must not carry envd authority"
        );
    }
}
