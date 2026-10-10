//! The worker's provider-connection lookups, as internal commands.
//!
//! Decision: a runtime tool's `UserConnectionResolver` (GitHub, Daytona,
//! service API keys, leased-resource cleanup) reaches these through
//! `ExecuteCommand` (gRPC workers) and `dispatch` (the in-process worker), so
//! one implementation serves both. They used to be bespoke RPCs that took a
//! session or a virtual user alone; as internal commands they run as the
//! worker's org, confirm the session or virtual user belongs to it, and answer
//! `NotFound` otherwise. Resolution itself is the server's resolver, unchanged:
//! the same decryption, GitHub App minting and OAuth refresh (with its
//! single-flight lock and write-back of the refreshed grant).
//!
//! THREAT[TM-AUTHZ-023]: the answers are decrypted credentials. All of these
//! are reads (`GET`), so entity history records nothing; `Command::run` logs
//! the command's name and org, never its output; failures carry fixed text.
//! The MCP grant lookups, which also re-validate the attachment, live with the
//! MCP domain (`mcp_servers::commands::worker`).

use crate::domains::common::*;
use crate::domains::session_storage::queries::{parse_owned_session_id, verify_session_ownership};
use crate::domains::sessions::SESSION_VIEW;
use crate::domains::virtual_users::VIRTUAL_USER_VIEW;
use crate::kernel_imports::contracts::typed_id::{SessionId, VirtualUserId};
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_core::connection_services::UserConnectionResolver;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// The server's connection resolver, or `Unavailable` when the deployment
/// cannot decrypt grants (no encryption key).
pub(crate) fn resolver(ctx: &Ctx) -> Result<Arc<dyn UserConnectionResolver>, CommandError> {
    ctx.connection_resolver
        .clone()
        .ok_or_else(|| CommandError::unavailable("Connection resolver not available"))
}

/// The resolver bound to the invocation the worker named, when it named one.
pub(crate) fn bound_resolver(
    ctx: &Ctx,
    input_message_id: Option<Uuid>,
) -> Result<Arc<dyn UserConnectionResolver>, CommandError> {
    let base = resolver(ctx)?;
    Ok(input_message_id
        .and_then(|id| base.for_execution(id))
        .unwrap_or(base))
}

/// The session, after confirming it belongs to the caller's org.
pub(crate) async fn owned_session(ctx: &Ctx, session_id: &str) -> Result<SessionId, CommandError> {
    let session_id = parse_owned_session_id(session_id)?;
    verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;
    Ok(session_id)
}

/// A resolver failure, logged here and returned as fixed text.
pub(crate) fn lookup_failed(
    what: &'static str,
) -> impl FnOnce(everruns_contracts::error::AgentLoopError) -> CommandError {
    move |error| {
        tracing::error!(%error, "{what}");
        CommandError::internal(anyhow::anyhow!(what))
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetConnectionToken {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Connection provider, e.g. `github` or `daytona`.
    pub provider: String,
    /// The invocation whose end user the lookup acts for, when bound to one.
    #[serde(default)]
    pub input_message_id: Option<Uuid>,
}

#[command(
    name = "worker_get_connection_token",
    category = "connections",
    description = "Internal: decrypt (or mint, or refresh) a session's connection token for a provider; null when not connected.",
    method = "GET",
    path = "/internal/sessions/{session_id}/connections/{provider}/token",
    policy = SESSION_VIEW
)]
impl Command for WorkerGetConnectionToken {
    type Output = Option<String>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<String>, CommandError> {
        let lookup = self;
        // MCP grants are attachment-scoped: `worker_get_mcp_connection_token`.
        if lookup.provider.starts_with("mcp_oauth_") {
            return Err(CommandError::forbidden(
                "MCP credentials require attachment-scoped resolution",
            ));
        }
        let session_id = owned_session(ctx, &lookup.session_id).await?;
        bound_resolver(ctx, lookup.input_message_id)?
            .get_connection_token(session_id, &lookup.provider)
            .await
            .map_err(lookup_failed("Failed to resolve connection token"))
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetSandboxConnectionToken {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Sandbox provider.
    pub provider: String,
    /// The credential binding, authorized against the session's pinned sandbox.
    #[schema(value_type = Object)]
    pub credential: SessionSandboxCredential,
}

#[command(
    name = "worker_get_sandbox_connection_token",
    category = "connections",
    description = "Internal: resolve a sandbox credential authorized by the session's pinned sandbox; null otherwise.",
    method = "GET",
    path = "/internal/sessions/{session_id}/connections/{provider}/sandbox_token",
    policy = SESSION_VIEW
)]
impl Command for WorkerGetSandboxConnectionToken {
    type Output = Option<String>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<String>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        resolver(ctx)?
            .get_sandbox_connection_token(session_id, &self.provider, &self.credential)
            .await
            .map_err(lookup_failed("Failed to resolve sandbox connection token"))
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetServiceApiKeyConnection {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Connection provider, e.g. `github` or `daytona`.
    pub provider: String,
    /// The invocation whose end user the lookup acts for, when bound to one.
    #[serde(default)]
    pub input_message_id: Option<Uuid>,
}

/// The responding agent's own API-key connection.
#[derive(Serialize, ToSchema)]
pub struct WorkerServiceApiKeyConnection {
    pub api_key: String,
    pub metadata: Option<serde_json::Value>,
}

impl std::fmt::Debug for WorkerServiceApiKeyConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerServiceApiKeyConnection")
            .field("api_key", &"<redacted>")
            .field("metadata", &self.metadata)
            .finish()
    }
}

#[command(
    name = "worker_get_service_api_key_connection",
    category = "connections",
    description = "Internal: the responding agent's own service-account API-key connection; null when it has none.",
    method = "GET",
    path = "/internal/sessions/{session_id}/connections/{provider}/service_api_key",
    policy = SESSION_VIEW
)]
impl Command for WorkerGetServiceApiKeyConnection {
    type Output = Option<WorkerServiceApiKeyConnection>;

    async fn execute(
        self,
        ctx: &Ctx,
    ) -> Result<Option<WorkerServiceApiKeyConnection>, CommandError> {
        let lookup = self;
        let session_id = owned_session(ctx, &lookup.session_id).await?;
        Ok(bound_resolver(ctx, lookup.input_message_id)?
            .get_service_api_key_connection(session_id, &lookup.provider)
            .await
            .map_err(lookup_failed("Failed to resolve service connection"))?
            .map(|connection| WorkerServiceApiKeyConnection {
                api_key: connection.api_key,
                metadata: connection.metadata,
            }))
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetConnectionUser {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Connection provider, e.g. `github` or `daytona`.
    pub provider: String,
    /// The invocation whose end user the lookup acts for, when bound to one.
    #[serde(default)]
    pub input_message_id: Option<Uuid>,
}

#[command(
    name = "worker_get_connection_user",
    category = "connections",
    description = "Internal: the virtual user whose connection a session would use for a provider; null when none.",
    method = "GET",
    path = "/internal/sessions/{session_id}/connections/{provider}/user",
    policy = SESSION_VIEW
)]
impl Command for WorkerGetConnectionUser {
    type Output = Option<Uuid>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<Uuid>, CommandError> {
        let lookup = self;
        let session_id = owned_session(ctx, &lookup.session_id).await?;
        bound_resolver(ctx, lookup.input_message_id)?
            .get_connection_user(session_id, &lookup.provider)
            .await
            .map_err(lookup_failed("Failed to resolve connection owner"))
    }
}

/// The virtual user, after confirming it belongs to the caller's org.
async fn owned_virtual_user(ctx: &Ctx, id: Uuid) -> Result<(), CommandError> {
    ctx.db
        .get_virtual_user(ctx.org_id(), VirtualUserId::from_uuid(id))
        .await
        .map_err(CommandError::internal)?
        .ok_or_else(|| CommandError::not_found("Virtual user"))?;
    Ok(())
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetVirtualUserConnectionToken {
    /// The virtual user that owns the connection (a leased resource's owner).
    pub virtual_user_id: Uuid,
    /// Connection provider.
    pub provider: String,
}

#[command(
    name = "worker_get_virtual_user_connection_token",
    category = "connections",
    description = "Internal: cleanup lookup of a virtual user's connection token for a provider; null when not connected.",
    method = "GET",
    path = "/internal/virtual_users/{virtual_user_id}/connections/{provider}/token",
    policy = VIRTUAL_USER_VIEW
)]
impl Command for WorkerGetVirtualUserConnectionToken {
    type Output = Option<String>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<String>, CommandError> {
        owned_virtual_user(ctx, self.virtual_user_id).await?;
        resolver(ctx)?
            .get_connection_token_for_user(self.virtual_user_id, &self.provider)
            .await
            .map_err(lookup_failed("Failed to resolve connection token for user"))
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetConnectionTokenForConnection {
    /// The virtual user that owns the connection.
    pub virtual_user_id: Uuid,
    /// The exact connection a leased resource was created with.
    pub connection_id: Uuid,
    /// Connection provider.
    pub provider: String,
}

#[command(
    name = "worker_get_connection_token_for_connection",
    category = "connections",
    description = "Internal: cleanup lookup of one exact connection's token; null when it is gone.",
    method = "GET",
    path = "/internal/virtual_users/{virtual_user_id}/connections/{provider}/{connection_id}/token",
    policy = VIRTUAL_USER_VIEW
)]
impl Command for WorkerGetConnectionTokenForConnection {
    type Output = Option<String>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<String>, CommandError> {
        owned_virtual_user(ctx, self.virtual_user_id).await?;
        resolver(ctx)?
            .get_connection_token_for_connection(
                self.connection_id,
                self.virtual_user_id,
                &self.provider,
            )
            .await
            .map_err(lookup_failed("Failed to resolve connection token"))
    }
}

#[cfg(test)]
mod tests;
