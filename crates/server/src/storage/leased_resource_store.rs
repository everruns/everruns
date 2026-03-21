// Database-backed leased resource store.
//
// Decision: Leased resources are the generic persistence layer for any
// provider-owned remote state that must eventually be cleaned.
// Decision: Tool-side operations only expose create/touch/release/list.
// Cleanup claiming and retry transitions stay in storage/backend APIs because
// they are server/worker control-plane concerns, not tool concerns.

use async_trait::async_trait;
use chrono::{TimeDelta, Utc};
use everruns_core::traits::LeasedResourceStore;
use everruns_core::{
    AgentLoopError, LeasedResource, LeasedResourceStatus, Result, SessionId, UpsertLeasedResource,
};

use super::backend::StorageBackend;
use super::models::{LeasedResourceRow, ReleaseLeasedResourceRow, UpsertLeasedResourceRow};
use std::sync::Arc;

/// Convert a storage row into the public leased-resource domain type.
pub fn row_to_domain(row: &LeasedResourceRow) -> Result<LeasedResource> {
    let status = match row.status.as_str() {
        "active" => LeasedResourceStatus::Active,
        "cleaning" => LeasedResourceStatus::Cleaning,
        "released" => LeasedResourceStatus::Released,
        "cleanup_failed" => LeasedResourceStatus::CleanupFailed,
        other => {
            return Err(AgentLoopError::store(format!(
                "Unknown leased resource status: {other}"
            )));
        }
    };

    Ok(LeasedResource {
        id: row.id,
        session_id: row.session_id,
        provider: row.provider.clone(),
        resource_type: row.resource_type.clone(),
        external_id: row.external_id.clone(),
        display_name: row.display_name.clone(),
        status,
        owner_user_id: row.owner_user_id,
        lease_duration_seconds: row.lease_duration_seconds as u32,
        last_touched_at: row.last_touched_at,
        lease_expires_at: row.lease_expires_at,
        cleanup_started_at: row.cleanup_started_at,
        cleanup_completed_at: row.cleanup_completed_at,
        cleanup_attempts: row.cleanup_attempts as u32,
        last_cleanup_error: row.last_cleanup_error.clone(),
        metadata: row.metadata.clone(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Database-backed leased resource store.
///
/// This adapter is injected into tool execution contexts so any capability that
/// creates remote state can opt into the same leased-resource cleanup model.
#[derive(Clone)]
pub struct DbLeasedResourceStore {
    db: Arc<StorageBackend>,
}

impl DbLeasedResourceStore {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl LeasedResourceStore for DbLeasedResourceStore {
    async fn upsert_resource(&self, input: UpsertLeasedResource) -> Result<LeasedResource> {
        let org_id = self
            .db
            .get_session_organization_id(input.session_id)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve session org: {e}")))?
            .ok_or_else(|| {
                AgentLoopError::store("Session not found for leased resource".to_string())
            })?;

        let lease_expires_at =
            Utc::now() + TimeDelta::seconds(i64::from(input.lease_duration_seconds));

        let row = self
            .db
            .upsert_leased_resource(UpsertLeasedResourceRow {
                org_id,
                session_id: input.session_id,
                provider: input.provider,
                resource_type: input.resource_type,
                external_id: input.external_id,
                display_name: input.display_name,
                owner_user_id: input.owner_user_id,
                lease_duration_seconds: input.lease_duration_seconds as i32,
                lease_expires_at,
                metadata: input.metadata,
            })
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to upsert leased resource: {e}")))?;

        row_to_domain(&row)
    }

    async fn release_resource(
        &self,
        session_id: SessionId,
        provider: &str,
        resource_type: &str,
        external_id: &str,
    ) -> Result<Option<LeasedResource>> {
        let Some(org_id) = self
            .db
            .get_session_organization_id(session_id)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve session org: {e}")))?
        else {
            return Ok(None);
        };

        let row = self
            .db
            .release_leased_resource(ReleaseLeasedResourceRow {
                org_id,
                session_id,
                provider: provider.to_string(),
                resource_type: resource_type.to_string(),
                external_id: external_id.to_string(),
            })
            .await
            .map_err(|e| {
                AgentLoopError::store(format!("Failed to release leased resource: {e}"))
            })?;

        row.as_ref().map(row_to_domain).transpose()
    }

    async fn list_resources(&self, session_id: SessionId) -> Result<Vec<LeasedResource>> {
        let rows = self
            .db
            .list_session_leased_resources(session_id)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to list leased resources: {e}")))?;

        rows.iter().map(row_to_domain).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{StorageBackend, models::CreateSessionRow};
    use everruns_core::DEFAULT_ORG_ID;
    use serde_json::json;

    async fn create_test_session(db: &StorageBackend) -> SessionId {
        db.create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            harness_id: None,
            agent_id: None,
            agent_identity_id: None,
            title: Some("Lease test session".to_string()),
            locale: None,
            tags: Vec::new(),
            model_id: None,
            capabilities: json!([]),
            tools: json!([]),
            hints: None,
        })
        .await
        .expect("test session should be created")
        .id
    }

    #[tokio::test]
    async fn upsert_list_and_release_round_trip() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_test_session(db.as_ref()).await;
        let store = DbLeasedResourceStore::new(db.clone());

        let created = store
            .upsert_resource(UpsertLeasedResource {
                session_id,
                provider: "daytona".to_string(),
                resource_type: "sandbox".to_string(),
                external_id: "sandbox-123".to_string(),
                display_name: Some("Build sandbox".to_string()),
                owner_user_id: None,
                lease_duration_seconds: 20 * 60,
                metadata: json!({ "workspace_path": "/workspace" }),
            })
            .await
            .expect("leased resource should be created");

        assert_eq!(created.status, LeasedResourceStatus::Active);
        assert_eq!(created.session_id, Some(session_id));

        let touched = store
            .upsert_resource(UpsertLeasedResource {
                session_id,
                provider: "daytona".to_string(),
                resource_type: "sandbox".to_string(),
                external_id: "sandbox-123".to_string(),
                display_name: Some("Renamed sandbox".to_string()),
                owner_user_id: None,
                lease_duration_seconds: 30 * 60,
                metadata: json!({ "workspace_path": "/workspace", "branch": "main" }),
            })
            .await
            .expect("leased resource should be refreshed");

        assert_eq!(touched.id, created.id);
        assert_eq!(touched.display_name.as_deref(), Some("Renamed sandbox"));
        assert_eq!(touched.lease_duration_seconds, 30 * 60);
        assert_eq!(touched.metadata["branch"], "main");

        let listed = store
            .list_resources(session_id)
            .await
            .expect("leased resources should list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);
        assert_eq!(listed[0].status, LeasedResourceStatus::Active);

        let released = store
            .release_resource(session_id, "daytona", "sandbox", "sandbox-123")
            .await
            .expect("leased resource should release")
            .expect("leased resource should exist");

        assert_eq!(released.id, created.id);
        assert_eq!(released.status, LeasedResourceStatus::Released);
        assert!(released.cleanup_completed_at.is_some());
    }

    #[tokio::test]
    async fn stale_cleanup_transition_is_rejected_after_refresh() {
        let db = Arc::new(StorageBackend::in_memory());
        let session_id = create_test_session(db.as_ref()).await;
        let store = DbLeasedResourceStore::new(db.clone());

        let created = store
            .upsert_resource(UpsertLeasedResource {
                session_id,
                provider: "browserless".to_string(),
                resource_type: "browser_session".to_string(),
                external_id: "wss://example.com/browser/abc".to_string(),
                display_name: Some("Persistent browser".to_string()),
                owner_user_id: None,
                lease_duration_seconds: 0,
                metadata: json!({ "ws_endpoint": "wss://example.com/browser/abc" }),
            })
            .await
            .expect("leased resource should be created");

        let claimed = db
            .claim_due_leased_resources(10, 300)
            .await
            .expect("resource should be claimable");
        assert_eq!(claimed.len(), 1);
        let cleanup_started_at = claimed[0]
            .cleanup_started_at
            .expect("claimed resource should have cleanup_started_at");

        let refreshed = store
            .upsert_resource(UpsertLeasedResource {
                session_id,
                provider: "browserless".to_string(),
                resource_type: "browser_session".to_string(),
                external_id: "wss://example.com/browser/abc".to_string(),
                display_name: Some("Persistent browser".to_string()),
                owner_user_id: None,
                lease_duration_seconds: 20 * 60,
                metadata: json!({ "ws_endpoint": "wss://example.com/browser/abc" }),
            })
            .await
            .expect("leased resource should refresh");

        assert_eq!(refreshed.id, created.id);
        assert_eq!(refreshed.status, LeasedResourceStatus::Active);
        assert!(refreshed.cleanup_started_at.is_none());

        let released = db
            .mark_leased_resource_released(created.id, cleanup_started_at)
            .await
            .expect("stale release transition should be checked");
        assert!(released.is_none());

        let failed = db
            .mark_leased_resource_cleanup_failed(created.id, cleanup_started_at, 60, "stale")
            .await
            .expect("stale failure transition should be checked");
        assert!(failed.is_none());
    }
}
