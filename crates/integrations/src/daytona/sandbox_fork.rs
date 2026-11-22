//! Copying a running Daytona sandbox for a forked session.

use super::*;
use everruns_contracts::runtime::resource_names::unique_resource_name;

/// Daytona refuses to fork a container sandbox with 422 "Forking is not
/// supported for this sandbox".
fn fork_unsupported(error: &str) -> bool {
    error.to_ascii_lowercase().contains("not supported")
}

/// The copy behind [`DaytonaSessionSandboxProvider::fork`].
pub(super) async fn fork(
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
    source: &SessionSandboxInstance,
) -> Result<Option<SessionSandboxInstance>, ToolExecutionResult> {
    if source.provider_state.get("recovery").is_some() {
        return Ok(None);
    }
    let api_key = get_managed_api_key(context, &config.credential).await?;
    let client = build_client(api_key, config);
    let requested_name =
        unique_resource_name(&requested_sandbox_name(context, config)).canonical_name;
    let info = match client
        .fork_sandbox(&source.external_id, &requested_name)
        .await
    {
        Ok(info) => info,
        Err(error) if fork_unsupported(&error) => return Ok(None),
        Err(error) => return Err(ToolExecutionResult::tool_error(error)),
    };
    let sandbox_id = info.id.clone();
    let canonical_name = info.name.clone().unwrap_or(requested_name);
    let setup = async {
        if let Err(err) = client.wait_for_ready(&sandbox_id).await {
            warn!(sandbox_id = %sandbox_id, error = %err, "Daytona sandbox copy readiness check failed");
        }
        // The copy carries the source's labels; give it this session's.
        let labels = create_body(&sandbox_labels(context).await, 0).labels;
        client
            .replace_labels(&sandbox_id, labels)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        client
            .set_autostop(&sandbox_id, auto_stop_minutes(config)?)
            .await
            .map_err(ToolExecutionResult::tool_error)?;
        network::apply_to_running(
            &client,
            &sandbox_id,
            &network::egress_rules(context, config)?,
        )
        .await?;
        let workspace_path = source
            .workspace_path
            .clone()
            .unwrap_or_else(|| workspace_path(config));
        let state = SandboxState {
            sandbox_id: sandbox_id.clone(),
            workspace_path: workspace_path.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        touch_sandbox_lease_with_credential(
            context,
            &state,
            Some(canonical_name.clone()),
            Some(config.credential.clone()),
        )
        .await?;
        Ok::<_, ToolExecutionResult>(SessionSandboxInstance {
            external_id: sandbox_id.clone(),
            display_name: Some(canonical_name.clone()),
            workspace_path: Some(workspace_path),
            provider_state: json!({}),
            metadata: json!({ "remote_state": info.state }),
        })
    }
    .await;
    match setup {
        Ok(instance) => Ok(Some(instance)),
        Err(error) => {
            if let Err(delete_err) = client.delete_sandbox(&sandbox_id).await {
                warn!(
                    sandbox_id = %sandbox_id,
                    error = %delete_err,
                    "Failed to delete Daytona sandbox copy after setup failure"
                );
            }
            Err(error)
        }
    }
}
