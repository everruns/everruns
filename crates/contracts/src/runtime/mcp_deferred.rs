// Deferred MCP servers: a server whose tools are not listed at turn start.
//
// Spec: knowledge/integrations/user-mcp-servers.md (D6).
//
// Decision: a deferred server costs no `tools/list` round trip until the model
// asks for it. In its place the turn carries one placeholder tool per server,
// `mcp_<prefix>` (name and description), which tool search finds like any
// other tool. Revealing a server writes a session record; the host lists the
// server's tools from the next step on (through the same identity-scoped tool
// cache as any other server), and keeps listing them for the rest of the
// session.
//
// Decision: the placeholder name cannot collide with a real MCP tool: those
// are `mcp_<prefix>__<tool>`, and a valid prefix never contains `__`.
//
// Decision: the placeholder is itself callable and reveals its server. Generic
// `tool_search` is the primary path, but on models with provider-hosted tool
// search the model never sees that tool; there a call to the placeholder is
// the only way in. Both paths write the same record through
// [`reveal_deferred_mcp_server`], so there is one reveal mechanism.

use crate::runtime::error::Result;
use crate::runtime::mcp_server::{ScopedMcpServers, sanitize_mcp_server_name};
use crate::runtime::session_services::SessionStorageStore;
use crate::runtime::tool_context::ToolContext;
use crate::runtime::tool_narration::{ToolNarrationContext, ToolNarrationPhase, labeled_phrase};
use crate::runtime::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolHints, ToolPolicy,
};
use crate::runtime::tools::{Tool, ToolExecutionResult};
use crate::runtime::typed_id::SessionId;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashSet;

/// Session key-value prefix recording that a deferred MCP server was revealed.
/// The suffix is the server's tool prefix. Reserved from the user-facing
/// `kv_store` tool.
pub const DEFERRED_MCP_REVEAL_KV_PREFIX: &str = "mcp_reveal:";

/// Category the placeholder tools are grouped under.
const MCP_SERVERS_CATEGORY: &str = "MCP Servers";

/// Name of the placeholder tool standing in for a deferred server.
pub fn deferred_mcp_server_tool_name(server_name: &str) -> String {
    format!("mcp_{}", sanitize_mcp_server_name(server_name))
}

/// The server prefix a placeholder tool stands for, or `None` when `tool_name`
/// is not a placeholder (a real MCP tool always carries `__`).
pub fn deferred_mcp_server_prefix(tool_name: &str) -> Option<&str> {
    let prefix = tool_name.strip_prefix("mcp_")?;
    (!prefix.is_empty() && !prefix.contains("__")).then_some(prefix)
}

/// Placeholder tool definition for a deferred server: its name, a description
/// telling the model how to load its tools, and no parameters. It is never
/// deferred itself, so the model always sees the line.
pub fn deferred_mcp_server_definition(
    server_name: &str,
    description: Option<&str>,
) -> ToolDefinition {
    let about = description
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| format!(": {text}"))
        .unwrap_or_default();
    ToolDefinition::Builtin(BuiltinTool {
        name: deferred_mcp_server_tool_name(server_name),
        display_name: Some(server_name.to_string()),
        description: format!(
            "MCP server `{server_name}`{about}. Its tools are not loaded yet. To use them, \
             search for this server with `tool_search`, or call this tool; they become \
             callable on your next step."
        ),
        parameters: json!({"type": "object", "properties": {}, "additionalProperties": false}),
        policy: ToolPolicy::Auto,
        category: Some(MCP_SERVERS_CATEGORY.to_string()),
        deferrable: DeferrablePolicy::Never,
        hints: ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true),
        full_parameters: None,
    })
}

/// Restore the placeholder policy on a definition that crossed a boundary
/// which keeps only name, description and parameters (the worker protocol).
pub fn normalize_deferred_mcp_server_definition(mut definition: ToolDefinition) -> ToolDefinition {
    if let ToolDefinition::Builtin(builtin) = &mut definition
        && deferred_mcp_server_prefix(&builtin.name).is_some()
    {
        builtin.deferrable = DeferrablePolicy::Never;
        builtin.category = Some(MCP_SERVERS_CATEGORY.to_string());
    }
    definition
}

/// Record that the server with tool prefix `prefix` was revealed in `session`.
pub async fn reveal_deferred_mcp_server(
    storage: &dyn SessionStorageStore,
    session: SessionId,
    prefix: &str,
) -> Result<()> {
    storage
        .set_value(
            session,
            &format!("{DEFERRED_MCP_REVEAL_KV_PREFIX}{prefix}"),
            "1",
        )
        .await
}

/// Tool prefixes of the deferred servers revealed in `session`. Best effort:
/// a storage failure reads as "nothing revealed", which only keeps servers
/// deferred.
pub async fn revealed_mcp_servers(
    storage: &dyn SessionStorageStore,
    session: SessionId,
) -> HashSet<String> {
    match storage.list_keys(session).await {
        Ok(keys) => keys
            .into_iter()
            .filter_map(|info| {
                info.key
                    .strip_prefix(DEFERRED_MCP_REVEAL_KV_PREFIX)
                    .map(str::to_string)
            })
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "Failed to read revealed MCP servers; keeping them deferred");
            HashSet::new()
        }
    }
}

/// Split a turn's servers into the ones to list now and the names of the
/// deferred ones still waiting for a reveal.
pub fn partition_deferred_mcp_servers(
    servers: &ScopedMcpServers,
    revealed: &HashSet<String>,
) -> (ScopedMcpServers, Vec<String>) {
    let mut listed = ScopedMcpServers::new();
    let mut deferred = Vec::new();
    for (name, server) in servers {
        if server.deferred
            && server.tool_discovery
            && !revealed.contains(&sanitize_mcp_server_name(name))
        {
            deferred.push(name.clone());
        } else {
            listed.insert(name.clone(), server.clone());
        }
    }
    (listed, deferred)
}

/// Registry tool behind a placeholder definition: calling it reveals the
/// server, exactly as a `tool_search` match does.
pub struct DeferredMcpServerTool {
    definition: BuiltinTool,
}

impl DeferredMcpServerTool {
    /// Wrap a placeholder definition (see [`deferred_mcp_server_definition`]).
    pub fn new(definition: BuiltinTool) -> Self {
        Self { definition }
    }

    fn prefix(&self) -> &str {
        deferred_mcp_server_prefix(&self.definition.name).unwrap_or_default()
    }
}

#[async_trait]
impl Tool for DeferredMcpServerTool {
    fn name(&self) -> &str {
        &self.definition.name
    }

    fn display_name(&self) -> Option<&str> {
        self.definition.display_name.as_deref()
    }

    fn description(&self) -> &str {
        &self.definition.description
    }

    fn parameters_schema(&self) -> Value {
        self.definition.parameters.clone()
    }

    fn hints(&self) -> ToolHints {
        self.definition.hints.clone()
    }

    fn deferrable_policy(&self) -> DeferrablePolicy {
        DeferrablePolicy::Never
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::Builtin(self.definition.clone())
    }

    fn narrate(
        &self,
        _tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        let server = self.definition.display_name.clone();
        let phrase = if locale.is_some_and(|l| l.starts_with("uk")) {
            labeled_phrase(
                "Завантажую інструменти MCP-сервера",
                "Завантажив інструменти MCP-сервера",
                "Не вдалося завантажити інструменти MCP-сервера",
                server,
                phase,
            )
        } else {
            labeled_phrase(
                "Loading MCP server tools",
                "Loaded MCP server tools",
                "Could not load MCP server tools",
                server,
                phase,
            )
        };
        Some(phrase)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "Loading an MCP server's tools requires session storage and cannot run standalone.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(storage) = &context.storage_store else {
            return ToolExecutionResult::tool_error(
                "Session storage is not available, so this MCP server's tools cannot be loaded.",
            );
        };
        match reveal_deferred_mcp_server(storage.as_ref(), context.session_id, self.prefix()).await
        {
            Ok(()) => ToolExecutionResult::success(json!({
                "server": self.definition.display_name,
                "message": "The server's tools are loading; they are callable on your next step.",
            })),
            Err(error) => ToolExecutionResult::tool_error(format!(
                "Could not load the MCP server's tools: {error}"
            )),
        }
    }
}

#[cfg(test)]
#[path = "mcp_deferred_tests.rs"]
mod tests;
