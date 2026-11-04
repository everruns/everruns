//! Session state for machines the boxd tools create: per-machine records,
//! credentials and leases.
//!
//! Decision: each machine is recorded as a session secret
//! (`boxd_sandbox:<region>/<id>`), like the other sandbox integrations. The
//! record carries the region so calls go straight to the right endpoint, and
//! nothing secret: the API key stays in the user's connection.

use everruns_contracts::runtime::UpsertLeasedResource;
use everruns_contracts::runtime::resource_ownership::verify_owned_external_resource_if_available;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::ToolExecutionResult;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{error, warn};

use super::client::BoxdClient;
use super::{BOXD_PROVIDER, BOXD_RESOURCE_TYPE, BOXD_SANDBOX_SECRET_PREFIX, MachineRef};

/// Lease registered for each machine. Every tool call refreshes it; when a
/// session goes quiet for this long the worker deletes the machine.
pub const BOXD_SANDBOX_LEASE_DURATION_SECONDS: u32 = 30 * 60;

/// Persisted record of one machine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoxdSandboxState {
    /// `<region>/<machine id>`, the id the model passes back.
    pub sandbox_id: String,
    pub region: String,
    pub machine_id: String,
    /// boxd's machine name (`er-...`).
    pub name: String,
    /// Default working directory for commands and relative paths.
    pub workspace_path: String,
    /// RFC 3339 creation time.
    pub started_at: String,
    #[serde(default)]
    pub title: Option<String>,
    /// The machine's HTTPS URL.
    #[serde(default)]
    pub url: Option<String>,
}

impl BoxdSandboxState {
    pub fn machine(&self) -> MachineRef {
        MachineRef {
            region: self.region.clone(),
            id: self.machine_id.clone(),
        }
    }
}

/// Test-only override for boxd's endpoints, attached to [`ToolContext`]
/// extensions: the mock serves both the token exchange and the gRPC API.
///
/// Decision: only exists under `boxd-test-util`, which only this crate's
/// tests enable, so shipped builds always talk to boxd and nothing at runtime
/// can send the key elsewhere.
#[cfg(feature = "boxd-test-util")]
#[derive(Debug, Clone)]
pub struct BoxdServerUrlOverride(pub String);

/// Resolve the user's boxd API key from their connection.
pub async fn get_api_key(context: &ToolContext) -> Result<String, ToolExecutionResult> {
    if let Some(resolver) = context.connection_resolver.as_ref() {
        match resolver
            .get_connection_token(context.session_id, BOXD_PROVIDER)
            .await
        {
            Ok(Some(key)) if !key.trim().is_empty() => return Ok(key),
            Ok(_) => {}
            Err(e) => error!("Failed to resolve boxd user connection: {e}"),
        }
    }
    // THREAT[TM-AGENT-016]: asking for credentials in chat would store them in
    // plaintext events. ConnectionRequired renders the inline connection flow.
    Err(ToolExecutionResult::connection_required(BOXD_PROVIDER))
}

/// A client for `context`: boxd itself, or under `boxd-test-util` a
/// [`BoxdServerUrlOverride`] extension when present.
pub fn client_for(
    api_key: String,
    context: &ToolContext,
) -> Result<BoxdClient, ToolExecutionResult> {
    #[cfg(feature = "boxd-test-util")]
    if let Some(url) = context.extension::<BoxdServerUrlOverride>() {
        return BoxdClient::with_endpoints(api_key, &format!("{}/auth/token", url.0), &url.0)
            .map_err(ToolExecutionResult::tool_error);
    }
    #[cfg(not(feature = "boxd-test-util"))]
    let _ = context;
    BoxdClient::new(api_key).map_err(ToolExecutionResult::tool_error)
}

/// Validate a sandbox id the model passed.
pub fn validate_sandbox_id(sandbox_id: &str) -> Result<MachineRef, ToolExecutionResult> {
    MachineRef::parse(sandbox_id).map_err(|e| {
        ToolExecutionResult::tool_error(format!(
            "{e}. Use an id returned by boxd_create_sandbox or boxd_list_sandboxes."
        ))
    })
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

/// Load a machine the session created.
pub async fn get_sandbox_state(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<BoxdSandboxState, ToolExecutionResult> {
    validate_sandbox_id(sandbox_id)?;
    // THREAT[TM-AGENT-020]: a `boxd_sandbox:` secret alone is not proof of
    // ownership. Re-check the session's leased resources so a forged state
    // entry cannot point tools at another session's machine.
    verify_owned_external_resource_if_available(
        context,
        BOXD_PROVIDER,
        BOXD_RESOURCE_TYPE,
        sandbox_id,
    )
    .await?;
    let json_str = storage(context)?
        .get_secret(context.session_id, &format!("{BOXD_SANDBOX_SECRET_PREFIX}{sandbox_id}"))
        .await
        .map_err(|e| {
            error!("Failed to read boxd sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read sandbox state: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "Sandbox '{sandbox_id}' not found in this session. Create one first with boxd_create_sandbox."
            ))
        })?;
    serde_json::from_str(&json_str).map_err(|e| {
        error!("Corrupt boxd sandbox state for {sandbox_id}: {e}");
        ToolExecutionResult::internal_error_msg(format!("Corrupt sandbox state: {e}"))
    })
}

/// Persist a machine record.
pub async fn save_sandbox_state(
    context: &ToolContext,
    state: &BoxdSandboxState,
) -> Result<(), ToolExecutionResult> {
    validate_sandbox_id(&state.sandbox_id)?;
    let json_str = serde_json::to_string(state).map_err(|e| {
        ToolExecutionResult::internal_error_msg(format!("Failed to serialize sandbox state: {e}"))
    })?;
    storage(context)?
        .set_secret(
            context.session_id,
            &format!("{BOXD_SANDBOX_SECRET_PREFIX}{}", state.sandbox_id),
            &json_str,
        )
        .await
        .map_err(|e| {
            error!("Failed to save boxd sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to save sandbox state: {e}"))
        })
}

/// Forget a machine record.
pub async fn delete_sandbox_state(
    context: &ToolContext,
    sandbox_id: &str,
) -> Result<(), ToolExecutionResult> {
    validate_sandbox_id(sandbox_id)?;
    storage(context)?
        .delete_secret(
            context.session_id,
            &format!("{BOXD_SANDBOX_SECRET_PREFIX}{sandbox_id}"),
        )
        .await
        .map_err(|e| {
            error!("Failed to delete boxd sandbox state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to delete sandbox state: {e}"))
        })?;
    Ok(())
}

/// Every machine record in the session, skipping unreadable ones.
pub async fn list_sandbox_states(
    context: &ToolContext,
) -> Result<Vec<BoxdSandboxState>, ToolExecutionResult> {
    let secrets = storage(context)?
        .list_secrets(context.session_id)
        .await
        .map_err(|e| {
            error!("Failed to list secrets: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to list secrets: {e}"))
        })?;
    let mut states = Vec::new();
    for secret in secrets {
        if let Some(sandbox_id) = secret.name.strip_prefix(BOXD_SANDBOX_SECRET_PREFIX) {
            match get_sandbox_state(context, sandbox_id).await {
                Ok(state) => states.push(state),
                Err(_) => warn!("Skipping unreadable boxd sandbox state: {sandbox_id}"),
            }
        }
    }
    states.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    Ok(states)
}

/// Register or refresh the machine lease so forgotten machines get deleted.
pub async fn touch_sandbox_lease(
    context: &ToolContext,
    state: &BoxdSandboxState,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };
    let owner_user_id = match context.connection_resolver.as_ref() {
        Some(resolver) => resolver
            .get_connection_user(context.session_id, BOXD_PROVIDER)
            .await
            .ok()
            .flatten(),
        None => None,
    };
    store
        .upsert_resource(UpsertLeasedResource {
            session_id: context.session_id,
            provider: BOXD_PROVIDER.to_string(),
            resource_type: BOXD_RESOURCE_TYPE.to_string(),
            external_id: state.sandbox_id.clone(),
            display_name: state.title.clone(),
            owner_user_id,
            connection_id: None,
            lease_duration_seconds: BOXD_SANDBOX_LEASE_DURATION_SECONDS,
            // THREAT[TM-API-015]: leased-resource metadata is API-visible;
            // keep it to non-secret descriptive fields.
            metadata: json!({
                "region": state.region,
                "name": state.name,
                "workspace_path": state.workspace_path,
                "started_at": state.started_at,
            }),
        })
        .await
        .map_err(|e| {
            error!("Failed to upsert boxd lease: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to update boxd lease: {e}"))
        })?;
    Ok(())
}

/// Release the lease after an explicit delete.
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
            BOXD_PROVIDER,
            BOXD_RESOURCE_TYPE,
            sandbox_id,
        )
        .await
        .map_err(|e| {
            error!("Failed to release boxd lease: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to release boxd lease: {e}"))
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
    fn state_round_trips_and_tolerates_missing_optional_fields() {
        let json = r#"{"sandbox_id":"fra/vm_1","region":"fra","machine_id":"vm_1",
            "name":"er-1","workspace_path":"/home/boxd/workspace",
            "started_at":"2026-11-04T00:00:00Z"}"#;
        let state: BoxdSandboxState = serde_json::from_str(json).unwrap();
        assert_eq!(state.title, None);
        assert_eq!(state.machine().to_string(), "fra/vm_1");
        let again: BoxdSandboxState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(state, again);
    }

    #[test]
    fn sandbox_ids_are_validated_with_a_hint() {
        assert!(validate_sandbox_id("nyc/vm_1").is_ok());
        assert!(validate_sandbox_id("../x").is_err());
    }
}
