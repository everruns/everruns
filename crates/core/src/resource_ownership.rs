// Shared helpers for session-scoped ownership checks over leased resources.
//
// Decision: tools that accept provider-owned external IDs (sandbox_id,
// browserless reconnect endpoint, etc.) should resolve ownership through the
// leased-resource store when available, with session-resource metadata as a
// fallback for runtimes that only wire the generic registry.

use std::collections::HashSet;

use crate::leased_resource::LeasedResourceStatus;
use crate::session_resource::{SessionResourceFilter, SessionResourceStatus};
use crate::tool_context::ToolContext;
use crate::tools::ToolExecutionResult;

/// Reserved metadata key storing the provider name on session-resource entries
/// auto-registered from leased resources.
pub const LEASED_RESOURCE_PROVIDER_KEY: &str = "leased_resource_provider";
/// Reserved metadata key storing the provider resource type on session-resource
/// entries auto-registered from leased resources.
pub const LEASED_RESOURCE_TYPE_KEY: &str = "leased_resource_type";
/// Reserved metadata key storing the provider external ID on session-resource
/// entries auto-registered from leased resources.
pub const LEASED_RESOURCE_EXTERNAL_ID_KEY: &str = "leased_resource_external_id";
/// Reserved metadata key storing the leased-resource public ID.
pub const LEASED_RESOURCE_ID_KEY: &str = "leased_resource_id";

/// Return the set of provider-owned external IDs currently attached to the
/// active session.
///
/// Returns `Ok(None)` when the runtime has no ownership registry/store wired
/// into the tool context. Callers that already have another scoped state source
/// (such as session secrets) can degrade gracefully in that case; callers that
/// would otherwise trust raw provider IDs should reject the operation.
pub async fn list_owned_external_resource_ids(
    context: &ToolContext,
    provider: &str,
    resource_type: &str,
) -> Result<Option<HashSet<String>>, ToolExecutionResult> {
    if let Some(store) = context.leased_resource_store.as_ref() {
        let resources = store
            .list_resources(context.session_id)
            .await
            .map_err(|e| {
                ToolExecutionResult::internal_error_msg(format!(
                    "Failed to read leased resources: {e}"
                ))
            })?;
        let owned = resources
            .into_iter()
            .filter(|resource| {
                resource.provider == provider
                    && resource.resource_type == resource_type
                    && resource.status == LeasedResourceStatus::Active
            })
            .map(|resource| resource.external_id)
            .collect();
        return Ok(Some(owned));
    }

    if let Some(registry) = context.session_resource_registry.as_ref() {
        let filter = SessionResourceFilter {
            kind: Some(resource_type.to_string()),
            status: Some(SessionResourceStatus::Active),
        };
        let entries = registry
            .list(context.session_id, Some(&filter))
            .await
            .map_err(|e| {
                ToolExecutionResult::internal_error_msg(format!(
                    "Failed to read session resources: {e}"
                ))
            })?;

        let mut owned = HashSet::new();
        let mut saw_untracked_entry = false;

        for entry in entries {
            let Some(metadata) = entry.metadata.as_object() else {
                saw_untracked_entry = true;
                continue;
            };

            let Some(entry_provider) = metadata
                .get(LEASED_RESOURCE_PROVIDER_KEY)
                .and_then(|value| value.as_str())
            else {
                saw_untracked_entry = true;
                continue;
            };

            let Some(entry_resource_type) = metadata
                .get(LEASED_RESOURCE_TYPE_KEY)
                .and_then(|value| value.as_str())
            else {
                saw_untracked_entry = true;
                continue;
            };

            let Some(external_id) = metadata
                .get(LEASED_RESOURCE_EXTERNAL_ID_KEY)
                .and_then(|value| value.as_str())
            else {
                saw_untracked_entry = true;
                continue;
            };

            if entry_provider == provider && entry_resource_type == resource_type {
                owned.insert(external_id.to_owned());
            }
        }

        if saw_untracked_entry {
            return Ok(None);
        }

        return Ok(Some(owned));
    }

    Ok(None)
}

/// Check ownership when a runtime has tracking wired in, but degrade gracefully
/// when it does not.
pub async fn verify_owned_external_resource_if_available(
    context: &ToolContext,
    provider: &str,
    resource_type: &str,
    external_id: &str,
) -> Result<(), ToolExecutionResult> {
    if let Some(owned_ids) =
        list_owned_external_resource_ids(context, provider, resource_type).await?
        && !owned_ids.contains(external_id)
    {
        return Err(resource_not_owned_error(external_id));
    }

    Ok(())
}

/// Require ownership tracking to be present and reject when the external ID is
/// not attached to the active session.
pub async fn require_owned_external_resource(
    context: &ToolContext,
    provider: &str,
    resource_type: &str,
    external_id: &str,
) -> Result<(), ToolExecutionResult> {
    let Some(owned_ids) =
        list_owned_external_resource_ids(context, provider, resource_type).await?
    else {
        return Err(ownership_tracking_unavailable_error(
            provider,
            resource_type,
        ));
    };

    if owned_ids.contains(external_id) {
        Ok(())
    } else {
        Err(resource_not_owned_error(external_id))
    }
}

pub fn resource_not_owned_error(external_id: &str) -> ToolExecutionResult {
    // THREAT[TM-AGENT-020]: Block cross-session resource access via guessed or
    // stale provider IDs by rejecting any external resource handle that the
    // active session does not own.
    ToolExecutionResult::tool_error(format!(
        "Resource {external_id} was not created by this session"
    ))
}

pub fn ownership_tracking_unavailable_error(
    provider: &str,
    resource_type: &str,
) -> ToolExecutionResult {
    ToolExecutionResult::tool_error(format!(
        "Session resource tracking is unavailable; cannot verify ownership for {provider} {resource_type} resources"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::{TimeDelta, Utc};
    use serde_json::json;
    use std::sync::Arc;

    use crate::error::Result;
    use crate::leased_resource::{LeasedResource, LeasedResourceStatus, UpsertLeasedResource};
    use crate::session_resource::{RegisterSessionResource, SessionResourceEntry};
    use crate::typed_id::{LeasedResourceId, SessionId};
    use crate::{session_services::LeasedResourceStore, session_services::SessionResourceRegistry};

    #[derive(Default)]
    struct TestLeasedResourceStore {
        resources: tokio::sync::Mutex<Vec<LeasedResource>>,
    }

    #[async_trait]
    impl LeasedResourceStore for TestLeasedResourceStore {
        async fn upsert_resource(&self, input: UpsertLeasedResource) -> Result<LeasedResource> {
            let now = Utc::now();
            let resource = LeasedResource {
                id: LeasedResourceId::new(),
                session_id: Some(input.session_id),
                provider: input.provider,
                resource_type: input.resource_type,
                external_id: input.external_id,
                display_name: input.display_name,
                status: LeasedResourceStatus::Active,
                owner_user_id: input.owner_user_id,
                lease_duration_seconds: input.lease_duration_seconds,
                last_touched_at: now,
                lease_expires_at: now + TimeDelta::seconds(i64::from(input.lease_duration_seconds)),
                cleanup_started_at: None,
                cleanup_completed_at: None,
                cleanup_attempts: 0,
                last_cleanup_error: None,
                metadata: input.metadata,
                created_at: now,
                updated_at: now,
            };
            self.resources.lock().await.push(resource.clone());
            Ok(resource)
        }

        async fn release_resource(
            &self,
            _session_id: SessionId,
            _provider: &str,
            _resource_type: &str,
            _external_id: &str,
        ) -> Result<Option<LeasedResource>> {
            Ok(None)
        }

        async fn list_resources(&self, session_id: SessionId) -> Result<Vec<LeasedResource>> {
            Ok(self
                .resources
                .lock()
                .await
                .iter()
                .filter(|resource| resource.session_id == Some(session_id))
                .cloned()
                .collect())
        }
    }

    #[derive(Default)]
    struct TestSessionResourceRegistry {
        entries: tokio::sync::Mutex<Vec<SessionResourceEntry>>,
    }

    #[async_trait]
    impl SessionResourceRegistry for TestSessionResourceRegistry {
        async fn register(&self, entry: RegisterSessionResource) -> Result<SessionResourceEntry> {
            let now = Utc::now();
            let entry = SessionResourceEntry {
                resource_id: entry.resource_id,
                session_id: entry.session_id,
                kind: entry.kind,
                display_name: entry.display_name,
                status: entry.status,
                metadata: entry.metadata,
                created_at: now,
                updated_at: now,
            };
            self.entries.lock().await.push(entry.clone());
            Ok(entry)
        }

        async fn update_status(
            &self,
            _session_id: SessionId,
            _resource_id: &str,
            _status: SessionResourceStatus,
        ) -> Result<Option<SessionResourceEntry>> {
            Ok(None)
        }

        async fn get(
            &self,
            _session_id: SessionId,
            _resource_id: &str,
        ) -> Result<Option<SessionResourceEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            session_id: SessionId,
            filter: Option<&SessionResourceFilter>,
        ) -> Result<Vec<SessionResourceEntry>> {
            let entries = self.entries.lock().await;
            Ok(entries
                .iter()
                .filter(|entry| entry.session_id == session_id)
                .filter(|entry| {
                    filter
                        .and_then(|filter| filter.kind.as_deref())
                        .is_none_or(|kind| entry.kind == kind)
                })
                .filter(|entry| {
                    filter
                        .and_then(|filter| filter.status)
                        .is_none_or(|status| entry.status == status)
                })
                .cloned()
                .collect())
        }

        async fn deregister(&self, _session_id: SessionId, _resource_id: &str) -> Result<bool> {
            Ok(false)
        }
    }

    #[tokio::test]
    async fn list_owned_ids_prefers_leased_resources() {
        let session_id = SessionId::new();
        let store = Arc::new(TestLeasedResourceStore::default());
        store
            .upsert_resource(UpsertLeasedResource {
                session_id,
                provider: "daytona".to_string(),
                resource_type: "sandbox".to_string(),
                external_id: "sb_test".to_string(),
                display_name: None,
                owner_user_id: None,
                lease_duration_seconds: 60,
                metadata: json!({}),
            })
            .await
            .unwrap();

        let context = ToolContext::new(session_id).with_leased_resource_store(store);
        let owned = list_owned_external_resource_ids(&context, "daytona", "sandbox")
            .await
            .unwrap()
            .unwrap();
        assert!(owned.contains("sb_test"));
    }

    #[tokio::test]
    async fn list_owned_ids_falls_back_to_session_resources() {
        let session_id = SessionId::new();
        let registry = Arc::new(TestSessionResourceRegistry::default());
        registry
            .register(RegisterSessionResource {
                session_id,
                resource_id: "resource_1".to_string(),
                kind: "sandbox".to_string(),
                display_name: "sandbox".to_string(),
                status: SessionResourceStatus::Active,
                metadata: json!({
                    LEASED_RESOURCE_PROVIDER_KEY: "daytona",
                    LEASED_RESOURCE_TYPE_KEY: "sandbox",
                    LEASED_RESOURCE_EXTERNAL_ID_KEY: "sb_test",
                }),
            })
            .await
            .unwrap();

        let context = ToolContext::new(session_id).with_session_resource_registry(registry);
        let owned = list_owned_external_resource_ids(&context, "daytona", "sandbox")
            .await
            .unwrap()
            .unwrap();
        assert!(owned.contains("sb_test"));
    }

    #[tokio::test]
    async fn list_owned_ids_returns_none_for_legacy_session_resources() {
        let session_id = SessionId::new();
        let registry = Arc::new(TestSessionResourceRegistry::default());
        registry
            .register(RegisterSessionResource {
                session_id,
                resource_id: "resource_legacy".to_string(),
                kind: "sandbox".to_string(),
                display_name: "sandbox".to_string(),
                status: SessionResourceStatus::Active,
                metadata: json!({}),
            })
            .await
            .unwrap();

        let context = ToolContext::new(session_id).with_session_resource_registry(registry);
        let owned = list_owned_external_resource_ids(&context, "daytona", "sandbox")
            .await
            .unwrap();
        assert!(owned.is_none());
    }
}
