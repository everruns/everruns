//! A v2 session's resources and leases, mirrored from its shard's feed
//! (`Record::SessionResource`, `Record::Lease`).
//!
//! Decisions:
//! - The shard keeps them, with the session (`crates/agentd/src/
//!   session_records.rs`); its tools never ask the control plane. The
//!   control plane keeps a copy in the same tables a v1 session's fill, so
//!   the console's Resources panel and the lease sweeper serve both planes
//!   with one code path.
//! - A mirrored lease keeps the shard's id, so the resource the lease
//!   registered names the row the sweeper works on.
//! - Only a session of the record's org on its data plane is mirrored; any
//!   other record is skipped and logged, never applied to another org.

use everruns_contracts::runtime::session_resource::SessionResourceEntry;
use everruns_contracts::runtime::{LeasedResource, LeasedResourceStatus};
use everruns_contracts::typed_id::SessionId;
use serde_json::Value;

use super::ControlPlane;
use crate::storage::{ReleaseLeasedResourceRow, UpsertLeasedResourceRow, UpsertSessionResourceRow};

/// `session_id` when it is `org_id`'s session on its data plane.
async fn owned(
    cp: &ControlPlane,
    org_id: i64,
    session_id: &str,
) -> anyhow::Result<Option<SessionId>> {
    if cp.db.dp_session_org(session_id).await? != Some(org_id) {
        tracing::warn!(
            org_id,
            session_id,
            "a session record for a session not the org's"
        );
        return Ok(None);
    }
    Ok(Some(session_id.parse()?))
}

/// Mirror one of a session's resources, or its removal (`resource` null).
pub async fn resource(
    cp: &ControlPlane,
    org_id: i64,
    session_id: &str,
    resource_id: &str,
    resource: &Value,
) -> anyhow::Result<()> {
    let Some(session) = owned(cp, org_id, session_id).await? else {
        return Ok(());
    };
    if resource.is_null() {
        cp.db.delete_session_resource(session, resource_id).await?;
        return Ok(());
    }
    let entry: SessionResourceEntry = serde_json::from_value(resource.clone())?;
    anyhow::ensure!(
        entry.session_id == session && entry.resource_id == resource_id,
        "a resource record names another resource"
    );
    cp.db
        .upsert_session_resource(UpsertSessionResourceRow {
            session_id: session,
            resource_id: entry.resource_id,
            kind: entry.kind,
            display_name: entry.display_name,
            status: entry.status.to_string(),
            metadata: entry.metadata,
        })
        .await?;
    Ok(())
}

/// Mirror one of a session's leases.
pub async fn lease(
    cp: &ControlPlane,
    org_id: i64,
    session_id: &str,
    lease: &Value,
) -> anyhow::Result<()> {
    let Some(session) = owned(cp, org_id, session_id).await? else {
        return Ok(());
    };
    let lease: LeasedResource = serde_json::from_value(lease.clone())?;
    anyhow::ensure!(
        lease.session_id == Some(session),
        "a lease record names another session"
    );
    if lease.status == LeasedResourceStatus::Released {
        cp.db
            .release_leased_resource(ReleaseLeasedResourceRow {
                org_id,
                session_id: session,
                provider: lease.provider,
                resource_type: lease.resource_type,
                external_id: lease.external_id,
            })
            .await?;
        return Ok(());
    }
    cp.db
        .upsert_leased_resource(UpsertLeasedResourceRow {
            id: Some(lease.id),
            org_id,
            session_id: session,
            provider: lease.provider,
            resource_type: lease.resource_type,
            external_id: lease.external_id,
            display_name: lease.display_name,
            owner_user_id: lease.owner_user_id,
            connection_id: lease.connection_id,
            lease_duration_seconds: lease.lease_duration_seconds as i32,
            lease_expires_at: lease.lease_expires_at,
            metadata: swept_by_data_plane(lease.metadata),
        })
        .await?;
    Ok(())
}

/// The marker the control plane's cleanup activity skips: agentd releases
/// a v2 session's leases itself (`agentd/src/lease_sweep.rs`), and the
/// mirror only shows them.
pub const SWEPT_BY: &str = "swept_by";
pub const DATA_PLANE: &str = "data_plane";

fn swept_by_data_plane(metadata: serde_json::Value) -> serde_json::Value {
    let mut metadata = match metadata {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    metadata.insert(SWEPT_BY.into(), serde_json::Value::from(DATA_PLANE));
    serde_json::Value::Object(metadata)
}

#[cfg(test)]
mod swept_by_tests {
    use super::*;

    /// A mirrored lease carries the marker the control plane's cleanup
    /// skips, and keeps its own metadata.
    #[test]
    fn a_mirrored_lease_is_marked_as_the_data_planes_to_release() {
        let marked = swept_by_data_plane(serde_json::json!({ "region": "us" }));
        assert_eq!(marked[SWEPT_BY], DATA_PLANE);
        assert_eq!(marked["region"], "us");
        assert_eq!(
            swept_by_data_plane(serde_json::Value::Null)[SWEPT_BY],
            DATA_PLANE
        );
    }
}
