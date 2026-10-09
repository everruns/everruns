use super::queries as q;
use crate::domains::common::*;
use crate::domains::session_storage::queries::verify_session_ownership;
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW};
use crate::kernel_imports::contracts::typed_id::SessionId;
use crate::storage::runtime::session_resource::DbSessionResourceRegistry;
use everruns_core::session_services::SessionResourceRegistry;
use everruns_core::{
    RegisterSessionResource, SessionResourceEntry, SessionResourceFilter, SessionResourceStatus,
};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListSessionResources {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "list_session_resources",
    category = "session_resources",
    description = "List all resources registered in a session.",
    method = "GET",
    path = "/v1/sessions/{session_id}/resources",
    positional = "session_id"
)]
impl Command for ListSessionResources {
    type Output = Vec<SessionResourceEntry>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SessionResourceEntry>, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::list_for_session(&ctx.db, ctx.org_id(), session_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session"))
    }
}

// ============================================================================
// Internal: the worker's session resource registry
// ============================================================================
//
// Decision: the runtime's `SessionResourceRegistry` (sandboxes, skills, and
// other capabilities registering what a session holds) reaches these through
// `ExecuteCommand` (gRPC workers) and `dispatch` (the in-process worker), so one
// implementation serves both. They used to be four bespoke RPCs that took the
// session alone; as internal commands (`INTERNAL_COMMAND_PATH_PREFIX`) they run
// as the worker's org and first confirm the session belongs to it. `get` stays
// a filtered list on the worker side, as it was over gRPC.

fn registry(ctx: &Ctx) -> DbSessionResourceRegistry {
    DbSessionResourceRegistry::new(ctx.db.clone())
}

fn registry_failure(error: impl std::fmt::Display) -> CommandError {
    CommandError::internal(anyhow::anyhow!("{error}"))
}

/// The session, after confirming it belongs to the caller's org: the registry
/// addresses rows by session alone.
async fn owned_session(ctx: &Ctx, session_id: &str) -> Result<SessionId, CommandError> {
    let session_id = q::parse_session_id(session_id)?;
    verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;
    Ok(session_id)
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct WorkerRegisterSessionResource {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Caller-provided stable ID, unique per session.
    pub resource_id: String,
    /// Resource kind: "sandbox", "subagent", "browser_session", etc.
    pub kind: String,
    /// Human-readable label.
    pub display_name: String,
    /// Initial status.
    pub status: SessionResourceStatus,
    /// Kind-specific non-secret metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[command(
    name = "worker_register_session_resource",
    category = "session_resources",
    description = "Internal: register or replace a resource in a session's registry.",
    method = "POST",
    path = "/internal/sessions/{session_id}/resources",
    policy = SESSION_MANAGE
)]
impl Command for WorkerRegisterSessionResource {
    type Output = SessionResourceEntry;

    async fn execute(self, ctx: &Ctx) -> Result<SessionResourceEntry, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        registry(ctx)
            .register(RegisterSessionResource {
                session_id,
                resource_id: self.resource_id,
                kind: self.kind,
                display_name: self.display_name,
                status: self.status,
                metadata: self.metadata,
            })
            .await
            .map_err(registry_failure)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct WorkerUpdateSessionResourceStatus {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// The resource's caller-provided ID.
    pub resource_id: String,
    /// New lifecycle status.
    pub status: SessionResourceStatus,
}

#[command(
    name = "worker_update_session_resource_status",
    category = "session_resources",
    description = "Internal: set a session resource's status; null when it is not registered.",
    method = "POST",
    path = "/internal/sessions/{session_id}/resources/{resource_id}/status",
    policy = SESSION_MANAGE
)]
impl Command for WorkerUpdateSessionResourceStatus {
    type Output = Option<SessionResourceEntry>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<SessionResourceEntry>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        registry(ctx)
            .update_status(session_id, &self.resource_id, self.status)
            .await
            .map_err(registry_failure)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct WorkerListSessionResources {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Only resources of this kind.
    #[serde(default)]
    pub kind: Option<String>,
    /// Only resources in this status.
    #[serde(default)]
    pub status: Option<SessionResourceStatus>,
}

#[command(
    name = "worker_list_session_resources",
    category = "session_resources",
    description = "Internal: list a session's registered resources, optionally filtered.",
    method = "GET",
    path = "/internal/sessions/{session_id}/resources",
    policy = SESSION_VIEW
)]
impl Command for WorkerListSessionResources {
    type Output = Vec<SessionResourceEntry>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SessionResourceEntry>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        let filter =
            (self.kind.is_some() || self.status.is_some()).then_some(SessionResourceFilter {
                kind: self.kind,
                status: self.status,
            });
        registry(ctx)
            .list(session_id, filter.as_ref())
            .await
            .map_err(registry_failure)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct WorkerDeregisterSessionResource {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// The resource's caller-provided ID.
    pub resource_id: String,
}

#[command(
    name = "worker_deregister_session_resource",
    category = "session_resources",
    description = "Internal: remove a resource from a session's registry; false when absent.",
    method = "DELETE",
    path = "/internal/sessions/{session_id}/resources/{resource_id}",
    policy = SESSION_MANAGE
)]
impl Command for WorkerDeregisterSessionResource {
    type Output = bool;

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        registry(ctx)
            .deregister(session_id, &self.resource_id)
            .await
            .map_err(registry_failure)
    }
}

mod leased;
pub use leased::*;

#[cfg(test)]
mod tests;
