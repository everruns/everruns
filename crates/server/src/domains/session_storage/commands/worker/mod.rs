//! The worker's session key/value storage, as internal commands.
//!
//! Decision: the runtime's `SessionStorageStore` value half (kv_store tools,
//! sandbox and browser state, tool approvals) reaches these through
//! `ExecuteCommand` (gRPC workers) and `dispatch` (the in-process worker), so
//! one implementation serves both. They used to be five bespoke RPCs that took
//! the session alone; as internal commands they run as the worker's org and
//! first confirm the session belongs to it. Unlike the public
//! `list_session_storage`, they do not hide internal keys: the runtime owns
//! them.
//!
//! The secret half is `secrets`: the same shape, but encrypted at rest and kept
//! out of logs and entity history.

use super::q;
use crate::domains::common::*;
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW};
use crate::kernel_imports::contracts::typed_id::SessionId;
use crate::storage::UpsertSessionKeyValue;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The session, after confirming it belongs to the caller's org: the store
/// addresses rows by session alone.
async fn owned_session(ctx: &Ctx, session_id: &str) -> Result<SessionId, CommandError> {
    let session_id = q::parse_owned_session_id(session_id)?;
    q::verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;
    Ok(session_id)
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerSetSessionStorageValue {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Key to create or overwrite.
    pub key: String,
    /// Plain-text value.
    pub value: String,
}

#[command(
    name = "worker_set_session_storage_value",
    category = "session_storage",
    description = "Internal: create or overwrite a session storage value.",
    method = "PUT",
    path = "/internal/sessions/{session_id}/storage/values/{key}",
    policy = SESSION_MANAGE
)]
impl Command for WorkerSetSessionStorageValue {
    type Output = ();

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        ctx.db
            .upsert_session_key_value(UpsertSessionKeyValue {
                session_id,
                key: self.key,
                value: self.value,
            })
            .await
            .map_err(CommandError::internal)?;
        Ok(())
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetSessionStorageValue {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Key to read.
    pub key: String,
}

#[command(
    name = "worker_get_session_storage_value",
    category = "session_storage",
    description = "Internal: read a session storage value; null when the key is absent.",
    method = "GET",
    path = "/internal/sessions/{session_id}/storage/values/{key}",
    policy = SESSION_VIEW
)]
impl Command for WorkerGetSessionStorageValue {
    type Output = Option<String>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<String>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        Ok(ctx
            .db
            .get_session_key_value(session_id.uuid(), &self.key)
            .await
            .map_err(CommandError::internal)?
            .map(|row| row.value))
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerTakeSessionStorageValue {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Key to remove and return.
    pub key: String,
}

#[command(
    name = "worker_take_session_storage_value",
    category = "session_storage",
    description = "Internal: atomically remove and return a session storage value; null when absent.",
    method = "POST",
    path = "/internal/sessions/{session_id}/storage/values/{key}/take",
    policy = SESSION_MANAGE
)]
impl Command for WorkerTakeSessionStorageValue {
    type Output = Option<String>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<String>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        ctx.db
            .take_session_key_value(session_id.uuid(), &self.key)
            .await
            .map_err(CommandError::internal)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerDeleteSessionStorageValue {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Key to delete.
    pub key: String,
}

#[command(
    name = "worker_delete_session_storage_value",
    category = "session_storage",
    description = "Internal: delete a session storage value; false when it was absent.",
    method = "DELETE",
    path = "/internal/sessions/{session_id}/storage/values/{key}",
    policy = SESSION_MANAGE
)]
impl Command for WorkerDeleteSessionStorageValue {
    type Output = bool;

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        ctx.db
            .delete_session_key_value(session_id.uuid(), &self.key)
            .await
            .map_err(CommandError::internal)
    }
}

/// One stored key, without its value.
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkerSessionStorageKey {
    pub key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerListSessionStorageKeys {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "worker_list_session_storage_keys",
    category = "session_storage",
    description = "Internal: list a session's storage keys, internal ones included.",
    method = "GET",
    path = "/internal/sessions/{session_id}/storage/values",
    policy = SESSION_VIEW
)]
impl Command for WorkerListSessionStorageKeys {
    type Output = Vec<WorkerSessionStorageKey>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<WorkerSessionStorageKey>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        Ok(ctx
            .db
            .list_session_keys(session_id.uuid())
            .await
            .map_err(CommandError::internal)?
            .into_iter()
            .map(|row| WorkerSessionStorageKey {
                key: row.key,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect())
    }
}

mod secrets;
pub use secrets::*;

#[cfg(test)]
mod secret_tests;
#[cfg(test)]
mod tests;
