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
        fail_reads: bool,
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
            if self.fail_reads {
                return Err(crate::AgentLoopError::store("leased unavailable"));
            }
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
        fail_reads: bool,
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
            if self.fail_reads {
                return Err(crate::AgentLoopError::store("registry unavailable"));
            }
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

    async fn register(
        registry: &TestSessionResourceRegistry,
        session: SessionId,
        id: &str,
        metadata: serde_json::Value,
    ) {
        registry
            .register(RegisterSessionResource {
                session_id: session,
                resource_id: id.into(),
                kind: "sandbox".into(),
                display_name: "sandbox".into(),
                status: SessionResourceStatus::Active,
                metadata,
            })
            .await
            .unwrap();
    }

    fn metadata(provider: &str, kind: &str, external: &str) -> serde_json::Value {
        json!({"leased_resource_provider":provider,"leased_resource_type":kind,"leased_resource_external_id":external})
    }

    #[tokio::test]
    async fn list_owned_ids_prefers_leased_resources_and_filters_exactly() {
        let session = SessionId::from_seed(1);
        let store = Arc::new(TestLeasedResourceStore::default());
        for (owner, provider, kind, id, status) in [
            (
                session,
                "daytona",
                "sandbox",
                "owned",
                LeasedResourceStatus::Active,
            ),
            (
                session,
                "daytona",
                "sandbox",
                "second",
                LeasedResourceStatus::Active,
            ),
            (
                session,
                "daytona",
                "sandbox",
                "owned",
                LeasedResourceStatus::Active,
            ),
            (
                SessionId::from_seed(2),
                "daytona",
                "sandbox",
                "foreign",
                LeasedResourceStatus::Active,
            ),
            (
                session,
                "other",
                "sandbox",
                "provider",
                LeasedResourceStatus::Active,
            ),
            (
                session,
                "daytona",
                "other",
                "kind",
                LeasedResourceStatus::Active,
            ),
            (
                session,
                "daytona",
                "sandbox",
                "cleaning",
                LeasedResourceStatus::Cleaning,
            ),
            (
                session,
                "daytona",
                "sandbox",
                "released",
                LeasedResourceStatus::Released,
            ),
            (
                session,
                "daytona",
                "sandbox",
                "failed",
                LeasedResourceStatus::CleanupFailed,
            ),
        ] {
            store
                .upsert_resource(UpsertLeasedResource {
                    session_id: owner,
                    provider: provider.into(),
                    resource_type: kind.into(),
                    external_id: id.into(),
                    display_name: None,
                    owner_user_id: None,
                    lease_duration_seconds: 60,
                    metadata: json!({}),
                })
                .await
                .unwrap();
            store.resources.lock().await.last_mut().unwrap().status = status;
        }
        let registry = Arc::new(TestSessionResourceRegistry {
            fail_reads: true,
            ..Default::default()
        });
        let context = ToolContext::new(session)
            .with_leased_resource_store(store)
            .with_session_resource_registry(registry);
        assert_eq!(
            list_owned_external_resource_ids(&context, "daytona", "sandbox")
                .await
                .unwrap(),
            Some(HashSet::from(["owned".into(), "second".into()]))
        );
        assert!(
            require_owned_external_resource(&context, "daytona", "sandbox", "owned")
                .await
                .is_ok()
        );
        assert!(
            verify_owned_external_resource_if_available(&context, "daytona", "sandbox", "second")
                .await
                .is_ok()
        );
        assert!(
            matches!(require_owned_external_resource(&context, "daytona", "sandbox", "foreign").await, Err(ToolExecutionResult::ToolError(message)) if message=="Resource foreign was not created by this session")
        );
    }

    #[tokio::test]
    async fn fallback_uses_literal_metadata_and_active_session_filter() {
        let session = SessionId::from_seed(1);
        let registry = Arc::new(TestSessionResourceRegistry::default());
        for (owner, id, data) in [
            (session, "a", metadata("daytona", "sandbox", "owned")),
            (session, "b", metadata("daytona", "sandbox", "owned")),
            (session, "c", metadata("other", "sandbox", "other-provider")),
            (session, "d", metadata("daytona", "other", "other-kind")),
            (
                SessionId::from_seed(2),
                "e",
                metadata("daytona", "sandbox", "foreign"),
            ),
        ] {
            register(&registry, owner, id, data).await;
        }
        register(&registry, session, "inactive", json!({})).await;
        registry.entries.lock().await.last_mut().unwrap().status = SessionResourceStatus::Released;
        register(&registry, session, "different-kind", json!({})).await;
        registry.entries.lock().await.last_mut().unwrap().kind = "other".into();
        let context = ToolContext::new(session).with_session_resource_registry(registry);
        assert_eq!(
            list_owned_external_resource_ids(&context, "daytona", "sandbox")
                .await
                .unwrap(),
            Some(HashSet::from(["owned".into()]))
        );
        assert!(
            require_owned_external_resource(&context, "daytona", "sandbox", "owned")
                .await
                .is_ok()
        );
        assert!(
            matches!(verify_owned_external_resource_if_available(&context, "daytona", "sandbox", "missing").await, Err(ToolExecutionResult::ToolError(message)) if message=="Resource missing was not created by this session")
        );
    }

    #[tokio::test]
    async fn missing_or_legacy_tracking_distinguishes_optional_and_required_guards() {
        let session = SessionId::from_seed(1);
        let mut contexts = vec![ToolContext::new(session)];
        for malformed in [
            json!(null),
            json!({}),
            json!({"leased_resource_provider":42}),
            json!({"leased_resource_provider":"daytona"}),
            json!({"leased_resource_provider":"daytona","leased_resource_type":"sandbox","leased_resource_external_id":false}),
        ] {
            let registry = Arc::new(TestSessionResourceRegistry::default());
            register(
                &registry,
                session,
                "known",
                metadata("daytona", "sandbox", "owned"),
            )
            .await;
            register(&registry, session, "legacy", malformed).await;
            contexts.push(ToolContext::new(session).with_session_resource_registry(registry));
        }
        for context in contexts {
            assert_eq!(
                list_owned_external_resource_ids(&context, "daytona", "sandbox")
                    .await
                    .unwrap(),
                None
            );
            assert!(
                verify_owned_external_resource_if_available(
                    &context, "daytona", "sandbox", "unknown"
                )
                .await
                .is_ok()
            );
            assert!(
                matches!(require_owned_external_resource(&context, "daytona", "sandbox", "unknown").await, Err(ToolExecutionResult::ToolError(message)) if message=="Session resource tracking is unavailable; cannot verify ownership for daytona sandbox resources")
            );
        }
    }

    #[tokio::test]
    async fn empty_authoritative_store_does_not_fall_back_or_grant_ownership() {
        let session = SessionId::from_seed(1);
        let registry = Arc::new(TestSessionResourceRegistry::default());
        register(
            &registry,
            session,
            "fallback",
            metadata("daytona", "sandbox", "fallback"),
        )
        .await;
        let context = ToolContext::new(session)
            .with_leased_resource_store(Arc::new(TestLeasedResourceStore::default()))
            .with_session_resource_registry(registry);
        assert_eq!(
            list_owned_external_resource_ids(&context, "daytona", "sandbox")
                .await
                .unwrap(),
            Some(HashSet::new())
        );
        for result in [
            require_owned_external_resource(&context, "daytona", "sandbox", "fallback").await,
            verify_owned_external_resource_if_available(&context, "daytona", "sandbox", "fallback")
                .await,
        ] {
            assert!(
                matches!(result, Err(ToolExecutionResult::ToolError(message)) if message=="Resource fallback was not created by this session")
            );
        }
    }

    #[tokio::test]
    async fn storage_failures_stay_internal_and_never_degrade_to_untracked() {
        let session = SessionId::from_seed(1);
        let store = Arc::new(TestLeasedResourceStore {
            fail_reads: true,
            ..Default::default()
        });
        let registry = Arc::new(TestSessionResourceRegistry {
            fail_reads: true,
            ..Default::default()
        });
        for (context, expected) in [
            (
                ToolContext::new(session)
                    .with_leased_resource_store(store)
                    .with_session_resource_registry(Arc::new(
                        TestSessionResourceRegistry::default(),
                    )),
                "Failed to read leased resources: Message store error: leased unavailable",
            ),
            (
                ToolContext::new(session).with_session_resource_registry(registry),
                "Failed to read session resources: Message store error: registry unavailable",
            ),
        ] {
            for result in [
                list_owned_external_resource_ids(&context, "daytona", "sandbox")
                    .await
                    .map(|_| ()),
                require_owned_external_resource(&context, "daytona", "sandbox", "id").await,
                verify_owned_external_resource_if_available(&context, "daytona", "sandbox", "id")
                    .await,
            ] {
                match result {
                    Err(ToolExecutionResult::InternalError(error)) => {
                        assert_eq!(error.message, expected)
                    }
                    other => panic!("expected internal storage error, got {other:?}"),
                }
            }
        }
    }
}
