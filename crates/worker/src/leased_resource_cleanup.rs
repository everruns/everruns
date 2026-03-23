// Durable leased-resource cleanup activity.
//
// Decision: Cleanup runs as a standalone durable activity scheduled by the
// control plane so it survives restarts and inherits durable scheduler
// observability (schedule executions, task history, worker heartbeats).
// Decision: The leased-resource model is generic and reusable by any subsystem
// that creates provider-owned state needing eventual cleanup.
// Decision: Cleanup completion/failure uses compare-and-set transitions keyed
// by the claim timestamp so stale cleanup attempts cannot overwrite a newer
// lease refresh.

use anyhow::{Result, anyhow};
use everruns_core::traits::{SessionStorageStore, UserConnectionResolver};
use everruns_core::{LeasedResource, SessionId};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::worker_adapters::WorkerAdapters;

const DAYTONA_SNAPSHOT_SECRET_PREFIX: &str = "daytona_sandbox:";
const SPRITES_SECRET_PREFIX: &str = "sprites_sprite:";
const E2B_SANDBOX_SECRET_PREFIX: &str = "e2b_sandbox:";
const DENO_SNAPSHOT_SECRET_PREFIX: &str = "deno_sandbox:";
const BROWSERLESS_SESSION_KEY: &str = "browserless_browser_session";

/// Durable activity input for leased-resource cleanup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeasedResourceCleanupInput {
    /// Max resources to claim per cleanup run.
    pub batch_size: u32,
    /// How long a `cleaning` claim may be stale before another worker reclaims it.
    pub stale_after_seconds: u32,
    /// Backoff before retrying a failed cleanup attempt.
    pub retry_after_seconds: u32,
}

impl Default for LeasedResourceCleanupInput {
    fn default() -> Self {
        Self {
            batch_size: 25,
            stale_after_seconds: 5 * 60,
            retry_after_seconds: 60,
        }
    }
}

#[derive(Debug, Serialize)]
struct CleanupOutcome {
    resource_id: String,
    provider: String,
    resource_type: String,
    external_id: String,
    status: String,
    detail: String,
}

#[derive(Debug, Serialize)]
struct CleanupSummary {
    claimed: usize,
    released: usize,
    failed: usize,
    skipped: usize,
    outcomes: Vec<CleanupOutcome>,
}

/// Execute one leased-resource cleanup pass using the worker adapters.
pub async fn execute_cleanup_activity<A: WorkerAdapters>(
    adapters: &A,
    input: &LeasedResourceCleanupInput,
) -> Result<serde_json::Value> {
    let resources = adapters
        .claim_due_leased_resources(input.batch_size, input.stale_after_seconds)
        .await?;

    let resolver = adapters.connection_resolver();
    let storage_store = adapters.storage_store();

    let mut summary = CleanupSummary {
        claimed: resources.len(),
        released: 0,
        failed: 0,
        skipped: 0,
        outcomes: Vec::with_capacity(resources.len()),
    };

    for resource in resources {
        let Some(expected_cleanup_started_at) = resource.cleanup_started_at else {
            summary.skipped += 1;
            summary.outcomes.push(CleanupOutcome {
                resource_id: resource.id.to_string(),
                provider: resource.provider.clone(),
                resource_type: resource.resource_type.clone(),
                external_id: resource.external_id.clone(),
                status: "skipped".to_string(),
                detail: "Resource claim missing cleanup_started_at".to_string(),
            });
            continue;
        };

        match cleanup_resource(&resource, resolver.as_ref(), storage_store.as_ref()).await {
            Ok(detail) => {
                let updated = adapters
                    .mark_leased_resource_released(resource.id, expected_cleanup_started_at)
                    .await?;
                if updated {
                    info!(
                        resource_id = %resource.id,
                        provider = %resource.provider,
                        resource_type = %resource.resource_type,
                        external_id = %resource.external_id,
                        "Leased resource cleanup released resource"
                    );
                } else {
                    info!(
                        resource_id = %resource.id,
                        provider = %resource.provider,
                        resource_type = %resource.resource_type,
                        external_id = %resource.external_id,
                        "Leased resource cleanup skipped stale release transition"
                    );
                }
                if updated {
                    summary.released += 1;
                } else {
                    summary.skipped += 1;
                }
                summary.outcomes.push(CleanupOutcome {
                    resource_id: resource.id.to_string(),
                    provider: resource.provider.clone(),
                    resource_type: resource.resource_type.clone(),
                    external_id: resource.external_id.clone(),
                    status: if updated { "released" } else { "skipped" }.to_string(),
                    detail,
                });
            }
            Err(error) => {
                let updated = adapters
                    .mark_leased_resource_cleanup_failed(
                        resource.id,
                        expected_cleanup_started_at,
                        input.retry_after_seconds,
                        &error.to_string(),
                    )
                    .await?;
                if updated {
                    warn!(
                        resource_id = %resource.id,
                        provider = %resource.provider,
                        resource_type = %resource.resource_type,
                        external_id = %resource.external_id,
                        error = %error,
                        "Leased resource cleanup failed; resource scheduled for retry"
                    );
                } else {
                    info!(
                        resource_id = %resource.id,
                        provider = %resource.provider,
                        resource_type = %resource.resource_type,
                        external_id = %resource.external_id,
                        error = %error,
                        "Leased resource cleanup skipped stale failure transition"
                    );
                }
                if updated {
                    summary.failed += 1;
                } else {
                    summary.skipped += 1;
                }
                summary.outcomes.push(CleanupOutcome {
                    resource_id: resource.id.to_string(),
                    provider: resource.provider.clone(),
                    resource_type: resource.resource_type.clone(),
                    external_id: resource.external_id.clone(),
                    status: if updated { "failed" } else { "skipped" }.to_string(),
                    detail: error.to_string(),
                });
            }
        }
    }

    info!(
        claimed = summary.claimed,
        released = summary.released,
        failed = summary.failed,
        skipped = summary.skipped,
        "Leased resource cleanup pass completed"
    );

    Ok(serde_json::to_value(summary)?)
}

async fn cleanup_resource(
    resource: &LeasedResource,
    resolver: &dyn UserConnectionResolver,
    storage_store: &dyn SessionStorageStore,
) -> Result<String> {
    let token = resolve_provider_token(resource, resolver, storage_store).await?;

    match (resource.provider.as_str(), resource.resource_type.as_str()) {
        ("daytona", "sandbox") => cleanup_daytona(resource, storage_store, &token).await,
        ("e2b", "sandbox") => cleanup_e2b(resource, storage_store, &token).await,
        ("deno", "sandbox") => cleanup_deno(resource, storage_store, &token).await,
        ("sprites", "sprite") => cleanup_sprites(resource, storage_store, &token).await,
        ("browserless", "browser_session") => {
            cleanup_browserless(resource, storage_store, &token).await
        }
        _ => Err(anyhow!(
            "No leased-resource cleanup handler registered for provider '{}' type '{}'",
            resource.provider,
            resource.resource_type
        )),
    }
}

async fn resolve_provider_token(
    resource: &LeasedResource,
    resolver: &dyn UserConnectionResolver,
    storage_store: &dyn SessionStorageStore,
) -> Result<String> {
    if resource.provider == "e2b" {
        return resolve_e2b_api_key(resource, storage_store).await;
    }
    if let Some(owner_user_id) = resource.owner_user_id
        && let Some(token) = resolver
            .get_connection_token_for_user(owner_user_id, &resource.provider)
            .await?
    {
        return Ok(token);
    }

    if let Some(session_id) = resource.session_id
        && let Some(token) = resolver
            .get_connection_token(session_id, &resource.provider)
            .await?
    {
        return Ok(token);
    }

    if resource.provider == "deno"
        && let Ok(token) = std::env::var("DENO_DEPLOY_TOKEN")
        && !token.is_empty()
    {
        return Ok(token);
    }

    Err(anyhow!(
        "No provider token available for leased resource {} ({}/{})",
        resource.id,
        resource.provider,
        resource.resource_type
    ))
}

async fn resolve_e2b_api_key(
    resource: &LeasedResource,
    storage_store: &dyn SessionStorageStore,
) -> Result<String> {
    if let Some(session_id) = resource.session_id
        && let Some(token) = storage_store.get_secret(session_id, "E2B_API_KEY").await?
        && !token.trim().is_empty()
    {
        return Ok(token);
    }

    let env_token = std::env::var("E2B_API_KEY")
        .map_err(|_| anyhow!("E2B_API_KEY not configured for leased resource cleanup"))?;

    if env_token.trim().is_empty() {
        return Err(anyhow!(
            "E2B_API_KEY is set but empty/whitespace for leased resource cleanup"
        ));
    }

    Ok(env_token)
}

async fn cleanup_daytona(
    resource: &LeasedResource,
    storage_store: &dyn SessionStorageStore,
    token: &str,
) -> Result<String> {
    let client = everruns_integrations_daytona::client::DaytonaClient::new(token.to_string());

    match client.delete_sandbox(&resource.external_id).await {
        Ok(()) => {}
        Err(error) if is_daytona_not_found(&error) => {
            warn!(
                sandbox_id = %resource.external_id,
                error = %error,
                "Daytona sandbox already gone during cleanup"
            );
        }
        Err(error) => return Err(anyhow!("Daytona cleanup failed: {error}")),
    }

    if let Some(session_id) = resource.session_id {
        let _ = storage_store
            .delete_secret(
                session_id,
                &format!("{DAYTONA_SNAPSHOT_SECRET_PREFIX}{}", resource.external_id),
            )
            .await;
    }

    Ok(format!("Deleted Daytona sandbox {}", resource.external_id))
}

async fn cleanup_e2b(
    resource: &LeasedResource,
    storage_store: &dyn SessionStorageStore,
    token: &str,
) -> Result<String> {
    let client = everruns_integrations_e2b::client::E2BClient::new(token.to_string());

    match client.delete_sandbox(&resource.external_id).await {
        Ok(()) => {}
        Err(error) if is_e2b_not_found(&error) => {
            warn!(
                sandbox_id = %resource.external_id,
                error = %error,
                "E2B sandbox already gone during cleanup"
            );
        }
        Err(error) => return Err(anyhow!("E2B cleanup failed: {error}")),
    }

    if let Some(session_id) = resource.session_id {
        let _ = storage_store
            .delete_secret(
                session_id,
                &format!("{E2B_SANDBOX_SECRET_PREFIX}{}", resource.external_id),
            )
            .await;
    }

    Ok(format!("Deleted E2B sandbox {}", resource.external_id))
}

async fn cleanup_deno(
    resource: &LeasedResource,
    storage_store: &dyn SessionStorageStore,
    token: &str,
) -> Result<String> {
    let region = resource
        .metadata
        .get("region")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("Deno leased resource missing metadata.region"))?;
    let org = deno_cleanup_org(resource);

    let client = everruns_integrations_deno::client::DenoClient::new(token.to_string(), org);
    match client.delete_sandbox(&resource.external_id, region).await {
        Ok(()) => {}
        Err(error) if is_deno_not_found(&error) => {
            warn!(
                sandbox_id = %resource.external_id,
                error = %error,
                "Deno sandbox already gone during cleanup"
            );
        }
        Err(error) => return Err(anyhow!("Deno cleanup failed: {error}")),
    }

    if let Some(session_id) = resource.session_id {
        let _ = storage_store
            .delete_secret(
                session_id,
                &format!("{DENO_SNAPSHOT_SECRET_PREFIX}{}", resource.external_id),
            )
            .await;
    }

    Ok(format!("Deleted Deno sandbox {}", resource.external_id))
}

async fn cleanup_sprites(
    resource: &LeasedResource,
    storage_store: &dyn SessionStorageStore,
    token: &str,
) -> Result<String> {
    let client = everruns_integrations_sprites::client::SpritesClient::new(token.to_string());

    match client.delete_sprite(&resource.external_id).await {
        Ok(()) => {}
        Err(error) if error.contains("404") || error.contains("not found") => {
            warn!(
                sprite_name = %resource.external_id,
                error = %error,
                "Sprites sprite already gone during cleanup"
            );
        }
        Err(error) => return Err(anyhow!("Sprites cleanup failed: {error}")),
    }

    if let Some(session_id) = resource.session_id {
        let _ = storage_store
            .delete_secret(
                session_id,
                &format!("{SPRITES_SECRET_PREFIX}{}", resource.external_id),
            )
            .await;
    }

    Ok(format!("Deleted Sprites sprite {}", resource.external_id))
}

async fn cleanup_browserless(
    resource: &LeasedResource,
    storage_store: &dyn SessionStorageStore,
    token: &str,
) -> Result<String> {
    let ws_endpoint = resource
        .metadata
        .get("ws_endpoint")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("Browserless leased resource missing metadata.ws_endpoint"))?;

    let reconnect_url = everruns_integrations_browserless::state::BrowserSessionState {
        ws_endpoint: ws_endpoint.to_string(),
        created_at: String::new(),
        last_active_at: String::new(),
    }
    .reconnect_url(token);

    match everruns_integrations_browserless::cdp::CdpSession::connect(&reconnect_url).await {
        Ok(session) => {
            session.disconnect().await;
        }
        Err(error) if is_browserless_already_gone(&error) => {
            warn!(
                external_id = %resource.external_id,
                error = %error,
                "Browserless session already gone during cleanup"
            );
        }
        Err(error) => return Err(anyhow!("Browserless cleanup failed: {error}")),
    }

    if let Some(session_id) = resource.session_id {
        clear_browserless_session_state_if_matches(session_id, resource, storage_store).await;
    }

    Ok(format!(
        "Released Browserless browser {}",
        resource.external_id
    ))
}

async fn clear_browserless_session_state_if_matches(
    session_id: SessionId,
    resource: &LeasedResource,
    storage_store: &dyn SessionStorageStore,
) {
    let Ok(Some(state_json)) = storage_store
        .get_value(session_id, BROWSERLESS_SESSION_KEY)
        .await
    else {
        return;
    };

    let Ok(state) = serde_json::from_str::<
        everruns_integrations_browserless::state::BrowserSessionState,
    >(&state_json) else {
        return;
    };

    if browserless_external_id(&state.ws_endpoint) != resource.external_id {
        return;
    }

    let _ = storage_store
        .delete_value(session_id, BROWSERLESS_SESSION_KEY)
        .await;
}

fn browserless_external_id(ws_endpoint: &str) -> String {
    everruns_integrations_browserless::state::browser_session_external_id(ws_endpoint)
}

fn is_daytona_not_found(error: &str) -> bool {
    error.contains("404") || error.to_ascii_lowercase().contains("not found")
}

fn is_e2b_not_found(error: &str) -> bool {
    error.contains("404") || error.to_ascii_lowercase().contains("not found")
}

fn deno_cleanup_org(resource: &LeasedResource) -> Option<String> {
    resource
        .metadata
        .get("org")
        .and_then(serde_json::Value::as_str)
        .filter(|org| !org.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("DENO_DEPLOY_ORG")
                .ok()
                .filter(|org| !org.is_empty())
        })
}

fn is_deno_not_found(error: &str) -> bool {
    error.contains("404") || error.to_ascii_lowercase().contains("not found")
}

fn is_browserless_already_gone(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("404")
        || error.contains("410")
        || error.contains("stream ended")
        || error.contains("closed unexpectedly")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use everruns_core::{KeyInfo, LeasedResourceStatus, SecretInfo};
    use uuid::Uuid;

    struct TestSessionStorageStore {
        secrets: Mutex<HashMap<(String, String), String>>,
    }

    impl TestSessionStorageStore {
        fn with_secret(session_id: SessionId, name: &str, value: &str) -> Self {
            let mut secrets = HashMap::new();
            secrets.insert(
                (session_id.to_string(), name.to_string()),
                value.to_string(),
            );
            Self {
                secrets: Mutex::new(secrets),
            }
        }

        fn empty() -> Self {
            Self {
                secrets: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl SessionStorageStore for TestSessionStorageStore {
        async fn set_value(
            &self,
            _session_id: SessionId,
            _key: &str,
            _value: &str,
        ) -> everruns_core::Result<()> {
            unimplemented!()
        }

        async fn get_value(
            &self,
            _session_id: SessionId,
            _key: &str,
        ) -> everruns_core::Result<Option<String>> {
            unimplemented!()
        }

        async fn delete_value(
            &self,
            _session_id: SessionId,
            _key: &str,
        ) -> everruns_core::Result<bool> {
            unimplemented!()
        }

        async fn list_keys(&self, _session_id: SessionId) -> everruns_core::Result<Vec<KeyInfo>> {
            unimplemented!()
        }

        async fn set_secret(
            &self,
            _session_id: SessionId,
            _name: &str,
            _value: &str,
        ) -> everruns_core::Result<()> {
            unimplemented!()
        }

        async fn get_secret(
            &self,
            session_id: SessionId,
            name: &str,
        ) -> everruns_core::Result<Option<String>> {
            Ok(self
                .secrets
                .lock()
                .expect("test secrets lock poisoned")
                .get(&(session_id.to_string(), name.to_string()))
                .cloned())
        }

        async fn delete_secret(
            &self,
            _session_id: SessionId,
            _name: &str,
        ) -> everruns_core::Result<bool> {
            unimplemented!()
        }

        async fn list_secrets(
            &self,
            _session_id: SessionId,
        ) -> everruns_core::Result<Vec<SecretInfo>> {
            unimplemented!()
        }
    }

    #[test]
    fn browserless_external_id_drops_query_string() {
        assert_eq!(
            browserless_external_id("wss://example.com/browser/abc123?token=secret"),
            "wss://example.com/browser/abc123"
        );
    }

    #[test]
    fn cleanup_input_defaults_are_reasonable() {
        let input = LeasedResourceCleanupInput::default();
        assert_eq!(input.batch_size, 25);
        assert_eq!(input.stale_after_seconds, 300);
        assert_eq!(input.retry_after_seconds, 60);
    }

    #[test]
    fn error_classifiers_match_expected_patterns() {
        assert!(is_daytona_not_found("Daytona API error (404): not found"));
        assert!(is_e2b_not_found("E2B API error (404): sandbox not found"));
        assert!(is_deno_not_found(
            "Deno sandbox delete error (404): not found"
        ));
        assert!(is_browserless_already_gone(
            "CDP WebSocket connection failed: HTTP error: 404 Not Found"
        ));
        assert!(!is_browserless_already_gone(
            "CDP WebSocket connection failed: dns error"
        ));
    }

    #[tokio::test]
    async fn e2b_cleanup_prefers_session_secret_override() {
        let session_id = SessionId::from_uuid(Uuid::nil());
        let storage =
            TestSessionStorageStore::with_secret(session_id, "E2B_API_KEY", "session-key");
        let resource = LeasedResource {
            id: Uuid::new_v4().into(),
            session_id: Some(session_id),
            provider: "e2b".to_string(),
            resource_type: "sandbox".to_string(),
            external_id: "sb_test".to_string(),
            display_name: None,
            status: LeasedResourceStatus::Active,
            owner_user_id: None,
            lease_duration_seconds: 60,
            last_touched_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            lease_expires_at: chrono::Utc::now(),
            cleanup_started_at: None,
            cleanup_completed_at: None,
            cleanup_attempts: 0,
            last_cleanup_error: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        unsafe {
            std::env::set_var("E2B_API_KEY", "env-key");
        }
        let token = resolve_e2b_api_key(&resource, &storage)
            .await
            .expect("session override should resolve");
        unsafe {
            std::env::remove_var("E2B_API_KEY");
        }

        assert_eq!(token, "session-key");
    }

    #[tokio::test]
    async fn e2b_cleanup_falls_back_to_env_key() {
        let session_id = SessionId::from_uuid(Uuid::nil());
        let storage = TestSessionStorageStore::empty();
        let resource = LeasedResource {
            id: Uuid::new_v4().into(),
            session_id: Some(session_id),
            provider: "e2b".to_string(),
            resource_type: "sandbox".to_string(),
            external_id: "sb_test".to_string(),
            display_name: None,
            status: LeasedResourceStatus::Active,
            owner_user_id: None,
            lease_duration_seconds: 60,
            last_touched_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            lease_expires_at: chrono::Utc::now(),
            cleanup_started_at: None,
            cleanup_completed_at: None,
            cleanup_attempts: 0,
            last_cleanup_error: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        unsafe {
            std::env::set_var("E2B_API_KEY", "env-key");
        }
        let token = resolve_e2b_api_key(&resource, &storage)
            .await
            .expect("env fallback should resolve");
        unsafe {
            std::env::remove_var("E2B_API_KEY");
        }

        assert_eq!(token, "env-key");
    }

    #[tokio::test]
    async fn e2b_cleanup_rejects_blank_env_key() {
        let session_id = SessionId::from_uuid(Uuid::nil());
        let storage = TestSessionStorageStore::empty();
        let resource = LeasedResource {
            id: Uuid::new_v4().into(),
            session_id: Some(session_id),
            provider: "e2b".to_string(),
            resource_type: "sandbox".to_string(),
            external_id: "sb_test".to_string(),
            display_name: None,
            status: LeasedResourceStatus::Active,
            owner_user_id: None,
            lease_duration_seconds: 60,
            last_touched_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            lease_expires_at: chrono::Utc::now(),
            cleanup_started_at: None,
            cleanup_completed_at: None,
            cleanup_attempts: 0,
            last_cleanup_error: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        unsafe {
            std::env::set_var("E2B_API_KEY", "   ");
        }
        let error = resolve_e2b_api_key(&resource, &storage)
            .await
            .expect_err("blank env token should fail");
        unsafe {
            std::env::remove_var("E2B_API_KEY");
        }

        assert!(error.to_string().contains("empty/whitespace"));
    }

    #[test]
    fn deno_cleanup_org_prefers_metadata() {
        let resource = LeasedResource {
            id: uuid::Uuid::nil().into(),
            session_id: None,
            provider: "deno".to_string(),
            resource_type: "sandbox".to_string(),
            external_id: "sb_123".to_string(),
            display_name: None,
            status: everruns_core::LeasedResourceStatus::Active,
            owner_user_id: None,
            lease_duration_seconds: 60,
            last_touched_at: chrono::Utc::now(),
            lease_expires_at: chrono::Utc::now(),
            cleanup_started_at: None,
            cleanup_completed_at: None,
            cleanup_attempts: 0,
            last_cleanup_error: None,
            metadata: serde_json::json!({
                "region": "ord",
                "org": "everruns",
            }),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        assert_eq!(deno_cleanup_org(&resource).as_deref(), Some("everruns"));
    }
}
