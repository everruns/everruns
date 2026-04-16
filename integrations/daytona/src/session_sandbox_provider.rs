//! Daytona implementation of the provider-neutral session_sandbox contract.

use everruns_core::session_sandbox::{
    SessionSandboxConfig, SessionSandboxExecRequest, SessionSandboxExecResponse,
    SessionSandboxInstance, SessionSandboxProvider, SessionSandboxReadFileResponse,
    SessionSandboxState, SessionSandboxStatus, SessionSandboxStatusResponse,
    SessionSandboxWriteFileResponse,
};
use everruns_core::tools::ToolExecutionResult;
use everruns_core::traits::ToolContext;
use serde_json::json;
use tracing::{debug, warn};

use crate::client::DaytonaClient;
use crate::state::{SandboxState, get_api_key, release_sandbox_lease, touch_sandbox_lease};
use crate::{
    AUTO_ARCHIVE_INTERVAL_MINUTES, AUTO_DELETE_INTERVAL_MINUTES, AUTO_STOP_INTERVAL_MINUTES,
    DAYTONA_WORKSPACE_PATH, EXEC_TIMEOUT_MS, LEASE_HEARTBEAT_INTERVAL,
};

pub struct DaytonaSessionSandboxProvider;

#[async_trait::async_trait]
impl SessionSandboxProvider for DaytonaSessionSandboxProvider {
    fn id(&self) -> &str {
        "daytona"
    }

    async fn create(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let api_key = get_api_key(context).await?;
        let client = build_client(api_key, config);
        let title = config
            .provider_config
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("Session Sandbox {}", context.session_id));
        let workspace_path = workspace_path(config);
        let auto_stop = auto_stop_minutes(config)?;
        let snapshot = snapshot_name(config)?;

        let mut labels = serde_json::Map::new();
        labels.insert("everruns".to_string(), json!("true"));
        labels.insert(
            "everruns.session_id".to_string(),
            json!(context.session_id.to_string()),
        );
        if let Some(session_store) = &context.session_store
            && let Ok(Some(session)) = session_store.get_session(context.session_id).await
        {
            labels.insert(
                "everruns.harness_id".to_string(),
                json!(session.harness_id.to_string()),
            );
            labels.insert(
                "everruns.org_id".to_string(),
                json!(session.organization_id),
            );
            if let Some(agent_id) = session.agent_id {
                labels.insert("everruns.agent_id".to_string(), json!(agent_id.to_string()));
            }
        }

        let sandbox_info = client
            .create_sandbox(json!({
                "name": title,
                "snapshot": snapshot,
                "autoStopInterval": auto_stop,
                "autoArchiveInterval": AUTO_ARCHIVE_INTERVAL_MINUTES,
                "autoDeleteInterval": AUTO_DELETE_INTERVAL_MINUTES,
                "labels": labels,
            }))
            .await
            .map_err(ToolExecutionResult::tool_error)?;

        if let Err(err) = client.wait_for_ready(&sandbox_info.id).await {
            warn!(
                sandbox_id = %sandbox_info.id,
                error = %err,
                "Daytona sandbox readiness check failed"
            );
        }

        if let Err(err) = client
            .exec(
                &sandbox_info.id,
                &format!("mkdir -p -- {}", shell_escape(&workspace_path)),
                None,
                None,
                |_| {},
            )
            .await
        {
            warn!(
                sandbox_id = %sandbox_info.id,
                error = %err,
                "Failed to create Daytona workspace directory"
            );
        }

        let state = SandboxState {
            sandbox_id: sandbox_info.id.clone(),
            workspace_path: workspace_path.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        touch_sandbox_lease(context, &state, sandbox_info.name.clone()).await?;

        Ok(SessionSandboxInstance {
            external_id: sandbox_info.id,
            display_name: sandbox_info.name.or(Some(title)),
            workspace_path: Some(workspace_path),
            provider_state: json!({}),
            metadata: json!({ "remote_state": sandbox_info.state }),
        })
    }

    async fn resume(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let api_key = get_api_key(context).await?;
        let client = build_client(api_key, config);
        let info = client
            .get_sandbox(&instance.external_id)
            .await
            .map_err(ToolExecutionResult::tool_error)?;

        if info.state != "started" {
            client
                .start_sandbox(&instance.external_id)
                .await
                .map_err(ToolExecutionResult::tool_error)?;
            client
                .wait_for_ready(&instance.external_id)
                .await
                .map_err(ToolExecutionResult::tool_error)?;
        }

        let workspace_path = instance
            .workspace_path
            .clone()
            .unwrap_or_else(|| workspace_path(config));
        let state = SandboxState {
            sandbox_id: instance.external_id.clone(),
            workspace_path: workspace_path.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        touch_sandbox_lease(context, &state, instance.display_name.clone()).await?;

        Ok(SessionSandboxInstance {
            external_id: instance.external_id.clone(),
            display_name: instance.display_name.clone().or(info.name.clone()),
            workspace_path: Some(workspace_path),
            provider_state: instance.provider_state.clone(),
            metadata: json!({ "remote_state": "started" }),
        })
    }

    async fn pause(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let api_key = get_api_key(context).await?;
        let client = build_client(api_key, config);
        client
            .stop_sandbox(&instance.external_id)
            .await
            .map_err(ToolExecutionResult::tool_error)?;

        Ok(SessionSandboxInstance {
            metadata: json!({ "remote_state": "stopped" }),
            ..instance.clone()
        })
    }

    async fn delete(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<(), ToolExecutionResult> {
        let api_key = get_api_key(context).await?;
        let client = build_client(api_key, config);
        client
            .delete_sandbox(&instance.external_id)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        release_sandbox_lease(context, &instance.external_id).await?;
        Ok(())
    }

    async fn exec(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        request: &SessionSandboxExecRequest,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        let api_key = get_api_key(context).await?;
        let client = build_client(api_key, config);
        let workspace_path = instance
            .workspace_path
            .clone()
            .unwrap_or_else(|| workspace_path(config));
        let lease_state = SandboxState {
            sandbox_id: instance.external_id.clone(),
            workspace_path,
            started_at: chrono::Utc::now().to_rfc3339(),
        };

        let heartbeat_ctx = context.clone();
        let heartbeat_state = lease_state.clone();
        let display_name = instance.display_name.clone();
        let heartbeat = tokio::spawn(async move {
            loop {
                if let Err(err) =
                    touch_sandbox_lease(&heartbeat_ctx, &heartbeat_state, display_name.clone())
                        .await
                {
                    warn!(error = ?err, "Daytona session sandbox heartbeat failed");
                }
                tokio::time::sleep(LEASE_HEARTBEAT_INTERVAL).await;
            }
        });

        debug!(
            sandbox_id = %instance.external_id,
            command = %request.command,
            "Executing in session-managed Daytona sandbox"
        );
        let result = client
            .exec(
                &instance.external_id,
                &request.command,
                request.cwd.as_deref(),
                Some(request.timeout_ms.unwrap_or(EXEC_TIMEOUT_MS)),
                |_| {},
            )
            .await;
        heartbeat.abort();

        let result = result.map_err(ToolExecutionResult::tool_error)?;
        touch_sandbox_lease(context, &lease_state, instance.display_name.clone()).await?;

        let clean_output = everruns_core::tool_output_sanitizer::clean_exec_output(&result.result);
        let output = if let Some(budget) =
            everruns_core::tool_output_sanitizer::output_verbosity_budget(&request.output_mode)
        {
            everruns_core::tool_output_sanitizer::priority_aware_truncate(&clean_output, budget)
        } else {
            clean_output.clone()
        };

        Ok(SessionSandboxExecResponse {
            exit_code: result.exit_code,
            output,
            raw_output: Some(clean_output),
            hint: exit_code_hint(result.exit_code).map(ToString::to_string),
        })
    }

    async fn read_file(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
    ) -> Result<SessionSandboxReadFileResponse, ToolExecutionResult> {
        let api_key = get_api_key(context).await?;
        let client = build_client(api_key, config);
        let bytes = client
            .file_download(&instance.external_id, path)
            .await
            .map_err(ToolExecutionResult::tool_error)?;

        let lease_state = SandboxState {
            sandbox_id: instance.external_id.clone(),
            workspace_path: instance
                .workspace_path
                .clone()
                .unwrap_or_else(|| workspace_path(config)),
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        touch_sandbox_lease(context, &lease_state, instance.display_name.clone()).await?;

        let (content, encoding) = everruns_core::SessionFile::encode_content(&bytes);
        Ok(SessionSandboxReadFileResponse {
            path: path.to_string(),
            content,
            encoding: encoding.to_string(),
        })
    }

    async fn write_file(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
        content: &str,
    ) -> Result<SessionSandboxWriteFileResponse, ToolExecutionResult> {
        let api_key = get_api_key(context).await?;
        let client = build_client(api_key, config);
        client
            .file_upload(&instance.external_id, path, content.as_bytes())
            .await
            .map_err(ToolExecutionResult::tool_error)?;

        let lease_state = SandboxState {
            sandbox_id: instance.external_id.clone(),
            workspace_path: instance
                .workspace_path
                .clone()
                .unwrap_or_else(|| workspace_path(config)),
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        touch_sandbox_lease(context, &lease_state, instance.display_name.clone()).await?;

        Ok(SessionSandboxWriteFileResponse {
            path: path.to_string(),
            bytes_written: content.len(),
        })
    }

    async fn status(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        state: &SessionSandboxState,
    ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult> {
        let api_key = get_api_key(context).await?;
        let client = build_client(api_key, config);
        let info = client
            .get_sandbox(&state.instance.external_id)
            .await
            .map_err(ToolExecutionResult::tool_error)?;

        Ok(SessionSandboxStatusResponse {
            provider: state.provider.clone(),
            session_status: match info.state.as_str() {
                "started" => SessionSandboxStatus::Running,
                _ => SessionSandboxStatus::Paused,
            },
            external_id: state.instance.external_id.clone(),
            display_name: state.instance.display_name.clone().or(info.name.clone()),
            workspace_path: state.instance.workspace_path.clone(),
            metadata: json!({
                "remote_state": info.state,
            }),
        })
    }
}

fn build_client(api_key: String, config: &SessionSandboxConfig) -> DaytonaClient {
    let api_base = config
        .provider_config
        .get("api_base")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .map(ToString::to_string);
    let toolbox_base = config
        .provider_config
        .get("toolbox_base")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .map(ToString::to_string);

    match (api_base, toolbox_base) {
        (Some(api_base), Some(toolbox_base)) => {
            DaytonaClient::with_base_urls(api_key, api_base, toolbox_base)
        }
        _ => DaytonaClient::new(api_key),
    }
}

fn workspace_path(config: &SessionSandboxConfig) -> String {
    config
        .provider_config
        .get("workspace_path")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(DAYTONA_WORKSPACE_PATH)
        .to_string()
}

fn auto_stop_minutes(config: &SessionSandboxConfig) -> Result<u64, ToolExecutionResult> {
    let value = config
        .provider_config
        .get("auto_stop_minutes")
        .and_then(|v| v.as_u64())
        .unwrap_or(AUTO_STOP_INTERVAL_MINUTES);
    if (1..=60).contains(&value) {
        Ok(value)
    } else {
        Err(ToolExecutionResult::tool_error(
            "session_sandbox Daytona auto_stop_minutes must be between 1 and 60",
        ))
    }
}

fn snapshot_name(config: &SessionSandboxConfig) -> Result<String, ToolExecutionResult> {
    if let Some(snapshot) = config
        .provider_config
        .get("snapshot")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
    {
        return Ok(snapshot.to_string());
    }

    let size = config
        .provider_config
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or("small");
    match size {
        "small" => Ok("daytona-small".to_string()),
        "medium" => Ok("daytona-medium".to_string()),
        "large" => Ok("daytona-large".to_string()),
        _ => Err(ToolExecutionResult::tool_error(
            "session_sandbox Daytona size must be one of: small, medium, large",
        )),
    }
}

fn exit_code_hint(exit_code: i32) -> Option<&'static str> {
    match exit_code {
        0 => None,
        137 => {
            Some("Process was killed (SIGKILL). This often means the process ran out of memory.")
        }
        139 => Some("Process crashed with a segmentation fault (SIGSEGV)."),
        141 => Some("Broken pipe (SIGPIPE). A downstream command closed the pipe early."),
        143 => Some("Process was terminated (SIGTERM)."),
        126 => Some("Command found but not executable. Check file permissions."),
        127 => Some("Command not found. Check that the tool is installed and in PATH."),
        _ if exit_code > 128 && exit_code <= 192 => Some("Process was killed by a signal."),
        _ => None,
    }
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
