//! State management helpers for CodeSandbox sandbox state persistence.
//!
//! All sandbox state is stored in session secrets (encrypted at rest).

use crate::types::*;

use everruns_core::tools::ToolExecutionResult;
use everruns_core::traits::ToolContext;
use serde_json::Value;
use tracing::{error, warn};

/// Retrieve the CodeSandbox API key from session secrets.
pub async fn get_api_key(context: &ToolContext) -> Result<String, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    storage
        .get_secret(context.session_id, CSB_API_KEY_SECRET)
        .await
        .map_err(|e| {
            error!("Failed to read CSB_API_KEY secret: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read API key: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(
                "CSB_API_KEY not set. Use `secret_store set CSB_API_KEY <your-key>` first. \
                 Get your key at https://codesandbox.io/t/api",
            )
        })
}

/// Load sandbox state from session secrets.
pub async fn get_sandbox_state(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<SandboxState, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{CSB_SANDBOX_SECRET_PREFIX}{sandbox_id}");
    let json_str = storage
        .get_secret(context.session_id, &secret_name)
        .await
        .map_err(|e| {
            error!("Failed to read sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read sandbox state: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Sandbox '{sandbox_id}' not found. Create one first with csb_create_sandbox."
            ))
        })?;

    serde_json::from_str(&json_str).map_err(|e| {
        error!("Corrupt sandbox state for {sandbox_id}: {e}");
        ToolExecutionResult::internal_error_msg(format!("Corrupt sandbox state: {e}"))
    })
}

/// Persist sandbox state to session secrets.
pub async fn save_sandbox_state(
    context: &ToolContext,
    state: &SandboxState,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{CSB_SANDBOX_SECRET_PREFIX}{}", state.sandbox_id);
    let json_str = serde_json::to_string(state).map_err(|e| {
        ToolExecutionResult::internal_error_msg(format!("Failed to serialize sandbox state: {e}"))
    })?;

    storage
        .set_secret(context.session_id, &secret_name, &json_str)
        .await
        .map_err(|e| {
            error!("Failed to save sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to save sandbox state: {e}"))
        })
}

/// Delete sandbox state from session secrets.
pub async fn delete_sandbox_state(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{CSB_SANDBOX_SECRET_PREFIX}{sandbox_id}");
    storage
        .delete_secret(context.session_id, &secret_name)
        .await
        .map_err(|e| {
            error!("Failed to delete sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to delete sandbox state: {e}"))
        })?;
    Ok(())
}

/// List all sandbox states for the current session.
pub async fn list_sandbox_states(
    context: &ToolContext,
) -> Result<Vec<SandboxState>, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secrets = storage
        .list_secrets(context.session_id)
        .await
        .map_err(|e| {
            error!("Failed to list secrets: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to list secrets: {e}"))
        })?;

    let mut states = Vec::new();
    for secret_info in secrets {
        if let Some(sandbox_id) = secret_info.name.strip_prefix(CSB_SANDBOX_SECRET_PREFIX) {
            match get_sandbox_state(context, sandbox_id).await {
                Ok(state) => states.push(state),
                Err(_) => {
                    warn!("Skipping corrupt sandbox state: {}", sandbox_id);
                }
            }
        }
    }

    Ok(states)
}

/// Extract a required string parameter from tool arguments.
pub fn required_str<'a>(args: &'a Value, name: &str) -> Result<&'a str, ToolExecutionResult> {
    args.get(name).and_then(|v| v.as_str()).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!("Missing required parameter: {name}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_required_str_present() {
        let args = serde_json::json!({"name": "test"});
        assert_eq!(required_str(&args, "name").unwrap(), "test");
    }

    #[test]
    fn test_required_str_missing() {
        let args = serde_json::json!({});
        let result = required_str(&args, "name");
        assert!(result.is_err());
    }

    #[test]
    fn test_required_str_wrong_type() {
        let args = serde_json::json!({"name": 42});
        let result = required_str(&args, "name");
        assert!(result.is_err());
    }

    #[test]
    fn test_required_str_null() {
        let args = serde_json::json!({"name": null});
        let result = required_str(&args, "name");
        assert!(result.is_err());
    }
}
