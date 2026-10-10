// Saved per-tool risk labels on organization MCP servers.
//
// Spec: knowledge/integrations/mcp-servers.md ("Tool risk labels").
//
// Decisions:
// - A person's label always wins over the tool's own annotations (user
//   decision 2026-10-10). `read_only` also clears `open_world`, otherwise tool
//   approval would still ask for every MCP tool in the normal mode.
// - Labels are keyed by server and tool name, so they survive tool refreshes
//   and a tool that disappears and comes back keeps its label.
// - Setting a label is editing the server, so it takes the same policy as
//   updating it (MCP_SERVER_MANAGE). Reading takes MCP_SERVER_VIEW.
// - Suggestions (user decision 2026-10-10, "rating suggests"): on request the
//   deployment's decisions service rates every tool without a saved label,
//   asking the question Tools in Shell ratings ask. The answer only fills
//   `suggested_label`; it is shown but never applied, so prompts change only
//   once a person confirms it by setting the label. A failed rating leaves the
//   old suggestion as it was. See `suggestions.rs`.
// - Setting a label drops the tool's suggestion, since the person settled it;
//   clearing a label keeps the suggestion. Keeps one rule with no stale hints.
// - Every runtime path loads labels for all of its servers in one query, never
//   one per tool.

use std::collections::{HashMap, HashSet};

use super::McpServerService;
use super::queries as q;
use super::record::McpServerStatus;
use super::{MCP_SERVER_MANAGE, MCP_SERVER_VIEW};
use crate::domains::common::*;
use crate::storage::StorageBackend;
use everruns_contracts::tool_types::ToolDefinition;
use everruns_contracts::typed_id::McpServerId;
use everruns_core::capabilities::Capability as _;
use everruns_core::mcp::{McpCapability, McpToolLabels, parse_mcp_capability_id};
use everruns_core::{Caller, McpToolAnnotations, McpToolDefinition, McpToolLabel};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// One tool an MCP server offers, with its saved risk label.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct McpServerTool {
    /// The tool's own name on the MCP server.
    #[schema(example = "search_docs")]
    pub name: String,
    /// Human-readable title the server gives the tool.
    pub title: Option<String>,
    /// What the tool does, as the server describes it.
    pub description: Option<String>,
    /// Hints the server declares about the tool (read-only, destructive, ...).
    pub annotations: Option<McpToolAnnotations>,
    /// Risk label a person set. It wins over the annotations; null means none.
    pub label: Option<McpToolLabel>,
    /// Suggested label waiting for a person to confirm. Never applied by itself.
    pub suggested_label: Option<McpToolLabel>,
}

/// Labels for the given servers, keyed by server id, in one query.
pub async fn load_tool_labels(
    db: &StorageBackend,
    org_id: i64,
    server_ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, McpToolLabels>> {
    let mut labels: HashMap<Uuid, McpToolLabels> = HashMap::new();
    for row in db.list_mcp_tool_labels(org_id, server_ids).await? {
        if let Some(label) = row.label.as_deref().and_then(McpToolLabel::parse) {
            labels
                .entry(row.mcp_server_id)
                .or_default()
                .insert(row.tool_name, label);
        }
    }
    Ok(labels)
}

/// Labels for the named organization servers (catalog presets), keyed by
/// server name, in one query.
pub async fn load_tool_labels_by_server_names(
    db: &StorageBackend,
    org_id: i64,
    server_names: &[String],
) -> anyhow::Result<HashMap<String, McpToolLabels>> {
    let mut labels: HashMap<String, McpToolLabels> = HashMap::new();
    for (server, tool, label) in db
        .list_mcp_tool_labels_by_server_names(org_id, server_names)
        .await?
    {
        if let Some(label) = McpToolLabel::parse(&label) {
            labels.entry(server).or_default().insert(tool, label);
        }
    }
    Ok(labels)
}

/// Tool definitions for an agent's organization MCP capabilities
/// (`mcp:{server_id}`), with saved labels applied.
///
/// The single mapping both worker paths use, so a session sees the same hints
/// the capability catalog shows. Servers are fetched in one batch and labels in
/// one query. Failures degrade to no tools, as a missing server always has.
pub async fn org_mcp_tool_definitions<'a>(
    service: &McpServerService,
    db: &StorageBackend,
    org_id: i64,
    capability_ids: impl IntoIterator<Item = &'a str>,
) -> Vec<ToolDefinition> {
    let mut seen = HashSet::new();
    let server_ids: Vec<Uuid> = capability_ids
        .into_iter()
        .filter_map(parse_mcp_capability_id)
        .filter(|id| seen.insert(*id))
        .collect();
    if server_ids.is_empty() {
        return vec![];
    }
    let caller = Caller::internal(org_id);
    let (servers, labels) = match tokio::try_join!(
        service.get_batch_with_tools(&caller, &server_ids),
        load_tool_labels(db, org_id, &server_ids),
    ) {
        Ok(loaded) => loaded,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to load organization MCP tools");
            return vec![];
        }
    };
    let mut labels = labels;
    let mut definitions = Vec::new();
    for server_id in server_ids {
        // A deleted server contributes nothing, matching a single-server read.
        let Some((server, tools)) = servers
            .get(&server_id)
            .filter(|(server, _)| !matches!(server.status, McpServerStatus::Deleted))
        else {
            tracing::warn!(server_id = %server_id, "MCP server not found, skipping");
            continue;
        };
        if !everruns_core::mcp_server::is_valid_mcp_server_name(&server.name) {
            tracing::warn!(server_id = %server_id, "MCP tools omitted: ambiguous server prefix");
            continue;
        }
        definitions.extend(
            McpCapability::new(
                server_id,
                server.name.clone(),
                server.description.clone(),
                tools.clone(),
            )
            .with_tool_labels(labels.remove(&server_id).unwrap_or_default())
            .tool_definitions(),
        );
    }
    definitions
}

/// The tools a server is known to offer: its own cache, plus the tools found
/// through per-agent or per-person discovery (OAuth servers keep only those).
async fn known_tools(
    ctx: &Ctx,
    row: &crate::storage::McpServerRow,
) -> Result<Vec<McpToolDefinition>, CommandError> {
    let mut tools = McpServerService::cached_tools(row);
    let mut names: HashSet<String> = tools.iter().map(|tool| tool.name.clone()).collect();
    for cached in ctx
        .db
        .list_mcp_discovered_tool_sets(ctx.org_id(), row.id.uuid())
        .await?
    {
        let discovered: Vec<McpToolDefinition> = serde_json::from_value(cached).unwrap_or_default();
        for tool in discovered {
            if names.insert(tool.name.clone()) {
                tools.push(tool);
            }
        }
    }
    Ok(tools)
}

async fn editable_server_row(
    ctx: &Ctx,
    id: &str,
) -> Result<crate::storage::McpServerRow, CommandError> {
    let row = server_row(ctx, id).await?;
    if !matches!(row.status.as_str(), "active" | "disabled") {
        return Err(CommandError::bad_request(
            "Archived MCP servers cannot be edited",
        ));
    }
    Ok(row)
}

async fn server_row(ctx: &Ctx, id: &str) -> Result<crate::storage::McpServerRow, CommandError> {
    let server_id: McpServerId = id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid MCP server ID: {e}")))?;
    q::get_row(&ctx.db, ctx.org_id(), server_id.uuid())
        .await?
        .filter(|row| row.status != "deleted")
        .ok_or_else(|| CommandError::not_found("MCP server"))
}

fn to_tool(
    tool: McpToolDefinition,
    row: Option<&crate::storage::McpToolLabelRow>,
) -> McpServerTool {
    McpServerTool {
        name: tool.name,
        title: tool.title,
        description: tool.description,
        annotations: tool.annotations,
        label: row
            .and_then(|row| row.label.as_deref())
            .and_then(McpToolLabel::parse),
        suggested_label: row
            .and_then(|row| row.suggested_label.as_deref())
            .and_then(McpToolLabel::parse),
    }
}

// ============================================================================
// ListMcpServerTools
// ============================================================================

/// List the tools an MCP server offers, with their saved risk labels.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ListMcpServerTools {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "list_mcp_server_tools",
    category = "mcp_servers",
    description = "List the tools an MCP server offers, with each tool's annotations and saved risk label.",
    method = "GET",
    path = "/v1/mcp-servers/{id}/tools",
    policy = MCP_SERVER_VIEW,
    positional = "id",
    http = plain,
    responses((status = 404, description = "MCP server not found")),
    cli = CliRoute::new(&["mcp-servers"], "tools").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("See which tools of an MCP server ask for approval", "everruns mcp-servers tools mcp_01h9",)]),
)]
impl Command for ListMcpServerTools {
    type Output = Vec<McpServerTool>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<McpServerTool>, CommandError> {
        let row = server_row(ctx, &self.id).await?;
        list_tools(ctx, &row).await
    }
}

/// A server's known tools with their labels and suggestions, sorted by name.
async fn list_tools(
    ctx: &Ctx,
    row: &crate::storage::McpServerRow,
) -> Result<Vec<McpServerTool>, CommandError> {
    let labels = ctx
        .db
        .list_mcp_tool_labels(ctx.org_id(), &[row.id.uuid()])
        .await?;
    let labels: HashMap<&str, _> = labels
        .iter()
        .map(|label| (label.tool_name.as_str(), label))
        .collect();
    let mut tools: Vec<McpServerTool> = known_tools(ctx, row)
        .await?
        .into_iter()
        .map(|tool| {
            let label = labels.get(tool.name.as_str()).copied();
            to_tool(tool, label)
        })
        .collect();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tools)
}

// ============================================================================
// SetMcpToolLabel
// ============================================================================

/// Set or clear a person's risk label for one tool of an MCP server.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct SetMcpToolLabel {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    /// The tool's own name on the MCP server.
    pub tool_name: String,
    /// `read_only` never asks for approval in the normal mode, `changes` always
    /// asks. Null clears the label, so the tool's own annotations decide again.
    #[serde(default)]
    pub label: Option<McpToolLabel>,
}

#[command(
    name = "set_mcp_tool_label",
    category = "mcp_servers",
    description = "Set or clear a person's risk label for one MCP server tool. read_only never asks for approval in normal mode, changes always asks, null clears.",
    method = "PUT",
    path = "/v1/mcp-servers/{id}/tools/{tool_name}/label",
    policy = MCP_SERVER_MANAGE,
    positional = "id",
    http = plain,
    responses((status = 404, description = "MCP server or tool not found")),
    cli = CliRoute::new(&["mcp-servers"], "label-tool").with_args(&[CliArg::new("id").at(1), CliArg::new("tool_name").at(2)]).with_examples(&[CliExample::new("Stop a read-only MCP tool from asking for approval", "everruns mcp-servers label-tool mcp_01h9 search_docs --label read_only --reason 'Only reads documentation'",)]),
)]
impl Command for SetMcpToolLabel {
    type Output = McpServerTool;

    async fn execute(self, ctx: &Ctx) -> Result<McpServerTool, CommandError> {
        let row = editable_server_row(ctx, &self.id).await?;
        let tool = known_tools(ctx, &row)
            .await?
            .into_iter()
            .find(|tool| tool.name == self.tool_name)
            .ok_or_else(|| CommandError::not_found("Tool on this MCP server"))?;
        let saved = ctx
            .db
            .set_mcp_tool_label(
                ctx.org_id(),
                row.id.uuid(),
                &tool.name,
                self.label.map(McpToolLabel::as_str),
                ctx.caller.user_id,
            )
            .await?;
        Ok(to_tool(tool, Some(&saved)))
    }
}

mod suggestions;
pub use suggestions::SuggestMcpToolLabels;

#[cfg(test)]
mod tests;
