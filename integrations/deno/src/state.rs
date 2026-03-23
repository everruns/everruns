//! Deno sandbox session state management.
//!
//! Decision: persist only non-secret sandbox metadata in session secrets; always
//! resolve credentials fresh from user connections or operator env vars.

use everruns_core::UpsertLeasedResource;
use everruns_core::tools::ToolExecutionResult;
use everruns_core::traits::ToolContext;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{error, warn};

use crate::DENO_SANDBOX_SECRET_PREFIX;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxState {
    pub sandbox_id: String,
    pub region: String,
    pub org: Option<String>,
    pub workspace_path: String,
    pub started_at: String,
}

pub const DENO_SANDBOX_LEASE_DURATION_SECONDS: u32 = 20 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenoCredentials {
    pub token: String,
    pub org: Option<String>,
}

pub async fn get_credentials(
    context: &ToolContext,
) -> Result<DenoCredentials, ToolExecutionResult> {
    if let Some(resolver) = context.connection_resolver.as_ref() {
        match resolver
            .get_connection_token(context.session_id, "deno")
            .await
        {
            Ok(Some(token)) => {
                // Resolve org from provider_metadata if stored (personal tokens)
                let org = match resolver
                    .get_connection_metadata(context.session_id, "deno")
                    .await
                {
                    Ok(Some(meta)) => meta
                        .get("org")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    Ok(None) => None,
                    Err(e) => {
                        warn!("Failed to resolve Deno connection metadata: {e}");
                        None
                    }
                };
                // Personal tokens require an org slug; fail early if missing.
                if token.starts_with("ddp_") && org.is_none() {
                    return Err(ToolExecutionResult::connection_required("deno"));
                }
                return Ok(DenoCredentials { token, org });
            }
            Ok(None) => {}
            Err(e) => error!("Failed to resolve Deno user connection: {e}"),
        }
    }

    if let Ok(token) = std::env::var("DENO_DEPLOY_TOKEN")
        && !token.is_empty()
    {
        let org = std::env::var("DENO_DEPLOY_ORG")
            .ok()
            .filter(|s| !s.is_empty());
        if token.starts_with("ddp_") && org.is_none() {
            return Err(ToolExecutionResult::tool_error(
                "DENO_DEPLOY_TOKEN is a personal token (ddp_...) but DENO_DEPLOY_ORG is not set. Set DENO_DEPLOY_ORG or use an organization token (ddo_...).",
            ));
        }
        return Ok(DenoCredentials { token, org });
    }

    // THREAT[TM-AGENT-016]: Asking for sandbox credentials in chat would store
    // them plaintext in events. Return ConnectionRequired so the UI can render
    // the inline connection flow instead of asking the user to paste secrets.
    Err(ToolExecutionResult::connection_required("deno"))
}

pub async fn get_sandbox_state(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<SandboxState, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{DENO_SANDBOX_SECRET_PREFIX}{sandbox_id}");
    let json_str = storage
        .get_secret(context.session_id, &secret_name)
        .await
        .map_err(|e| {
            error!("Failed to read Deno sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read sandbox state: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Sandbox '{sandbox_id}' not found. Create one first with deno_create_sandbox."
            ))
        })?;

    serde_json::from_str(&json_str).map_err(|e| {
        error!("Corrupt Deno sandbox state for {sandbox_id}: {e}");
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

    let secret_name = format!("{DENO_SANDBOX_SECRET_PREFIX}{}", state.sandbox_id);
    let json_str = serde_json::to_string(state).map_err(|e| {
        ToolExecutionResult::internal_error_msg(format!("Failed to serialize sandbox state: {e}"))
    })?;

    storage
        .set_secret(context.session_id, &secret_name, &json_str)
        .await
        .map_err(|e| {
            error!("Failed to save Deno sandbox state: {e}");
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

    storage
        .delete_secret(
            context.session_id,
            &format!("{DENO_SANDBOX_SECRET_PREFIX}{sandbox_id}"),
        )
        .await
        .map_err(|e| {
            error!("Failed to delete Deno sandbox state: {e}");
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
        if let Some(sandbox_id) = secret_info.name.strip_prefix(DENO_SANDBOX_SECRET_PREFIX) {
            match get_sandbox_state(context, sandbox_id).await {
                Ok(state) => states.push(state),
                Err(_) => warn!("Skipping corrupt Deno sandbox state: {sandbox_id}"),
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

    let owner_user_id = if let Some(resolver) = context.connection_resolver.as_ref() {
        resolver
            .get_connection_user(context.session_id, "deno")
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    store
        .upsert_resource(UpsertLeasedResource {
            session_id: context.session_id,
            provider: "deno".to_string(),
            resource_type: "sandbox".to_string(),
            external_id: state.sandbox_id.clone(),
            display_name,
            owner_user_id,
            lease_duration_seconds: DENO_SANDBOX_LEASE_DURATION_SECONDS,
            // THREAT[TM-API-015]: leased-resource metadata is API-visible.
            // Keep only non-secret routing/debug fields here. The access token
            // still resolves from the user connection or operator env fallback.
            metadata: json!({
                "region": state.region,
                "org": state.org,
                "workspace_path": state.workspace_path,
                "started_at": state.started_at,
            }),
        })
        .await
        .map_err(|e| {
            error!("Failed to upsert Deno sandbox lease: {e}");
            ToolExecutionResult::internal_error_msg(format!(
                "Failed to update Deno sandbox lease: {e}"
            ))
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
        .release_resource(context.session_id, "deno", "sandbox", sandbox_id)
        .await
        .map_err(|e| {
            error!("Failed to release Deno sandbox lease: {e}");
            ToolExecutionResult::internal_error_msg(format!(
                "Failed to release Deno sandbox lease: {e}"
            ))
        })?;

    Ok(())
}

pub fn required_str<'a>(args: &'a Value, name: &str) -> Result<&'a str, ToolExecutionResult> {
    args.get(name).and_then(|v| v.as_str()).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!("Missing required parameter: {name}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_state_roundtrip() {
        let state = SandboxState {
            sandbox_id: "sb_123".to_string(),
            region: "ord".to_string(),
            org: Some("everruns".to_string()),
            workspace_path: "/home/sandbox".to_string(),
            started_at: "2026-03-22T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SandboxState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.sandbox_id, "sb_123");
        assert_eq!(deserialized.region, "ord");
        assert_eq!(deserialized.org.as_deref(), Some("everruns"));
    }

    #[test]
    fn required_str_reports_missing_param() {
        let err = required_str(&json!({}), "sandbox_id").unwrap_err();
        match err {
            ToolExecutionResult::ToolError(message) => assert!(message.contains("sandbox_id")),
            other => panic!("unexpected result: {other:?}"),
        }
    }
}
