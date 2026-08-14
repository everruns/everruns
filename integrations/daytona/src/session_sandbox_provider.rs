//! Daytona implementation of the provider-neutral session_sandbox contract.

use everruns_core::exec_tool_result::ExecToolResultPayload;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::ToolExecutionResult;
use everruns_platform::session_sandbox::{
    SessionSandboxConfig, SessionSandboxExecRequest, SessionSandboxExecResponse,
    SessionSandboxInstance, SessionSandboxProvider, SessionSandboxReadFileResponse,
    SessionSandboxState, SessionSandboxStatus, SessionSandboxStatusResponse,
    SessionSandboxWriteFileResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tracing::{debug, warn};

use crate::client::DaytonaClient;
use crate::naming::create_sandbox_with_unique_name;
use crate::state::{
    SandboxInfo, SandboxState, VolumeInfo, get_api_key, release_sandbox_lease, touch_sandbox_lease,
};
use crate::{
    AUTO_ARCHIVE_INTERVAL_MINUTES, AUTO_DELETE_INTERVAL_MINUTES, AUTO_STOP_INTERVAL_MINUTES,
    DAYTONA_WORKSPACE_PATH, EXEC_TIMEOUT_MS, LEASE_HEARTBEAT_INTERVAL,
};

pub struct DaytonaSessionSandboxProvider;

const DAYTONA_STATE_POLL_INTERVAL: Duration = Duration::from_secs(1);
const DAYTONA_STATE_POLL_ATTEMPTS: usize = 20;
const DAYTONA_VOLUME_POLL_ATTEMPTS: usize = 60;
const DEFAULT_RECOVERY_VOLUME_NAME: &str = "everruns-recovery";
const DEFAULT_RECOVERY_MOUNT_PATH: &str = "/mnt/everruns-recovery";
const DEFAULT_RECOVERY_RETAINED_REVISIONS: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
struct DaytonaRecoveryBinding {
    volume_id: String,
    volume_name: String,
    mount_path: String,
    subpath: String,
    retained_revisions: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    head_revision: Option<String>,
}

#[derive(Debug, Clone)]
struct DaytonaRecoverySettings {
    volume_id: Option<String>,
    volume_name: String,
    mount_path: String,
    retained_revisions: usize,
}

fn is_not_found(err: &str) -> bool {
    err.contains("404 Not Found") || err.contains("(404)") || err.contains("statusCode\":404")
}

fn is_conflict(err: &str) -> bool {
    err.contains("409 Conflict") || err.contains("(409)") || err.contains("statusCode\":409")
}

fn is_safe_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        && path != "/"
        && path
            .split('/')
            .skip(1)
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn is_valid_revision(revision: &str) -> bool {
    revision.strip_prefix("rev-").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn recovery_settings(
    config: &SessionSandboxConfig,
    workspace_path: &str,
) -> Result<Option<DaytonaRecoverySettings>, ToolExecutionResult> {
    let Some(recovery) = config
        .provider_config
        .get("recovery")
        .filter(|value| value.get("enabled").and_then(|v| v.as_bool()) == Some(true))
    else {
        return Ok(None);
    };

    if !is_safe_absolute_path(workspace_path) {
        return Err(ToolExecutionResult::tool_error(
            "session_sandbox Daytona recovery workspace_path must be a normalized absolute non-root path",
        ));
    }
    if workspace_path == DAYTONA_WORKSPACE_PATH {
        return Err(ToolExecutionResult::tool_error(format!(
            "session_sandbox Daytona recovery requires a dedicated workspace below {DAYTONA_WORKSPACE_PATH}, such as {DAYTONA_WORKSPACE_PATH}/workspace"
        )));
    }

    let mount_path = recovery
        .get("mount_path")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(DEFAULT_RECOVERY_MOUNT_PATH)
        .trim_end_matches('/')
        .to_string();
    if !is_safe_absolute_path(&mount_path) {
        return Err(ToolExecutionResult::tool_error(
            "session_sandbox Daytona recovery mount_path must be a normalized absolute non-root path",
        ));
    }
    let workspace_prefix = format!("{}/", workspace_path.trim_end_matches('/'));
    if mount_path == workspace_path || mount_path.starts_with(&workspace_prefix) {
        return Err(ToolExecutionResult::tool_error(
            "session_sandbox Daytona recovery mount_path must be outside workspace_path",
        ));
    }

    let retained_revisions = recovery
        .get("retained_revisions")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_RECOVERY_RETAINED_REVISIONS as u64);
    if !(2..=100).contains(&retained_revisions) {
        return Err(ToolExecutionResult::tool_error(
            "session_sandbox Daytona recovery retained_revisions must be between 2 and 100",
        ));
    }

    Ok(Some(DaytonaRecoverySettings {
        volume_id: recovery
            .get("volume_id")
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
            .map(ToString::to_string),
        volume_name: recovery
            .get("volume_name")
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
            .unwrap_or(DEFAULT_RECOVERY_VOLUME_NAME)
            .to_string(),
        mount_path,
        retained_revisions: retained_revisions as usize,
    }))
}

fn recovery_binding(
    instance: &SessionSandboxInstance,
) -> Result<Option<DaytonaRecoveryBinding>, ToolExecutionResult> {
    let Some(value) = instance.provider_state.get("recovery") else {
        return Ok(None);
    };
    let binding: DaytonaRecoveryBinding = serde_json::from_value(value.clone()).map_err(|err| {
        ToolExecutionResult::internal_error_msg(format!("Corrupt Daytona recovery binding: {err}"))
    })?;
    let workspace_path = instance
        .workspace_path
        .as_deref()
        .unwrap_or(DAYTONA_WORKSPACE_PATH);
    let workspace_prefix = format!("{}/", workspace_path.trim_end_matches('/'));
    // THREAT[TM-DAYTONA-009]: Persisted provider state is validated before it
    // becomes a volume mount or a path in a maintenance shell command.
    if binding.volume_id.trim().is_empty()
        || !is_safe_absolute_path(&binding.mount_path)
        || binding.mount_path == workspace_path
        || binding.mount_path.starts_with(&workspace_prefix)
        || binding.subpath.starts_with('/')
        || binding.subpath.contains("..")
        || binding.subpath.contains("//")
        || !(2..=100).contains(&binding.retained_revisions)
        || binding
            .head_revision
            .as_deref()
            .is_some_and(|revision| !is_valid_revision(revision))
    {
        return Err(ToolExecutionResult::internal_error_msg(
            "Corrupt Daytona recovery binding: invalid volume, mount, subpath, retention, or revision",
        ));
    }
    Ok(Some(binding))
}

async fn wait_for_volume_ready(
    client: &DaytonaClient,
    mut volume: VolumeInfo,
) -> Result<VolumeInfo, ToolExecutionResult> {
    for _ in 0..DAYTONA_VOLUME_POLL_ATTEMPTS {
        match volume.state.as_str() {
            "ready" => return Ok(volume),
            "error" | "deleting" | "pending_delete" => {
                return Err(ToolExecutionResult::tool_error(format!(
                    "Daytona recovery volume '{}' entered '{}' state",
                    volume.name, volume.state
                )));
            }
            _ => {
                tokio::time::sleep(DAYTONA_STATE_POLL_INTERVAL).await;
                volume = client
                    .get_volume(&volume.id)
                    .await
                    .map_err(ToolExecutionResult::tool_error)?;
            }
        }
    }

    Err(ToolExecutionResult::tool_error(format!(
        "Daytona recovery volume '{}' did not reach 'ready' state (last state: {})",
        volume.name, volume.state
    )))
}

async fn resolve_recovery_binding(
    client: &DaytonaClient,
    context: &ToolContext,
    settings: DaytonaRecoverySettings,
) -> Result<DaytonaRecoveryBinding, ToolExecutionResult> {
    let volume = if let Some(volume_id) = &settings.volume_id {
        client
            .get_volume(volume_id)
            .await
            .map_err(ToolExecutionResult::tool_error)?
    } else {
        match client.get_volume_by_name(&settings.volume_name).await {
            Ok(volume) => volume,
            Err(err) if is_not_found(&err) => {
                match client.create_volume(&settings.volume_name).await {
                    Ok(volume) => volume,
                    Err(create_err) if is_conflict(&create_err) => client
                        .get_volume_by_name(&settings.volume_name)
                        .await
                        .map_err(ToolExecutionResult::tool_error)?,
                    Err(create_err) => return Err(ToolExecutionResult::tool_error(create_err)),
                }
            }
            Err(err) => return Err(ToolExecutionResult::tool_error(err)),
        }
    };
    let volume = wait_for_volume_ready(client, volume).await?;

    Ok(DaytonaRecoveryBinding {
        volume_id: volume.id,
        volume_name: volume.name,
        mount_path: settings.mount_path,
        subpath: format!("sessions/{}", context.session_id),
        retained_revisions: settings.retained_revisions,
        head_revision: None,
    })
}

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
            Err(err) if is_not_found(&err) => return Ok(()),
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

fn requested_sandbox_name(context: &ToolContext, config: &SessionSandboxConfig) -> String {
    config
        .provider_config
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("Session Sandbox {}", context.session_id))
}

async fn sandbox_labels(context: &ToolContext) -> serde_json::Map<String, serde_json::Value> {
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
    labels
}

async fn run_sandbox_maintenance_command(
    client: &DaytonaClient,
    sandbox_id: &str,
    command: &str,
    operation: &str,
) -> Result<(), ToolExecutionResult> {
    let result = client
        .exec(sandbox_id, command, None, Some(EXEC_TIMEOUT_MS), |_| {})
        .await
        .map_err(|err| {
            ToolExecutionResult::tool_error(format!(
                "Failed to {operation} in Daytona sandbox: {err}"
            ))
        })?;
    if result.exit_code == 0 {
        return Ok(());
    }

    let output = if result.stderr.trim().is_empty() {
        result.stdout.trim()
    } else {
        result.stderr.trim()
    };
    Err(ToolExecutionResult::tool_error(format!(
        "Failed to {operation} in Daytona sandbox (exit code {}): {output}",
        result.exit_code
    )))
}

async fn checkpoint_workspace(
    client: &DaytonaClient,
    sandbox_id: &str,
    workspace_path: &str,
    recovery: &DaytonaRecoveryBinding,
) -> Result<String, ToolExecutionResult> {
    let timestamp = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| chrono::Utc::now().timestamp_micros() * 1_000);
    let revision = format!("rev-{timestamp}");
    let command = format!(
        r#"set -eu
mount={mount}
workspace={workspace}
revision={revision}
protected_revision={protected_revision}
revision_dir="$mount/revisions/$revision"
mkdir -p -- "$revision_dir"
tar --exclude='./node_modules' --exclude='*/node_modules' \
    --exclude='./target' --exclude='*/target' \
    --exclude='./.venv' --exclude='*/.venv' \
    --exclude='./.cache' --exclude='*/.cache' \
    -czf "$revision_dir/workspace.tar.gz" -C "$workspace" .
(cd "$revision_dir" && sha256sum workspace.tar.gz > workspace.tar.gz.sha256)
printf '%s\n' "$revision" > "$revision_dir/COMPLETE"
# Daytona Volume's FUSE layer does not implement rename. HEAD is only a
# convenience marker; persisted provider state remains authoritative.
printf '%s\n' "$revision" > "$mount/HEAD"
kept=0
for candidate in $(ls -1 "$mount/revisions" | sort -r); do
    kept=$((kept + 1))
    if [ "$kept" -gt {retained} ] && [ "$candidate" != "$protected_revision" ]; then
        rm -rf -- "$mount/revisions/$candidate"
    fi
done"#,
        mount = shell_escape(&recovery.mount_path),
        workspace = shell_escape(workspace_path),
        revision = shell_escape(&revision),
        protected_revision = shell_escape(recovery.head_revision.as_deref().unwrap_or("")),
        retained = recovery.retained_revisions,
    );
    run_sandbox_maintenance_command(client, sandbox_id, &command, "checkpoint the workspace")
        .await?;
    Ok(revision)
}

async fn restore_workspace(
    client: &DaytonaClient,
    sandbox_id: &str,
    workspace_path: &str,
    recovery: &DaytonaRecoveryBinding,
) -> Result<(), ToolExecutionResult> {
    let Some(revision) = recovery.head_revision.as_deref() else {
        let command = format!(
            r#"set -eu
workspace={workspace}
mkdir -p -- "$workspace"
find "$workspace" -mindepth 1 -maxdepth 1 -exec rm -rf -- {{}} +"#,
            workspace = shell_escape(workspace_path),
        );
        return run_sandbox_maintenance_command(
            client,
            sandbox_id,
            &command,
            "restore the empty workspace",
        )
        .await;
    };
    let verify_command = format!(
        r#"set -eu
mount={mount}
revision={revision}
case "$revision" in
    rev-[0-9]*) ;;
    *) echo "Invalid recovery revision: $revision" >&2; exit 72 ;;
esac
revision_dir="$mount/revisions/$revision"
test -f "$revision_dir/COMPLETE"
test -f "$revision_dir/workspace.tar.gz"
(cd "$revision_dir" && sha256sum -c workspace.tar.gz.sha256)"#,
        mount = shell_escape(&recovery.mount_path),
        revision = shell_escape(revision),
    );
    run_sandbox_maintenance_command(
        client,
        sandbox_id,
        &verify_command,
        "verify the recovery revision",
    )
    .await?;

    let restore_command = format!(
        r#"set -eu
mount={mount}
workspace={workspace}
revision={revision}
revision_dir="$mount/revisions/$revision"
mkdir -p -- "$workspace"
find "$workspace" -mindepth 1 -maxdepth 1 -exec rm -rf -- {{}} +
tar -xzf "$revision_dir/workspace.tar.gz" -C "$workspace""#,
        mount = shell_escape(&recovery.mount_path),
        workspace = shell_escape(workspace_path),
        revision = shell_escape(revision),
    );
    run_sandbox_maintenance_command(
        client,
        sandbox_id,
        &restore_command,
        "restore the workspace",
    )
    .await
}

async fn clear_recovery_storage(
    client: &DaytonaClient,
    sandbox_id: &str,
    recovery: &DaytonaRecoveryBinding,
) -> Result<(), ToolExecutionResult> {
    let command = format!(
        r#"set -eu
mount={mount}
find "$mount" -mindepth 1 -maxdepth 1 -exec rm -rf -- {{}} +"#,
        mount = shell_escape(&recovery.mount_path),
    );
    run_sandbox_maintenance_command(client, sandbox_id, &command, "clear recovery storage").await
}

async fn create_daytona_instance(
    client: &DaytonaClient,
    context: &ToolContext,
    config: &SessionSandboxConfig,
    requested_name: &str,
    recovery: Option<DaytonaRecoveryBinding>,
    restore: bool,
) -> Result<SessionSandboxInstance, ToolExecutionResult> {
    let workspace_path = workspace_path(config);
    let mut create_body = json!({
        "name": requested_name,
        "snapshot": snapshot_name(config)?,
        "autoStopInterval": auto_stop_minutes(config)?,
        "autoArchiveInterval": AUTO_ARCHIVE_INTERVAL_MINUTES,
        "autoDeleteInterval": AUTO_DELETE_INTERVAL_MINUTES,
        "labels": sandbox_labels(context).await,
    });
    if let Some(recovery) = &recovery {
        create_body["volumes"] = json!([{
            "volumeId": recovery.volume_id,
            "mountPath": recovery.mount_path,
            "subpath": recovery.subpath,
        }]);
    }

    let (sandbox_info, sandbox_name) =
        create_sandbox_with_unique_name(client, requested_name, create_body)
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

    let setup_result = async {
        run_sandbox_maintenance_command(
            client,
            &sandbox_info.id,
            &format!("mkdir -p -- {}", shell_escape(&workspace_path)),
            "create the workspace directory",
        )
        .await?;
        if restore && let Some(recovery) = &recovery {
            restore_workspace(client, &sandbox_info.id, &workspace_path, recovery).await?;
        }
        Ok::<(), ToolExecutionResult>(())
    }
    .await;
    if let Err(err) = setup_result {
        if let Err(delete_err) = client.delete_sandbox(&sandbox_info.id).await {
            warn!(
                sandbox_id = %sandbox_info.id,
                error = %delete_err,
                "Failed to delete Daytona sandbox after setup failure"
            );
        }
        return Err(err);
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
        provider_state: match recovery {
            Some(recovery) => json!({ "recovery": recovery }),
            None => json!({}),
        },
        metadata: json!({
            "remote_state": sandbox_info.state,
            "recovered": restore,
        }),
    })
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
        let workspace_path = workspace_path(config);
        let recovery = match recovery_settings(config, &workspace_path)? {
            Some(settings) => Some(resolve_recovery_binding(&client, context, settings).await?),
            None => None,
        };
        create_daytona_instance(
            &client,
            context,
            config,
            &requested_sandbox_name(context, config),
            recovery,
            false,
        )
        .await
    }

    async fn resume(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let api_key = get_api_key(context).await?;
        let client = build_client(api_key, config);
        let Some(recovery) = recovery_binding(instance)? else {
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

            return Ok(SessionSandboxInstance {
                external_id: instance.external_id.clone(),
                display_name: instance.display_name.clone().or(info.name.clone()),
                workspace_path: Some(workspace_path),
                provider_state: instance.provider_state.clone(),
                metadata: json!({ "remote_state": "started" }),
            });
        };

        let info = match client.get_sandbox(&instance.external_id).await {
            Ok(_) => ensure_sandbox_started(&client, &instance.external_id).await?,
            Err(err) if is_not_found(&err) => {
                let replacement_name = instance
                    .display_name
                    .clone()
                    .unwrap_or_else(|| requested_sandbox_name(context, config));
                let replacement = create_daytona_instance(
                    &client,
                    context,
                    config,
                    &replacement_name,
                    Some(recovery),
                    true,
                )
                .await?;
                if let Err(release_err) =
                    release_sandbox_lease(context, &instance.external_id).await
                {
                    warn!(
                        sandbox_id = %instance.external_id,
                        error = ?release_err,
                        "Failed to release lease for lost Daytona sandbox"
                    );
                }
                return Ok(replacement);
            }
            Err(err) => return Err(ToolExecutionResult::tool_error(err)),
        };

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
        match client.stop_sandbox(&instance.external_id).await {
            Ok(()) => {}
            Err(err) if is_not_found(&err) => {
                return Ok(SessionSandboxInstance {
                    metadata: json!({
                        "remote_state": "lost",
                        "recoverable": recovery_binding(instance)?.is_some(),
                    }),
                    ..instance.clone()
                });
            }
            Err(err) => return Err(ToolExecutionResult::tool_error(err)),
        }
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
        let recovery = recovery_binding(instance)?;
        let delete_instance = if let Some(recovery) = &recovery {
            match client.get_sandbox(&instance.external_id).await {
                Ok(_) => {
                    ensure_sandbox_started(&client, &instance.external_id).await?;
                    instance.clone()
                }
                Err(err) if is_not_found(&err) => {
                    let replacement_name = instance
                        .display_name
                        .clone()
                        .unwrap_or_else(|| requested_sandbox_name(context, config));
                    create_daytona_instance(
                        &client,
                        context,
                        config,
                        &replacement_name,
                        Some(recovery.clone()),
                        false,
                    )
                    .await?
                }
                Err(err) => return Err(ToolExecutionResult::tool_error(err)),
            }
        } else {
            instance.clone()
        };

        if let Some(recovery) = &recovery {
            clear_recovery_storage(&client, &delete_instance.external_id, recovery).await?;
        }
        delete_sandbox_with_retry(&client, &delete_instance.external_id).await?;
        release_sandbox_lease(context, &delete_instance.external_id).await?;
        if delete_instance.external_id != instance.external_id {
            release_sandbox_lease(context, &instance.external_id).await?;
        }
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

        let workspace_path = instance
            .workspace_path
            .clone()
            .unwrap_or_else(|| workspace_path(config));
        let lease_state = SandboxState {
            sandbox_id: instance.external_id.clone(),
            workspace_path,
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        touch_sandbox_lease(context, &lease_state, instance.display_name.clone()).await?;

        Ok(SessionSandboxWriteFileResponse {
            path: path.to_string(),
            bytes_written: content.len(),
        })
    }

    async fn checkpoint(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let Some(mut recovery) = recovery_binding(instance)? else {
            return Ok(instance.clone());
        };
        let api_key = get_api_key(context).await?;
        let client = build_client(api_key, config);
        let workspace_path = instance
            .workspace_path
            .clone()
            .unwrap_or_else(|| workspace_path(config));

        let revision = match checkpoint_workspace(
            &client,
            &instance.external_id,
            &workspace_path,
            &recovery,
        )
        .await
        {
            Ok(revision) => revision,
            Err(checkpoint_err) => {
                if let Err(restore_err) =
                    restore_workspace(&client, &instance.external_id, &workspace_path, &recovery)
                        .await
                {
                    return Err(ToolExecutionResult::tool_error(format!(
                        "Daytona workspace checkpoint failed ({checkpoint_err:?}) and rollback failed ({restore_err:?})"
                    )));
                }
                return Err(checkpoint_err);
            }
        };

        recovery.head_revision = Some(revision.clone());
        let mut checkpointed = instance.clone();
        checkpointed.provider_state["recovery"] =
            serde_json::to_value(&recovery).map_err(|err| {
                ToolExecutionResult::internal_error_msg(format!(
                    "Failed to persist Daytona recovery binding: {err}"
                ))
            })?;
        checkpointed.metadata["checkpoint_revision"] = json!(revision);
        Ok(checkpointed)
    }

    async fn status(
        &self,
        context: &ToolContext,
        config: &SessionSandboxConfig,
        state: &SessionSandboxState,
    ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult> {
        let api_key = get_api_key(context).await?;
        let client = build_client(api_key, config);
        let info = match client.get_sandbox(&state.instance.external_id).await {
            Ok(info) => info,
            Err(err) if is_not_found(&err) => {
                return Ok(SessionSandboxStatusResponse {
                    provider: state.provider.clone(),
                    session_status: SessionSandboxStatus::Lost,
                    external_id: state.instance.external_id.clone(),
                    display_name: state.instance.display_name.clone(),
                    workspace_path: state.instance.workspace_path.clone(),
                    metadata: json!({
                        "remote_state": "lost",
                        "recoverable": recovery_binding(&state.instance)?.is_some(),
                    }),
                });
            }
            Err(err) => return Err(ToolExecutionResult::tool_error(err)),
        };

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

#[cfg(test)]
mod tests {
    use super::*;

    fn recovery_instance(mount_path: &str, head_revision: Option<&str>) -> SessionSandboxInstance {
        SessionSandboxInstance {
            external_id: "sb_test".to_string(),
            display_name: None,
            workspace_path: Some("/workspace".to_string()),
            provider_state: json!({
                "recovery": {
                    "volume_id": "vol_test",
                    "volume_name": "everruns-recovery",
                    "mount_path": mount_path,
                    "subpath": "sessions/session_123",
                    "retained_revisions": 10,
                    "head_revision": head_revision,
                }
            }),
            metadata: json!({}),
        }
    }

    #[test]
    fn recovery_binding_rejects_non_normalized_mount_path() {
        let instance = recovery_instance("/mnt/recovery/../workspace", Some("rev-1"));

        assert!(recovery_binding(&instance).is_err());
    }

    #[test]
    fn recovery_binding_rejects_revision_path_traversal() {
        let instance = recovery_instance("/mnt/recovery", Some("rev-1/../../workspace"));

        assert!(recovery_binding(&instance).is_err());
    }

    #[test]
    fn recovery_binding_accepts_generated_revision() {
        let instance = recovery_instance("/mnt/recovery", Some("rev-1723223212345678900"));

        assert!(recovery_binding(&instance).is_ok());
    }

    #[test]
    fn recovery_settings_reject_daytona_home_as_workspace() {
        let config = SessionSandboxConfig {
            provider: "daytona".to_string(),
            auto_start: true,
            idle_pause_after_seconds: 180,
            provider_config: json!({
                "recovery": { "enabled": true }
            }),
            init: Default::default(),
        };

        assert!(recovery_settings(&config, DAYTONA_WORKSPACE_PATH).is_err());
        assert!(
            recovery_settings(&config, "/home/daytona/workspace/../control").is_err(),
            "a destructive restore target must not contain traversal"
        );
        assert!(
            recovery_settings(&config, "/home/daytona/workspace").is_ok(),
            "a dedicated child worktree should be recoverable"
        );
    }
}
