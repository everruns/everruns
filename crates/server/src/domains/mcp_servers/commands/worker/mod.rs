//! The worker's MCP grant lookups, as internal commands.
//!
//! Decision: an MCP server's OAuth grant is resolved only for the attachment
//! the worker names. Before reading any grant the command re-resolves the
//! server the session's scope gives that prefix (`worker_lookup`, the same
//! resolution `GetMcpServerByPrefix` serves) and requires its OAuth provider and
//! `actsAs` to match the request, so a worker cannot pick another credential
//! owner by supplying a different `acts_as` (THREAT[TM-TOOL-041]). These used
//! to be the `GetMcpConnectionToken` / `InvalidateMcpConnection` RPCs, which
//! found the session's org from the session; as internal commands they run as
//! the worker's org and a foreign session is `NotFound`.
//!
//! THREAT[TM-AUTHZ-023]: the token lookup is a read, so entity history records
//! nothing; invalidation is declared `Exempt`. Neither output nor params are
//! logged by `Command::run`.

use crate::domains::common::*;
use crate::domains::mcp_servers::McpServerService;
use crate::domains::mcp_servers::worker_lookup::{self, WorkerMcpLookup, WorkerMcpLookupError};
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW, SessionService};
use crate::domains::user_connections::commands::{bound_resolver, lookup_failed, owned_session};
use crate::kernel_imports::contracts::typed_id::SessionId;
use everruns_core::McpServerActsAs;
use everruns_core::session_services::SessionStorageStore;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// One MCP grant request: the attachment and the identity it acts as.
struct McpOperation<'a> {
    session_id: &'a str,
    server_prefix: Option<&'a str>,
    input_message_id: Option<Uuid>,
    provider: &'a str,
    acts_as: &'a str,
}

fn parse_acts_as(value: &str) -> Result<McpServerActsAs, CommandError> {
    match value {
        "none" | "service" | "user" | "user_or_service" => Ok(McpServerActsAs::from(value)),
        _ => Err(CommandError::bad_request("Invalid MCP acts_as value")),
    }
}

/// THREAT[TM-TOOL-041]: the request must name the configured attachment, as
/// that attachment's own OAuth provider and acting identity.
async fn validate(
    ctx: &Ctx,
    operation: &McpOperation<'_>,
) -> Result<(SessionId, McpServerActsAs), CommandError> {
    let acts_as = parse_acts_as(operation.acts_as)?;
    let input = operation
        .input_message_id
        .ok_or_else(|| CommandError::forbidden("MCP operation requires an invocation"))?;
    let prefix = operation
        .server_prefix
        .ok_or_else(|| CommandError::forbidden("MCP attachment scope required"))?;
    let session_id = owned_session(ctx, operation.session_id).await?;

    let session_service = ctx
        .session_service
        .clone()
        .unwrap_or_else(|| Arc::new(SessionService::new(ctx.db.clone())));
    let mcp_server_service = McpServerService::with_egress_service(
        ctx.db.clone(),
        ctx.encryption.clone(),
        ctx.capability_service.egress_service(),
    );
    let database = ctx.db.database().clone();
    let storage: Box<dyn SessionStorageStore> = match &ctx.encryption {
        Some(encryption) => Box::new(crate::storage::create_db_session_storage_store(
            database,
            encryption.as_ref().clone(),
        )),
        None => {
            Box::new(crate::storage::create_db_session_storage_store_without_encryption(database))
        }
    };
    let lookup = WorkerMcpLookup {
        db: &ctx.db,
        session_service: &session_service,
        mcp_server_service: &mcp_server_service,
        registry: ctx.capability_service.registry(),
        encryption: ctx.encryption.as_deref(),
        storage: Some(storage.as_ref()),
    };
    let server = worker_lookup::resolve(
        &lookup,
        ctx.org_id(),
        Some(session_id.uuid()),
        Some(input),
        prefix,
    )
    .await
    .map_err(|error| match error {
        WorkerMcpLookupError::UnknownInvocation => CommandError::forbidden("Unknown invocation"),
        WorkerMcpLookupError::Internal(message) => CommandError::internal(anyhow::anyhow!(message)),
    })?
    .ok_or_else(|| CommandError::forbidden("MCP attachment unavailable"))?
    .server;

    // A `user_or_service` attachment resolves as the person, then as the
    // agent, so the worker asks for each concrete identity in turn.
    let acts_as_matches = server.acts_as == acts_as
        || (server.acts_as == McpServerActsAs::UserOrService
            && matches!(acts_as, McpServerActsAs::User | McpServerActsAs::Service));
    if server.oauth_provider_id.as_deref() != Some(operation.provider) || !acts_as_matches {
        return Err(CommandError::forbidden(
            "MCP operation does not match the configured attachment",
        ));
    }
    Ok((session_id, acts_as))
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerGetMcpConnectionToken {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// The MCP attachment's tool prefix.
    #[serde(default)]
    pub server_prefix: Option<String>,
    /// The invocation the call belongs to.
    #[serde(default)]
    pub input_message_id: Option<Uuid>,
    /// The attachment's OAuth provider (`mcp_oauth_{server}`).
    pub provider: String,
    /// The concrete identity to read: `none`, `service`, `user` or `user_or_service`.
    pub acts_as: String,
}

#[command(
    name = "worker_get_mcp_connection_token",
    category = "mcp_servers",
    description = "Internal: decrypt (or refresh) the MCP grant a configured attachment acts as; null when there is none.",
    method = "GET",
    path = "/internal/sessions/{session_id}/mcp_servers/{server_prefix}/token",
    policy = SESSION_VIEW
)]
impl Command for WorkerGetMcpConnectionToken {
    type Output = Option<String>;

    async fn execute(self, ctx: &Ctx) -> Result<Option<String>, CommandError> {
        let (session_id, acts_as) = validate(
            ctx,
            &McpOperation {
                session_id: &self.session_id,
                server_prefix: self.server_prefix.as_deref(),
                input_message_id: self.input_message_id,
                provider: &self.provider,
                acts_as: &self.acts_as,
            },
        )
        .await?;
        bound_resolver(ctx, self.input_message_id)?
            .get_mcp_connection_token(session_id, &self.provider, acts_as)
            .await
            .map_err(lookup_failed("Failed to resolve MCP connection token"))
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerInvalidateMcpConnection {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// The MCP attachment's tool prefix.
    #[serde(default)]
    pub server_prefix: Option<String>,
    /// The invocation the rejected call belonged to.
    #[serde(default)]
    pub input_message_id: Option<Uuid>,
    /// The attachment's OAuth provider (`mcp_oauth_{server}`).
    pub provider: String,
    /// The identity whose grant served the rejected call.
    pub acts_as: String,
    /// Fingerprint of the credential the remote server rejected; a grant that
    /// has since been replaced is kept.
    pub rejected_credential_fingerprint: String,
}

#[command(
    name = "worker_invalidate_mcp_connection",
    category = "mcp_servers",
    description = "Internal: drop the MCP grant a remote server rejected, unless it was already replaced.",
    method = "POST",
    path = "/internal/sessions/{session_id}/mcp_servers/{server_prefix}/invalidate",
    policy = SESSION_MANAGE
)]
impl Command for WorkerInvalidateMcpConnection {
    type Output = ();

    async fn execute(self, ctx: &Ctx) -> Result<(), CommandError> {
        let (session_id, acts_as) = validate(
            ctx,
            &McpOperation {
                session_id: &self.session_id,
                server_prefix: self.server_prefix.as_deref(),
                input_message_id: self.input_message_id,
                provider: &self.provider,
                acts_as: &self.acts_as,
            },
        )
        .await?;
        bound_resolver(ctx, self.input_message_id)?
            .invalidate_mcp_connection(
                session_id,
                &self.provider,
                acts_as,
                &self.rejected_credential_fingerprint,
            )
            .await
            .map_err(lookup_failed("Failed to invalidate MCP connection"))
    }
}

#[cfg(test)]
mod tests;
