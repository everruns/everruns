//! Session state for Modal sandboxes: credentials, per-sandbox records, leases.
//!
//! Decision: each sandbox the session creates is recorded as a session secret
//! (`modal_sandbox:<id>`), like the other sandbox integrations. The record
//! carries the task ID so execs skip a control-plane lookup, and nothing
//! secret: the Modal token pair stays in the user's connection.

use everruns_contracts::runtime::UpsertLeasedResource;
use everruns_contracts::runtime::resource_ownership::verify_owned_external_resource_if_available;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::ToolExecutionResult;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{error, warn};

use super::client::{ModalClient, ModalCredentials};
use super::{MODAL_PROVIDER, MODAL_RESOURCE_TYPE, MODAL_SANDBOX_SECRET_PREFIX};

/// Lease registered for each sandbox. Every tool call refreshes it; when a
/// session goes quiet for this long the worker terminates the sandbox.
pub const MODAL_SANDBOX_LEASE_DURATION_SECONDS: u32 = 30 * 60;

/// Persisted record of one sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModalSandboxState {
    /// `sb-...`
    pub sandbox_id: String,
    /// `ta-...`, the container commands run in.
    pub task_id: String,
    /// The Modal app the sandbox belongs to.
    pub app_id: String,
    /// `vm` or `gvisor`.
    pub runtime: String,
    /// Registry tag or snapshot image it booted from.
    pub image: String,
    /// Default working directory for commands and relative paths.
    pub workspace_path: String,
    /// RFC 3339 creation time.
    pub started_at: String,
    /// Maximum lifetime Modal enforces.
    pub timeout_seconds: u32,
    /// Human-readable label.
    #[serde(default)]
    pub title: Option<String>,
    /// Ports exposed through tunnels.
    #[serde(default)]
    pub exposed_ports: Vec<u32>,
    /// Modal Secret holding injected connection tokens; deleted with the sandbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_secret_id: Option<String>,
}

/// Test-only override for the Modal control-plane URL, attached to
/// [`ToolContext`] extensions.
///
/// Decision: only exists under the `test-util` feature, which only this
/// crate's own tests enable, so shipped builds always talk to the real Modal
/// API and nothing at runtime can send the token pair elsewhere.
#[cfg(feature = "modal-test-util")]
#[derive(Debug, Clone)]
pub struct ModalServerUrlOverride(pub String);

/// Resolve the user's Modal token pair from their connection.
pub async fn get_credentials(
    context: &ToolContext,
) -> Result<ModalCredentials, ToolExecutionResult> {
    if let Some(resolver) = context.connection_resolver.as_ref() {
        match resolver
            .get_connection_token(context.session_id, MODAL_PROVIDER)
            .await
        {
            Ok(Some(raw)) if !raw.trim().is_empty() => {
                return ModalCredentials::parse(&raw).map_err(|e| {
                    ToolExecutionResult::tool_error(format!(
                        "The saved Modal connection is not a valid token pair: {e} Reconnect Modal in settings."
                    ))
                });
            }
            Ok(_) => {}
            Err(e) => error!("Failed to resolve Modal user connection: {e}"),
        }
    }
    // THREAT[TM-AGENT-016]: asking for credentials in chat would store them in
    // plaintext events. ConnectionRequired renders the inline connection flow.
    Err(ToolExecutionResult::connection_required(MODAL_PROVIDER))
}

/// Build a client for `context`: the real Modal API, or under `test-util` a
/// [`ModalServerUrlOverride`] extension when present.
pub fn client_for(
    credentials: ModalCredentials,
    context: &ToolContext,
) -> Result<ModalClient, ToolExecutionResult> {
    #[cfg(feature = "modal-test-util")]
    if let Some(url) = context.extension::<ModalServerUrlOverride>() {
        return ModalClient::with_server_url(credentials, &url.0)
            .map_err(ToolExecutionResult::tool_error);
    }
    #[cfg(not(feature = "modal-test-util"))]
    let _ = context;
    ModalClient::new(credentials).map_err(ToolExecutionResult::tool_error)
}

/// Sandbox IDs come from Modal (`sb-` plus base62), but the model passes them
/// back, so they are validated before reaching a secret name or request.
pub fn validate_sandbox_id(sandbox_id: &str) -> Result<(), ToolExecutionResult> {
    let valid = sandbox_id.len() <= 64
        && sandbox_id.strip_prefix("sb-").is_some_and(|rest| {
            !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric())
        });
    if valid {
        Ok(())
    } else {
        Err(ToolExecutionResult::tool_error(format!(
            "Invalid Modal sandbox ID '{sandbox_id}'. Use an ID returned by modal_create_sandbox or modal_list_sandboxes."
        )))
    }
}

fn storage(
    context: &ToolContext,
) -> Result<
    &std::sync::Arc<dyn everruns_contracts::runtime::session_services::SessionStorageStore>,
    ToolExecutionResult,
> {
    context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))
}

/// Load a sandbox the session created.
pub async fn get_sandbox_state(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<ModalSandboxState, ToolExecutionResult> {
    validate_sandbox_id(sandbox_id)?;
    // THREAT[TM-AGENT-020]: a `modal_sandbox:` secret alone is not proof of
    // ownership. Re-check the session's leased resources so a forged state
    // entry cannot point tools at another session's sandbox.
    verify_owned_external_resource_if_available(
        context,
        MODAL_PROVIDER,
        MODAL_RESOURCE_TYPE,
        sandbox_id,
    )
    .await?;
    let json_str = storage(context)?
        .get_secret(context.session_id, &format!("{MODAL_SANDBOX_SECRET_PREFIX}{sandbox_id}"))
        .await
        .map_err(|e| {
            error!("Failed to read Modal sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read sandbox state: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Sandbox '{sandbox_id}' not found in this session. Create one first with modal_create_sandbox."
            ))
        })?;
    serde_json::from_str(&json_str).map_err(|e| {
        error!("Corrupt Modal sandbox state for {sandbox_id}: {e}");
        ToolExecutionResult::internal_error_msg(format!("Corrupt sandbox state: {e}"))
    })
}

/// Persist a sandbox record.
pub async fn save_sandbox_state(
    context: &ToolContext,
    state: &ModalSandboxState,
) -> Result<(), ToolExecutionResult> {
    validate_sandbox_id(&state.sandbox_id)?;
    let json_str = serde_json::to_string(state).map_err(|e| {
        ToolExecutionResult::internal_error_msg(format!("Failed to serialize sandbox state: {e}"))
    })?;
    storage(context)?
        .set_secret(
            context.session_id,
            &format!("{MODAL_SANDBOX_SECRET_PREFIX}{}", state.sandbox_id),
            &json_str,
        )
        .await
        .map_err(|e| {
            error!("Failed to save Modal sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to save sandbox state: {e}"))
        })
}

/// Forget a sandbox record.
pub async fn delete_sandbox_state(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<(), ToolExecutionResult> {
    validate_sandbox_id(sandbox_id)?;
    storage(context)?
        .delete_secret(
            context.session_id,
            &format!("{MODAL_SANDBOX_SECRET_PREFIX}{sandbox_id}"),
        )
        .await
        .map_err(|e| {
            error!("Failed to delete Modal sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to delete sandbox state: {e}"))
        })?;
    Ok(())
}

/// Every sandbox record in the session, skipping unreadable ones.
pub async fn list_sandbox_states(
    context: &ToolContext,
) -> Result<Vec<ModalSandboxState>, ToolExecutionResult> {
    let secrets = storage(context)?
        .list_secrets(context.session_id)
        .await
        .map_err(|e| {
            error!("Failed to list secrets: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to list secrets: {e}"))
        })?;
    let mut states = Vec::new();
    for secret in secrets {
        if let Some(sandbox_id) = secret.name.strip_prefix(MODAL_SANDBOX_SECRET_PREFIX) {
            match get_sandbox_state(context, sandbox_id).await {
                Ok(state) => states.push(state),
                Err(_) => warn!("Skipping unreadable Modal sandbox state: {sandbox_id}"),
            }
        }
    }
    states.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    Ok(states)
}

/// Register or refresh the sandbox lease so forgotten sandboxes get terminated.
pub async fn touch_sandbox_lease(
    context: &ToolContext,
    state: &ModalSandboxState,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };
    let owner_user_id = match context.connection_resolver.as_ref() {
        Some(resolver) => resolver
            .get_connection_user(context.session_id, MODAL_PROVIDER)
            .await
            .ok()
            .flatten(),
        None => None,
    };
    store
        .upsert_resource(UpsertLeasedResource {
            session_id: context.session_id,
            provider: MODAL_PROVIDER.to_string(),
            resource_type: MODAL_RESOURCE_TYPE.to_string(),
            external_id: state.sandbox_id.clone(),
            display_name: state.title.clone(),
            owner_user_id,
            lease_duration_seconds: MODAL_SANDBOX_LEASE_DURATION_SECONDS,
            // THREAT[TM-API-015]: leased-resource metadata is API-visible;
            // keep it to non-secret descriptive fields.
            metadata: json!({
                "runtime": state.runtime,
                "image": state.image,
                "workspace_path": state.workspace_path,
                "started_at": state.started_at,
                "timeout_seconds": state.timeout_seconds,
                // An ID, not the secret: cleanup deletes the Modal Secret by it.
                "egress_secret_id": state.egress_secret_id,
            }),
        })
        .await
        .map_err(|e| {
            error!("Failed to upsert Modal lease: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to update Modal lease: {e}"))
        })?;
    Ok(())
}

/// Release the lease after an explicit terminate.
pub async fn release_sandbox_lease(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };
    store
        .release_resource(
            context.session_id,
            MODAL_PROVIDER,
            MODAL_RESOURCE_TYPE,
            sandbox_id,
        )
        .await
        .map_err(|e| {
            error!("Failed to release Modal lease: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to release Modal lease: {e}"))
        })?;
    Ok(())
}

/// A required string argument.
pub fn required_str<'a>(args: &'a Value, name: &str) -> Result<&'a str, ToolExecutionResult> {
    args.get(name).and_then(Value::as_str).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!("Missing required parameter: {name}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_ids_are_validated() {
        assert!(validate_sandbox_id("sb-kqbNMSqnOGhBmH3Zi3g5pl").is_ok());
        for bad in [
            "",
            "sb-",
            "kqbNMS",
            "sb-../../x",
            "sb-a b",
            "sb-a/b",
            "ta-abc",
        ] {
            assert!(validate_sandbox_id(bad).is_err(), "{bad}");
        }
        assert!(validate_sandbox_id(&format!("sb-{}", "a".repeat(80))).is_err());
    }

    #[test]
    fn state_round_trips_and_tolerates_missing_optional_fields() {
        let json = r#"{"sandbox_id":"sb-1","task_id":"ta-1","app_id":"ap-1","runtime":"vm",
            "image":"python:3.13-slim","workspace_path":"/workspace",
            "started_at":"2026-10-05T00:00:00Z","timeout_seconds":3600}"#;
        let state: ModalSandboxState = serde_json::from_str(json).unwrap();
        assert_eq!(state.title, None);
        assert!(state.exposed_ports.is_empty());
        let again: ModalSandboxState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(state, again);
    }

    #[test]
    fn required_str_rejects_missing_and_non_string() {
        assert_eq!(required_str(&json!({"a": "x"}), "a").unwrap(), "x");
        assert!(required_str(&json!({"a": 1}), "a").is_err());
        assert!(required_str(&json!({}), "a").is_err());
    }
}
