//! Sandbox state management — persistence, lease tracking, naming.
//!
//! Decision: Sandbox state is stored in session secrets (same pattern as Daytona).
//! Decision: Container/network names include session UUID for multi-tenant isolation.

use everruns_core::leased_resource::UpsertLeasedResource;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::ToolExecutionResult;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tracing::error;

/// Secret key prefix for sandbox state.
const SANDBOX_SECRET_PREFIX: &str = "container_sandbox:";

/// Lease duration for sandbox containers (20 minutes).
pub const SANDBOX_LEASE_DURATION_SECONDS: u32 = 20 * 60;

/// Sandbox state persisted in session secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxState {
    pub container_id: String,
    pub network_id: String,
    pub container_name: String,
    pub network_name: String,
    pub image: String,
    pub working_dir: String,
    pub started_at: String,
}

fn session_suffix(session_id: &str) -> String {
    let digest = Sha256::digest(session_id.as_bytes());
    hex::encode(&digest[..8])
}

/// Derive container name from session ID.
pub fn container_name(session_id: &str) -> String {
    format!("evr-{}-sandbox", session_suffix(session_id))
}

/// Derive network name from session ID.
pub fn network_name(session_id: &str) -> String {
    format!("sandbox-{}", session_suffix(session_id))
}

/// Standard labels applied to all containers and networks.
pub fn sandbox_labels(session_id: &str) -> std::collections::HashMap<String, String> {
    std::collections::HashMap::from([
        ("managed-by".to_string(), "everruns".to_string()),
        ("session".to_string(), session_id.to_string()),
    ])
}

/// Save sandbox state to session secrets.
pub async fn save_sandbox_state(
    context: &ToolContext,
    state: &SandboxState,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available"))?;

    let json_str = serde_json::to_string(state).map_err(|e| {
        error!("Failed to serialize sandbox state: {e}");
        ToolExecutionResult::internal_error_msg(format!("Serialization error: {e}"))
    })?;

    let key = format!("{SANDBOX_SECRET_PREFIX}{}", state.container_name);
    storage
        .set_secret(context.session_id, &key, &json_str)
        .await
        .map_err(|e| {
            error!("Failed to save sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to save state: {e}"))
        })?;

    Ok(())
}

/// Load sandbox state from session secrets.
pub async fn get_sandbox_state(context: &ToolContext) -> Result<SandboxState, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available"))?;

    // Find the sandbox state key by listing secrets with our prefix
    let session_id = context.session_id.to_string();
    let name = container_name(&session_id);
    let key = format!("{SANDBOX_SECRET_PREFIX}{name}");

    let json_str = storage
        .get_secret(context.session_id, &key)
        .await
        .map_err(|e| {
            error!("Failed to read sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read state: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(
                "No sandbox found for this session. Use sandbox_create first.",
            )
        })?;

    serde_json::from_str(&json_str).map_err(|e| {
        error!("Corrupt sandbox state: {e}");
        ToolExecutionResult::internal_error_msg(format!("Corrupt sandbox state: {e}"))
    })
}

/// Delete sandbox state from session secrets.
pub async fn delete_sandbox_state(
    context: &ToolContext,
    container_name: &str,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available"))?;

    let key = format!("{SANDBOX_SECRET_PREFIX}{container_name}");
    storage
        .delete_secret(context.session_id, &key)
        .await
        .map_err(|e| {
            error!("Failed to delete sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to delete state: {e}"))
        })?;

    Ok(())
}

/// Refresh or create the leased resource for this sandbox.
pub async fn touch_sandbox_lease(
    context: &ToolContext,
    state: &SandboxState,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };

    store
        .upsert_resource(UpsertLeasedResource {
            session_id: context.session_id,
            provider: "container_sandbox".to_string(),
            resource_type: "container".to_string(),
            external_id: state.container_id.clone(),
            display_name: Some(state.container_name.clone()),
            owner_user_id: None,
            lease_duration_seconds: SANDBOX_LEASE_DURATION_SECONDS,
            metadata: json!({
                "image": &state.image,
                "working_dir": &state.working_dir,
                "started_at": &state.started_at,
            }),
        })
        .await
        .map_err(|e| {
            error!("Failed to upsert leased resource: {e}");
            ToolExecutionResult::internal_error_msg(format!("Lease update failed: {e}"))
        })?;

    Ok(())
}

/// Release the leased resource for this sandbox.
pub async fn release_sandbox_lease(
    context: &ToolContext,
    container_id: &str,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };

    store
        .release_resource(
            context.session_id,
            "container_sandbox",
            "container",
            container_id,
        )
        .await
        .map_err(|e| {
            error!("Failed to release leased resource: {e}");
            ToolExecutionResult::internal_error_msg(format!("Lease release failed: {e}"))
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_name() {
        let name = container_name("abc123def456-more-stuff");
        assert_eq!(name, "evr-8ad60df670162fbe-sandbox");
    }

    #[test]
    fn test_network_name() {
        let name = network_name("abc123def456-more-stuff");
        assert_eq!(name, "sandbox-8ad60df670162fbe");
    }

    #[test]
    fn test_names_do_not_collide_for_shared_prefixes() {
        let first = "session_0193aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let second = "session_0193bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        assert_ne!(container_name(first), container_name(second));
        assert_ne!(network_name(first), network_name(second));
    }

    #[test]
    fn test_sandbox_labels() {
        let labels = sandbox_labels("session-123");
        assert_eq!(labels.get("managed-by").unwrap(), "everruns");
        assert_eq!(labels.get("session").unwrap(), "session-123");
    }

    #[test]
    fn test_sandbox_state_serialization() {
        let state = SandboxState {
            container_id: "abc123".to_string(),
            network_id: "net456".to_string(),
            container_name: "evr-abc-sandbox".to_string(),
            network_name: "sandbox-abc".to_string(),
            image: "ubuntu:24.04".to_string(),
            working_dir: "/workspace".to_string(),
            started_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let restored: SandboxState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.container_id, "abc123");
        assert_eq!(restored.image, "ubuntu:24.04");
    }
}
