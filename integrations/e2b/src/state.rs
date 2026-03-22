//! E2B API types and session state helpers.

use std::env;

use everruns_core::UpsertLeasedResource;
use everruns_core::tools::ToolExecutionResult;
use everruns_core::traits::ToolContext;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, warn};

use crate::{E2B_API_KEY_SECRET, E2B_DEFAULT_WORKSPACE_PATH, E2B_SANDBOX_SECRET_PREFIX};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct E2BSandboxCreateResponse {
    pub client_id: String,
    pub envd_version: String,
    pub sandbox_id: String,
    pub template_id: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub envd_access_token: Option<String>,
    #[serde(default)]
    pub traffic_access_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct E2BSandboxDetail {
    pub client_id: String,
    pub cpu_count: i64,
    pub disk_size_mb: i64,
    pub end_at: String,
    pub envd_version: String,
    pub memory_mb: i64,
    pub sandbox_id: String,
    pub started_at: String,
    pub state: String,
    pub template_id: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub envd_access_token: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxState {
    pub sandbox_id: String,
    pub sandbox_domain: String,
    pub envd_version: String,
    pub envd_access_token: Option<String>,
    pub workspace_path: String,
    pub started_at: String,
    pub timeout_seconds: u64,
}

pub const E2B_SANDBOX_LEASE_DURATION_SECONDS: u32 = 20 * 60;

pub async fn get_api_key(context: &ToolContext) -> Result<String, ToolExecutionResult> {
    if let Some(storage) = context.storage_store.as_ref() {
        match storage
            .get_secret(context.session_id, E2B_API_KEY_SECRET)
            .await
        {
            Ok(Some(key)) if !key.trim().is_empty() => return Ok(key),
            Ok(_) => {}
            Err(e) => {
                error!("Failed to resolve E2B session secret: {e}");
            }
        }
    }

    match env::var(E2B_API_KEY_SECRET) {
        Ok(key) if !key.trim().is_empty() => Ok(key),
        _ => Err(ToolExecutionResult::tool_error(
            "E2B API key not configured. Set E2B_API_KEY in the server/worker environment or as a session secret.",
        )),
    }
}

pub async fn get_sandbox_state(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<SandboxState, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{E2B_SANDBOX_SECRET_PREFIX}{sandbox_id}");
    let json_str = storage
        .get_secret(context.session_id, &secret_name)
        .await
        .map_err(|e| {
            error!("Failed to read E2B sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read sandbox state: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Sandbox '{sandbox_id}' not found. Create one first with e2b_create_sandbox."
            ))
        })?;

    serde_json::from_str(&json_str).map_err(|e| {
        error!("Corrupt E2B sandbox state for {sandbox_id}: {e}");
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

    let secret_name = format!("{E2B_SANDBOX_SECRET_PREFIX}{}", state.sandbox_id);
    let json_str = serde_json::to_string(state).map_err(|e| {
        ToolExecutionResult::internal_error_msg(format!("Failed to serialize sandbox state: {e}"))
    })?;

    storage
        .set_secret(context.session_id, &secret_name, &json_str)
        .await
        .map_err(|e| {
            error!("Failed to save E2B sandbox state: {e}");
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

    let secret_name = format!("{E2B_SANDBOX_SECRET_PREFIX}{sandbox_id}");
    storage
        .delete_secret(context.session_id, &secret_name)
        .await
        .map(|_| ())
        .map_err(|e| {
            error!("Failed to delete E2B sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to delete sandbox state: {e}"))
        })
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
            error!("Failed to list E2B secrets: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to list secrets: {e}"))
        })?;

    let mut states = Vec::new();
    for secret_info in secrets {
        if let Some(sandbox_id) = secret_info.name.strip_prefix(E2B_SANDBOX_SECRET_PREFIX) {
            match get_sandbox_state(context, sandbox_id).await {
                Ok(state) => states.push(state),
                Err(_) => warn!("Skipping corrupt E2B sandbox state: {sandbox_id}"),
            }
        }
    }

    Ok(states)
}

pub async fn touch_sandbox_lease(
    context: &ToolContext,
    state: &SandboxState,
    display_name: Option<String>,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };

    store
        .upsert_resource(UpsertLeasedResource {
            session_id: context.session_id,
            provider: "e2b".to_string(),
            resource_type: "sandbox".to_string(),
            external_id: state.sandbox_id.clone(),
            display_name,
            owner_user_id: None,
            lease_duration_seconds: E2B_SANDBOX_LEASE_DURATION_SECONDS,
            metadata: json!({
                "workspace_path": state.workspace_path,
                "sandbox_domain": state.sandbox_domain,
                "started_at": state.started_at,
            }),
        })
        .await
        .map_err(|e| {
            error!("Failed to upsert E2B sandbox lease: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to update sandbox lease: {e}"))
        })?;

    Ok(())
}

pub async fn release_sandbox_lease(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };

    store
        .release_resource(context.session_id, "e2b", "sandbox", sandbox_id)
        .await
        .map_err(|e| {
            error!("Failed to release E2B sandbox lease: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to release sandbox lease: {e}"))
        })?;

    Ok(())
}

pub fn required_str<'a>(
    args: &'a serde_json::Value,
    name: &str,
) -> Result<&'a str, ToolExecutionResult> {
    args.get(name).and_then(|v| v.as_str()).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!("Missing required parameter: {name}"))
    })
}

pub fn build_state(detail: &E2BSandboxDetail, timeout_seconds: u64) -> SandboxState {
    SandboxState {
        sandbox_id: detail.sandbox_id.clone(),
        sandbox_domain: detail
            .domain
            .clone()
            .unwrap_or_else(|| "e2b.app".to_string()),
        envd_version: detail.envd_version.clone(),
        envd_access_token: detail.envd_access_token.clone(),
        workspace_path: E2B_DEFAULT_WORKSPACE_PATH.to_string(),
        started_at: detail.started_at.clone(),
        timeout_seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_state_roundtrip() {
        let state = SandboxState {
            sandbox_id: "sb_test".to_string(),
            sandbox_domain: "example.e2b.app".to_string(),
            envd_version: "0.1.0".to_string(),
            envd_access_token: Some("token".to_string()),
            workspace_path: E2B_DEFAULT_WORKSPACE_PATH.to_string(),
            started_at: "2026-03-22T00:00:00Z".to_string(),
            timeout_seconds: 3600,
        };

        let json = serde_json::to_string(&state).unwrap();
        let decoded: SandboxState = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.sandbox_id, "sb_test");
        assert_eq!(decoded.sandbox_domain, "example.e2b.app");
        assert_eq!(decoded.timeout_seconds, 3600);
    }
}
