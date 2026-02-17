//! Daytona API types and session state management.

use everruns_core::tools::ToolExecutionResult;
use everruns_core::traits::ToolContext;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{error, warn};

use crate::{DAYTONA_API_KEY_SECRET, DAYTONA_SANDBOX_SECRET_PREFIX};

// ============================================================================
// API Response Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxInfo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecResult {
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub exit_code: i32,
}

// ============================================================================
// Persisted Sandbox State (stored in session secrets as JSON)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxState {
    pub sandbox_id: String,
    pub workspace_path: String,
    pub started_at: String,
}

// ============================================================================
// State Management Helpers
// ============================================================================

pub async fn get_api_key(context: &ToolContext) -> Result<String, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    storage
        .get_secret(context.session_id, DAYTONA_API_KEY_SECRET)
        .await
        .map_err(|e| {
            error!("Failed to read DAYTONA_API_KEY secret: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read API key: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(
                "DAYTONA_API_KEY not set. Use `secret_store set DAYTONA_API_KEY <your-key>` first. \
                 Get your key at https://app.daytona.io/ under API Keys.",
            )
        })
}

pub async fn get_sandbox_state(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<SandboxState, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{DAYTONA_SANDBOX_SECRET_PREFIX}{sandbox_id}");
    let json_str = storage
        .get_secret(context.session_id, &secret_name)
        .await
        .map_err(|e| {
            error!("Failed to read sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read sandbox state: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Sandbox '{sandbox_id}' not found. Create one first with daytona_create_sandbox."
            ))
        })?;

    serde_json::from_str(&json_str).map_err(|e| {
        error!("Corrupt sandbox state for {sandbox_id}: {e}");
        ToolExecutionResult::internal_error_msg(format!("Corrupt sandbox state: {e}"))
    })
}

pub async fn save_sandbox_state(
    context: &ToolContext,
    state: &SandboxState,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{DAYTONA_SANDBOX_SECRET_PREFIX}{}", state.sandbox_id);
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

pub async fn delete_sandbox_state(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{DAYTONA_SANDBOX_SECRET_PREFIX}{sandbox_id}");
    storage
        .delete_secret(context.session_id, &secret_name)
        .await
        .map_err(|e| {
            error!("Failed to delete sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to delete sandbox state: {e}"))
        })?;
    Ok(())
}

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
        if let Some(sandbox_id) = secret_info.name.strip_prefix(DAYTONA_SANDBOX_SECRET_PREFIX) {
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_state_roundtrip() {
        let state = SandboxState {
            sandbox_id: "sb_123".to_string(),
            workspace_path: "/sandbox".to_string(),
            started_at: "2026-02-16T10:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SandboxState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.sandbox_id, "sb_123");
        assert_eq!(deserialized.workspace_path, "/sandbox");
    }

    #[test]
    fn test_sandbox_state_with_special_chars() {
        let state = SandboxState {
            sandbox_id: "sb-test_123".to_string(),
            workspace_path: "/sandbox/my project".to_string(),
            started_at: "2026-02-16T10:00:00+05:30".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SandboxState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.workspace_path, "/sandbox/my project");
    }

    #[test]
    fn test_required_str_present() {
        let args = serde_json::json!({"name": "test_value"});
        let result = required_str(&args, "name");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test_value");
    }

    #[test]
    fn test_required_str_missing() {
        let args = serde_json::json!({"other": "value"});
        let result = required_str(&args, "name");
        assert!(result.is_err());
    }

    #[test]
    fn test_required_str_null_value() {
        let args = serde_json::json!({"name": null});
        let result = required_str(&args, "name");
        assert!(result.is_err());
    }

    #[test]
    fn test_required_str_non_string_value() {
        let args = serde_json::json!({"name": 42});
        let result = required_str(&args, "name");
        assert!(result.is_err());
    }

    #[test]
    fn test_sandbox_info_deserialization() {
        let json = r#"{"id": "sb_abc", "name": "My Sandbox", "state": "started"}"#;
        let info: SandboxInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "sb_abc");
        assert_eq!(info.name, Some("My Sandbox".to_string()));
        assert_eq!(info.state, "started");
    }

    #[test]
    fn test_sandbox_info_without_name() {
        let json = r#"{"id": "sb_abc", "state": "stopped"}"#;
        let info: SandboxInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "sb_abc");
        assert_eq!(info.name, None);
        assert_eq!(info.state, "stopped");
    }

    #[test]
    fn test_exec_result_deserialization() {
        let json = r#"{"result": "hello\n", "exitCode": 0}"#;
        let result: ExecResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.result, "hello\n");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_exec_result_defaults() {
        let json = r#"{}"#;
        let result: ExecResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.result, "");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_exec_result_nonzero_exit() {
        let json = r#"{"result": "error msg", "exitCode": 1}"#;
        let result: ExecResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.result, "error msg");
    }

    #[test]
    fn test_sandbox_state_json_has_all_fields() {
        let state = SandboxState {
            sandbox_id: "sb_x".to_string(),
            workspace_path: "/sandbox".to_string(),
            started_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let val: serde_json::Value = serde_json::to_value(&state).unwrap();
        assert!(val.get("sandbox_id").is_some());
        assert!(val.get("workspace_path").is_some());
        assert!(val.get("started_at").is_some());
    }
}
