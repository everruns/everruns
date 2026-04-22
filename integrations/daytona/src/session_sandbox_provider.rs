//! Daytona implementation of the provider-neutral session_sandbox contract.

use everruns_core::exec_tool_result::ExecToolResultPayload;
use everruns_core::session_sandbox::{
    SessionSandboxConfig, SessionSandboxExecRequest, SessionSandboxExecResponse,
    SessionSandboxInstance, SessionSandboxProvider, SessionSandboxReadFileResponse,
    SessionSandboxState, SessionSandboxStatus, SessionSandboxStatusResponse,
    SessionSandboxWriteFileResponse,
};
use everruns_core::tools::ToolExecutionResult;
use everruns_core::traits::ToolContext;
use serde_json::json;
use std::time::Duration;
use tracing::{debug, warn};

use crate::client::DaytonaClient;
use crate::naming::create_sandbox_with_unique_name;
use crate::state::{
    SandboxInfo, SandboxState, get_api_key, release_sandbox_lease, touch_sandbox_lease,
};
use crate::{
    AUTO_ARCHIVE_INTERVAL_MINUTES, AUTO_DELETE_INTERVAL_MINUTES, AUTO_STOP_INTERVAL_MINUTES,
    DAYTONA_WORKSPACE_PATH, EXEC_TIMEOUT_MS, LEASE_HEARTBEAT_INTERVAL,
};

pub struct DaytonaSessionSandboxProvider;

const DAYTONA_STATE_POLL_INTERVAL: Duration = Duration::from_secs(1);
const DAYTONA_STATE_POLL_ATTEMPTS: usize = 20;

fn is_transition_state(state: &str) -> bool {
    matches!(state, "starting" | "stopping")
}

fn is_state_change_in_progress(err: &str) -> bool {
    err.contains("409 Conflict") && err.contains("state change in progress")
}

async fn wait_for_sandbox_state(
    client: &DaytonaClient,
    sandbox_id: &str,
    desired_state: &str,
) -> Result<SandboxInfo, ToolExecutionResult> {
    let mut last_state = None;

    for _ in 0..DAYTONA_STATE_POLL_ATTEMPTS {
        let info = client
            .get_sandbox(sandbox_id)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        if info.state == desired_state {
            return Ok(info);
        }
        last_state = Some(info.state);
        tokio::time::sleep(DAYTONA_STATE_POLL_INTERVAL).await;
    }

    Err(ToolExecutionResult::tool_error(format!(
        "Daytona sandbox did not reach '{desired_state}' state (last state: {})",
        last_state.unwrap_or_else(|| "unknown".to_string())
    )))
}

async fn ensure_sandbox_started(
    client: &DaytonaClient,
    sandbox_id: &str,
) -> Result<SandboxInfo, ToolExecutionResult> {
    let mut last_state = None;

    for _ in 0..DAYTONA_STATE_POLL_ATTEMPTS {
        let info = client
            .get_sandbox(sandbox_id)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        last_state = Some(info.state.clone());

        if info.state == "started" {
            return Ok(info);
        }
        if is_transition_state(&info.state) {
            tokio::time::sleep(DAYTONA_STATE_POLL_INTERVAL).await;
            continue;
        }

        match client.start_sandbox(sandbox_id).await {
            Ok(()) => {
                if let Err(err) = client.wait_for_ready(sandbox_id).await {
                    warn!(
                        sandbox_id = %sandbox_id,
                        error = %err,
                        "Daytona sandbox readiness check failed after start"
                    );
                }
            }
            Err(err) if is_state_change_in_progress(&err) => {}
            Err(err) => return Err(ToolExecutionResult::tool_error(err)),
        }

        tokio::time::sleep(DAYTONA_STATE_POLL_INTERVAL).await;
    }

    Err(ToolExecutionResult::tool_error(format!(
        "Daytona sandbox '{sandbox_id}' did not reach 'started' state (last state: {})",
        last_state.unwrap_or_else(|| "unknown".to_string())
    )))
}

async fn delete_sandbox_with_retry(
    client: &DaytonaClient,
    sandbox_id: &str,
) -> Result<(), ToolExecutionResult> {
    for _ in 0..DAYTONA_STATE_POLL_ATTEMPTS {
        match client.delete_sandbox(sandbox_id).await {
            Ok(()) => return Ok(()),
            Err(err) if is_state_change_in_progress(&err) => {
                tokio::time::sleep(DAYTONA_STATE_POLL_INTERVAL).await;
            }
            Err(err) => return Err(ToolExecutionResult::tool_error(err)),
        }
    }

    Err(ToolExecutionResult::tool_error(format!(
        "Daytona sandbox delete remained in transition for sandbox {sandbox_id}"
    )))
}

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
        let requested_name = config
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

        let (sandbox_info, sandbox_name) = create_sandbox_with_unique_name(
            &client,
            &requested_name,
            json!({
                "name": requested_name,
                "snapshot": snapshot,
                "autoStopInterval": auto_stop,
                "autoArchiveInterval": AUTO_ARCHIVE_INTERVAL_MINUTES,
                "autoDeleteInterval": AUTO_DELETE_INTERVAL_MINUTES,
                "labels": labels,
            }),
        )
        .await
        .map_err(ToolExecutionResult::tool_error)?;
        let canonical_name = sandbox_name.canonical_name;

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
        touch_sandbox_lease(context, &state, Some(canonical_name.clone())).await?;

        Ok(SessionSandboxInstance {
            external_id: sandbox_info.id,
            display_name: Some(canonical_name),
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
        let info = ensure_sandbox_started(&client, &instance.external_id).await?;

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
        let info = wait_for_sandbox_state(&client, &instance.external_id, "stopped").await?;

        Ok(SessionSandboxInstance {
            metadata: json!({ "remote_state": info.state }),
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
        delete_sandbox_with_retry(&client, &instance.external_id).await?;
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
            hint: exit_code_hint(payload.exit_code).map(ToString::to_string),
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
