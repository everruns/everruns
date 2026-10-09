//! The worker's leased resource store, as internal commands.
//!
//! Decision: the runtime's `LeasedResourceStore` (sandboxes and other
//! provider-owned remote state that tools create, touch and release) reaches
//! these through `ExecuteCommand` (gRPC workers) and `dispatch` (the in-process
//! worker), so one implementation serves both. They used to be three bespoke
//! RPCs that took the session alone; as internal commands they run as the
//! worker's org and first confirm the session belongs to it. Upsert and release
//! still mirror into the session resource registry, as the RPCs' store did.
//!
//! The cleanup sweeper's `ClaimDueLeasedResources`,
//! `MarkLeasedResourceReleased` and `MarkLeasedResourceCleanupFailed` stay
//! dedicated RPCs: the sweeper claims due leases across every org and settles
//! them by resource id, outside any one org's turn, so there is no org for a
//! command to run as.

use super::owned_session;
use crate::domains::common::*;
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW};
use crate::kernel_imports::{
    LeasedResource, UpsertLeasedResource, session_services::LeasedResourceStore,
};
use crate::storage::DbLeasedResourceStore;
use crate::storage::runtime::session_resource::DbSessionResourceRegistry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

fn store(ctx: &Ctx) -> DbLeasedResourceStore {
    DbLeasedResourceStore::new(ctx.db.clone())
        .with_registry(Arc::new(DbSessionResourceRegistry::new(ctx.db.clone())))
}

fn store_failure(error: impl std::fmt::Display) -> CommandError {
    CommandError::internal(anyhow::anyhow!("{error}"))
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerUpsertLeasedResource {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// External provider responsible for the resource.
    pub provider: String,
    /// Provider-specific resource type.
    pub resource_type: String,
    /// Stable provider identifier for cleanup calls.
    pub external_id: String,
    /// Optional user-facing label.
    #[serde(default)]
    pub display_name: Option<String>,
    /// User connection owner used for provider cleanup.
    #[serde(default)]
    pub owner_user_id: Option<Uuid>,
    /// Exact provider connection used to create the resource.
    #[serde(default)]
    pub connection_id: Option<Uuid>,
    /// Lease duration used when refreshing the lease.
    pub lease_duration_seconds: u32,
    /// Provider-specific non-secret metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[command(
    name = "worker_upsert_leased_resource",
    category = "session_resources",
    description = "Internal: create or refresh a session's leased resource.",
    method = "POST",
    path = "/internal/sessions/{session_id}/leased-resources",
    policy = SESSION_MANAGE
)]
impl Command for WorkerUpsertLeasedResource {
    type Output = LeasedResource;

    async fn execute(self, ctx: &Ctx) -> Result<LeasedResource, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        store(ctx)
            .upsert_resource(UpsertLeasedResource {
                session_id,
                provider: self.provider,
                resource_type: self.resource_type,
                external_id: self.external_id,
                display_name: self.display_name,
                owner_user_id: self.owner_user_id,
                connection_id: self.connection_id,
                lease_duration_seconds: self.lease_duration_seconds,
                metadata: self.metadata,
            })
            .await
            .map_err(store_failure)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerReleaseLeasedResource {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// External provider responsible for the resource.
    pub provider: String,
    /// Provider-specific resource type.
    pub resource_type: String,
    /// Stable provider identifier.
    pub external_id: String,
}

#[command(
    name = "worker_release_leased_resource",
    category = "session_resources",
    description = "Internal: release a session's leased resource; null when there is none.",
    method = "POST",
    path = "/internal/sessions/{session_id}/leased-resources/release",
    policy = SESSION_MANAGE
)]
impl Command for WorkerReleaseLeasedResource {
    type Output = Option<LeasedResource>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<LeasedResource>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        store(ctx)
            .release_resource(
                session_id,
                &self.provider,
                &self.resource_type,
                &self.external_id,
            )
            .await
            .map_err(store_failure)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerListSessionLeasedResources {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "worker_list_session_leased_resources",
    category = "session_resources",
    description = "Internal: list a session's leased resources, released ones included.",
    method = "GET",
    path = "/internal/sessions/{session_id}/leased-resources",
    policy = SESSION_VIEW
)]
impl Command for WorkerListSessionLeasedResources {
    type Output = Vec<LeasedResource>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<LeasedResource>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        store(ctx)
            .list_resources(session_id)
            .await
            .map_err(store_failure)
    }
}

#[cfg(test)]
pub(super) mod tests;
