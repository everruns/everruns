//! The worker's session secrets, as internal commands.
//!
//! Decision: the runtime's `SessionStorageStore` secret half (secret_store
//! tools, sandbox and provider state that must not sit in plain values) used
//! to be four `SessionStorage*Secret` RPCs that took the session alone. As
//! internal commands they run as the worker's org, first confirm the session
//! belongs to it, and answer `NotFound` otherwise. Unlike the public
//! `list_session_secrets` / `delete_session_secret`, they do not hide internal
//! names: the runtime owns them.
//!
//! THREAT[TM-AUTHZ-023]: a secret's plaintext crosses only in a set's params
//! and a get's answer. Neither is logged (`Command::run` records name and org,
//! never params or output), set and delete are declared `Exempt` from entity
//! history (only `Change::Subject` captures params), and the params' `Debug`
//! redacts the value.

use crate::domains::common::*;
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW};
use crate::storage::UpsertSessionSecret;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

fn encryption(ctx: &Ctx) -> Result<&crate::storage::EncryptionService, CommandError> {
    ctx.encryption.as_deref().ok_or_else(|| {
        CommandError::bad_request(
            "Encryption not configured. Set SECRETS_ENCRYPTION_KEY environment variable.",
        )
    })
}

#[derive(Deserialize, ToSchema, Serialize)]
pub struct WorkerSetSessionSecret {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Secret name to create or overwrite.
    pub name: String,
    /// Plain-text value; encrypted before it is stored.
    pub value: String,
}

impl std::fmt::Debug for WorkerSetSessionSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerSetSessionSecret")
            .field("session_id", &self.session_id)
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .finish()
    }
}

#[command(
    name = "worker_set_session_secret",
    category = "session_storage",
    description = "Internal: encrypt and store a session secret.",
    method = "PUT",
    path = "/internal/sessions/{session_id}/storage/secrets/{name}",
    policy = SESSION_MANAGE
)]
impl Command for WorkerSetSessionSecret {
    type Output = ();

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let session_id = super::owned_session(ctx, &self.session_id).await?;
        let value_encrypted = encryption(ctx)?
            .encrypt_string(&self.value)
            .map_err(CommandError::internal)?;
        ctx.db
            .upsert_session_secret(UpsertSessionSecret {
                session_id,
                name: self.name,
                value_encrypted,
            })
            .await
            .map_err(CommandError::internal)?;
        Ok(())
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetSessionSecret {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Secret name to read.
    pub name: String,
}

#[command(
    name = "worker_get_session_secret",
    category = "session_storage",
    description = "Internal: read a decrypted session secret; null when the name is absent.",
    method = "GET",
    path = "/internal/sessions/{session_id}/storage/secrets/{name}",
    policy = SESSION_VIEW
)]
impl Command for WorkerGetSessionSecret {
    type Output = Option<String>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<String>, CommandError> {
        let session_id = super::owned_session(ctx, &self.session_id).await?;
        let encryption = encryption(ctx)?;
        let Some(row) = ctx
            .db
            .get_session_secret(session_id.uuid(), &self.name)
            .await
            .map_err(CommandError::internal)?
        else {
            return Ok(None);
        };
        encryption
            .decrypt_to_string(&row.value_encrypted)
            .map(Some)
            .map_err(CommandError::internal)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerDeleteSessionSecret {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Secret name to delete.
    pub name: String,
}

#[command(
    name = "worker_delete_session_secret",
    category = "session_storage",
    description = "Internal: delete a session secret; false when it was absent.",
    method = "DELETE",
    path = "/internal/sessions/{session_id}/storage/secrets/{name}",
    policy = SESSION_MANAGE
)]
impl Command for WorkerDeleteSessionSecret {
    type Output = bool;

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let session_id = super::owned_session(ctx, &self.session_id).await?;
        ctx.db
            .delete_session_secret(session_id.uuid(), &self.name)
            .await
            .map_err(CommandError::internal)
    }
}

/// One stored secret, without its value.
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkerSessionSecret {
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerListSessionSecrets {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "worker_list_session_secrets",
    category = "session_storage",
    description = "Internal: list a session's secret names, internal ones included.",
    method = "GET",
    path = "/internal/sessions/{session_id}/storage/secrets",
    policy = SESSION_VIEW
)]
impl Command for WorkerListSessionSecrets {
    type Output = Vec<WorkerSessionSecret>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<WorkerSessionSecret>, CommandError> {
        let session_id = super::owned_session(ctx, &self.session_id).await?;
        Ok(ctx
            .db
            .list_session_secrets(session_id.uuid())
            .await
            .map_err(CommandError::internal)?
            .into_iter()
            .map(|row| WorkerSessionSecret {
                name: row.name,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect())
    }
}
