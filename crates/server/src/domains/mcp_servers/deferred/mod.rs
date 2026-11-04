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
//
// Decision: a failed `tools/list` is not a removal. The session keeps a record
// of what each server listed last (`MCP_LAST_TOOLS_KV_PREFIX`), and a turn
// whose listing fails publishes that record instead of dropping the server.
// The tool list the model sees, and so the cached prompt, stays the same; a
// call while the server is down fails with the server's connection error.
// The record lives in session storage, not process memory, so every worker
// sees it. Design of record: knowledge/execution/cache-stable-tool-changes.md.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::Result;
use everruns_core::connection_services::UserConnectionResolver;
use everruns_core::session_services::SessionStorageStore;

use super::scoped_mcp::build_materialized_scoped_mcp_tool_definitions_reporting;
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
    let mut failed = Vec::new();
    let mut definitions = build_materialized_scoped_mcp_tool_definitions_reporting(
        db,
        org_id,
        &listed,
        Some(session_id),
        connection_resolver,
        egress_service,
        &mut failed,
    )
    .await?;
    if let Some(storage) = storage {
        keep_last_listed_tools(storage, session_id, &listed, &failed, &mut definitions).await;
    }
    let mut metadata_servers = servers.clone();
    for name in deferred {
        let effective_description = effective_server_description(db, org_id, &servers[&name]).await;
        let description = effective_description.clone().or_else(|| {
            url::Url::parse(&servers[&name].url)
                .ok()
                .and_then(|url| url.host_str().map(|host| format!("tools at {host}")))
        });
        if let Some(description) = effective_description {
            metadata_servers
                .get_mut(&name)
                .expect("deferred server exists")
                .description = Some(description);
        }
        definitions.push(everruns_core::deferred_mcp_server_definition(
            &name,
            description.as_deref(),
        ));
    }
    super::scoped_mcp::attach_server_search_metadata(&mut definitions, &metadata_servers);
    Ok(definitions)
}

/// Hash of the record this process last wrote per (session, server prefix),
/// so an unchanged listing costs no storage write. Bounded by clearing: a
/// forgotten entry only costs one rewrite of the same record.
static WRITTEN: LazyLock<Mutex<HashMap<(uuid::Uuid, String), u64>>> =
    LazyLock::new(Default::default);
const WRITTEN_MAX_ENTRIES: usize = 10_000;

/// Publish the last listed tools of each server in `failed`, and record the
/// tools every other listed server published now (none included, so a server
/// that stops offering tools, or loses its connection, is not revived later).
/// Best effort: a storage error only means the server is dropped as before.
async fn keep_last_listed_tools(
    storage: &dyn SessionStorageStore,
    session_id: SessionId,
    listed: &ScopedMcpServers,
    failed: &[String],
    definitions: &mut Vec<ToolDefinition>,
) {
    let mut by_prefix: HashMap<String, Vec<ToolDefinition>> = HashMap::new();
    for definition in definitions.iter() {
        if let Some((prefix, _)) = everruns_core::mcp_server::parse_mcp_tool_name(definition.name())
        {
            by_prefix
                .entry(prefix)
                .or_default()
                .push(definition.clone());
        }
    }
    for name in listed.keys() {
        let prefix = everruns_core::mcp_server::sanitize_mcp_server_name(name);
        let key = format!("{}{prefix}", everruns_core::MCP_LAST_TOOLS_KV_PREFIX);
        if failed.contains(name) {
            let kept = match storage.get_value(session_id, &key).await {
                Ok(Some(value)) => {
                    serde_json::from_str::<Vec<ToolDefinition>>(&value).unwrap_or_default()
                }
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(server_name = %name, %error, "Failed to read last listed MCP tools");
                    continue;
                }
            };
            tracing::info!(
                server_name = %name,
                tools = kept.len(),
                "MCP tool listing failed; keeping the tools it listed last"
            );
            definitions.extend(kept);
            continue;
        }
        let tools = by_prefix.remove(&prefix).unwrap_or_default();
        let Ok(value) = serde_json::to_string(&tools) else {
            continue;
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        let hash = hasher.finish();
        let memo_key = (session_id.uuid(), prefix);
        let unchanged = WRITTEN
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&memo_key)
            == Some(&hash);
        if unchanged {
            continue;
        }
        match storage.set_value(session_id, &key, &value).await {
            Ok(()) => {
                let mut written = WRITTEN.lock().unwrap_or_else(|error| error.into_inner());
                if written.len() >= WRITTEN_MAX_ENTRIES {
                    written.clear();
                }
                written.insert(memo_key, hash);
            }
            Err(error) => {
                tracing::warn!(server_name = %name, %error, "Failed to record listed MCP tools");
            }
        }
    }
}

async fn effective_server_description(
    db: &StorageBackend,
    org_id: i64,
    server: &everruns_core::ScopedMcpServer,
) -> Option<String> {
    if let Some(description) = server
        .description
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        return Some(description.to_string());
    }
    if let Some(preset) = &server.preset {
        let row = db
            .get_mcp_server_by_name(org_id, preset.catalog_name())
            .await
            .ok()
            .flatten();
        if let Some(description) = row
            .and_then(|row| row.description)
            .filter(|text| !text.trim().is_empty())
        {
            return Some(description);
        }
    }
    None
}

#[cfg(test)]
mod tests;
