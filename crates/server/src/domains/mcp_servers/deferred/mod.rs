// A turn's MCP tool definitions with deferred servers held back.
//
// Spec: knowledge/integrations/user-mcp-servers.md (D6).
//
// Decision: a deferred server that the session has not revealed yet costs no
// `tools/list` at turn start; the turn carries its placeholder (name and a
// one-line description) instead. Once tool search or the placeholder reveals
// it, the server goes through the ordinary discovery below, with the same
// identity-scoped tool cache as every other server. Turn-context loaders
// (gRPC and in-process) call this; previews keep listing everything.

use std::sync::Arc;

use anyhow::Result;
use everruns_core::connection_services::UserConnectionResolver;
use everruns_core::session_services::SessionStorageStore;

use super::scoped_mcp::build_materialized_scoped_mcp_tool_definitions;
use crate::kernel_imports::{
    EgressService, ScopedMcpServers, contracts::tool_types::ToolDefinition,
    contracts::typed_id::SessionId,
};
use crate::storage::StorageBackend;

/// The turn's MCP tool definitions: listed tools for eager and revealed
/// servers, one placeholder per deferred server not yet revealed. Without
/// session storage nothing can be revealed, so deferred servers stay
/// placeholders.
pub async fn build_turn_mcp_tool_definitions(
    db: &StorageBackend,
    org_id: i64,
    servers: &ScopedMcpServers,
    session_id: SessionId,
    connection_resolver: Option<&Arc<dyn UserConnectionResolver>>,
    egress_service: &dyn EgressService,
    storage: Option<&dyn SessionStorageStore>,
) -> Result<Vec<ToolDefinition>> {
    let revealed = match storage {
        Some(storage) if servers.values().any(|server| server.deferred) => {
            everruns_core::revealed_mcp_servers(storage, session_id).await
        }
        _ => Default::default(),
    };
    let (listed, deferred) = everruns_core::partition_deferred_mcp_servers(servers, &revealed);
    let mut definitions = build_materialized_scoped_mcp_tool_definitions(
        db,
        org_id,
        &listed,
        Some(session_id),
        connection_resolver,
        egress_service,
    )
    .await?;
    for name in deferred {
        let description = placeholder_description(db, org_id, &servers[&name]).await;
        definitions.push(everruns_core::deferred_mcp_server_definition(
            &name,
            description.as_deref(),
        ));
    }
    Ok(definitions)
}

/// What the model reads about a deferred server: the catalog preset's
/// description, else the host it lives on.
async fn placeholder_description(
    db: &StorageBackend,
    org_id: i64,
    server: &everruns_core::ScopedMcpServer,
) -> Option<String> {
    if let Some(preset) = &server.preset {
        let row = db
            .get_mcp_server_by_name(org_id, preset.catalog_name())
            .await
            .ok()
            .flatten()?;
        return row.description.filter(|text| !text.trim().is_empty());
    }
    url::Url::parse(&server.url)
        .ok()
        .and_then(|url| url.host_str().map(|host| format!("tools at {host}")))
}

#[cfg(test)]
mod tests;
