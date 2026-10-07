//! Sandbox state management — persistence, lease tracking, naming.
//!
//! Decision: Sandbox state is stored in session secrets (same pattern as Daytona).
//! Decision: Container/network names include session UUID for multi-tenant isolation.
//! Decision: Docker operations re-check `managed-by` + `session` labels on the
//! live object; mutable stored IDs alone are not trusted (EVE-1151).

use everruns_core::leased_resource::UpsertLeasedResource;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::ToolExecutionResult;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tracing::error;

use super::client::DockerClient;

/// Secret key prefix for sandbox state.
///
/// Reserved from the user-facing `secret_store` in `everruns_core::host`. The host
/// cannot import this constant (crate layering), so the reservation string is
/// duplicated there and pinned by
/// `the_container_sandbox_secret_prefix_is_reserved_from_session_storage`.
pub const CONTAINER_SANDBOX_SECRET_PREFIX: &str = "container_sandbox:";

/// Label key identifying Everruns-managed Docker objects.
pub const MANAGED_BY_LABEL: &str = "managed-by";
/// Label value for Everruns-managed Docker objects.
pub const MANAGED_BY_VALUE: &str = "everruns";
/// Label key binding a Docker object to the session that created it.
pub const SESSION_LABEL: &str = "session";

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
        (MANAGED_BY_LABEL.to_string(), MANAGED_BY_VALUE.to_string()),
        (SESSION_LABEL.to_string(), session_id.to_string()),
    ])
}

/// True when Docker object labels identify an Everruns sandbox for `session_id`.
pub fn labels_owned_by_session(
    labels: &std::collections::HashMap<String, String>,
    session_id: &str,
) -> bool {
    labels.get(MANAGED_BY_LABEL).map(String::as_str) == Some(MANAGED_BY_VALUE)
        && labels.get(SESSION_LABEL).map(String::as_str) == Some(session_id)
}

/// Reject a stored container ID unless Docker inspect proves session ownership.
///
/// THREAT[TM-SANDBOX-004]: session secrets are mutable; tools must not act on a
/// forged `container_id` that points at another container on a shared daemon.
pub async fn verify_sandbox_container_ownership(
    client: &DockerClient,
    session_id: &str,
    container_id: &str,
) -> Result<(), ToolExecutionResult> {
    let inspect = client.inspect_container(container_id).await.map_err(|e| {
        // Do not echo the raw Docker message when the object is simply missing
        // or foreign: treat both as "not owned by this session".
        error!(container_id = %container_id, error = %e, "sandbox ownership inspect failed");
        ToolExecutionResult::tool_error(
            "Sandbox container is not an Everruns-managed sandbox for this session",
        )
    })?;

    if !labels_owned_by_session(&inspect.config.labels, session_id) {
        return Err(ToolExecutionResult::tool_error(
            "Sandbox container is not an Everruns-managed sandbox for this session",
        ));
    }

    Ok(())
}

/// Reject a stored network ID unless Docker inspect proves session ownership.
pub async fn verify_sandbox_network_ownership(
    client: &DockerClient,
    session_id: &str,
    network_id: &str,
) -> Result<(), ToolExecutionResult> {
    let inspect = client.inspect_network(network_id).await.map_err(|e| {
        error!(network_id = %network_id, error = %e, "sandbox network ownership inspect failed");
        ToolExecutionResult::tool_error(
            "Sandbox network is not an Everruns-managed sandbox network for this session",
        )
    })?;

    if !labels_owned_by_session(&inspect.labels, session_id) {
        return Err(ToolExecutionResult::tool_error(
            "Sandbox network is not an Everruns-managed sandbox network for this session",
        ));
    }

    Ok(())
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

    let key = format!("{CONTAINER_SANDBOX_SECRET_PREFIX}{}", state.container_name);
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
    let key = format!("{CONTAINER_SANDBOX_SECRET_PREFIX}{name}");

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

    let key = format!("{CONTAINER_SANDBOX_SECRET_PREFIX}{container_name}");
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
            connection_id: None,
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // `everruns_core::host` reserves this prefix from the user-facing secret_store,
    // but it cannot name the constant: host is a dependency of this crate.
    // Pin the two together here so a rename cannot quietly reopen the write
    // path that lets a session forge another container's ID into its state.
    #[test]
    fn the_container_sandbox_secret_prefix_is_reserved_from_session_storage() {
        assert!(crate::capabilities::is_internal_session_secret_name(
            &format!("{CONTAINER_SANDBOX_SECRET_PREFIX}evr-deadbeef-sandbox")
        ));
    }

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
        assert_eq!(labels.get(MANAGED_BY_LABEL).unwrap(), MANAGED_BY_VALUE);
        assert_eq!(labels.get(SESSION_LABEL).unwrap(), "session-123");
    }

    #[test]
    fn labels_owned_by_session_requires_both_markers() {
        let owned = sandbox_labels("session-a");
        assert!(labels_owned_by_session(&owned, "session-a"));
        assert!(!labels_owned_by_session(&owned, "session-b"));

        let mut foreign = owned.clone();
        foreign.insert(MANAGED_BY_LABEL.to_string(), "other".to_string());
        assert!(!labels_owned_by_session(&foreign, "session-a"));
    }

    #[tokio::test]
    async fn verify_sandbox_container_ownership_accepts_matching_labels() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1.47/containers/ctr-owned/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Id": "ctr-owned",
                "Name": "/evr-owned-sandbox",
                "State": {"Status": "running", "Running": true, "ExitCode": 0},
                "Config": {
                    "Labels": {
                        "managed-by": "everruns",
                        "session": "session-a"
                    }
                }
            })))
            .mount(&server)
            .await;

        let client = DockerClient::with_base_url(server.uri());
        verify_sandbox_container_ownership(&client, "session-a", "ctr-owned")
            .await
            .expect("owned container must pass");
    }

    #[tokio::test]
    async fn verify_sandbox_container_ownership_rejects_foreign_session_label() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1.47/containers/ctr-foreign/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Id": "ctr-foreign",
                "Name": "/evr-foreign-sandbox",
                "State": {"Status": "running", "Running": true, "ExitCode": 0},
                "Config": {
                    "Labels": {
                        "managed-by": "everruns",
                        "session": "session-victim"
                    }
                }
            })))
            .mount(&server)
            .await;

        let client = DockerClient::with_base_url(server.uri());
        let err = verify_sandbox_container_ownership(&client, "session-attacker", "ctr-foreign")
            .await
            .expect_err("foreign session label must fail");
        assert!(
            matches!(err, ToolExecutionResult::ToolError(ref msg) if msg.contains("not an Everruns-managed sandbox")),
            "unexpected error: {err:?}"
        );
    }

    #[tokio::test]
    async fn verify_sandbox_container_ownership_rejects_unmanaged_container() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1.47/containers/ctr-plain/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Id": "ctr-plain",
                "Name": "/postgres",
                "State": {"Status": "running", "Running": true, "ExitCode": 0},
                "Config": {"Labels": {}}
            })))
            .mount(&server)
            .await;

        let client = DockerClient::with_base_url(server.uri());
        let err = verify_sandbox_container_ownership(&client, "session-a", "ctr-plain")
            .await
            .expect_err("unmanaged container must fail");
        assert!(matches!(err, ToolExecutionResult::ToolError(_)));
    }

    #[tokio::test]
    async fn verify_sandbox_network_ownership_rejects_foreign_network() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1.47/networks/net-foreign"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Id": "net-foreign",
                "Name": "sandbox-victim",
                "Labels": {
                    "managed-by": "everruns",
                    "session": "session-victim"
                }
            })))
            .mount(&server)
            .await;

        let client = DockerClient::with_base_url(server.uri());
        let err = verify_sandbox_network_ownership(&client, "session-attacker", "net-foreign")
            .await
            .expect_err("foreign network must fail");
        assert!(
            matches!(err, ToolExecutionResult::ToolError(ref msg) if msg.contains("network")),
            "unexpected error: {err:?}"
        );
    }
}
