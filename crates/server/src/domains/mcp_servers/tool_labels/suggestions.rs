// Rating suggestions for MCP tool risk labels.
//
// Decisions are in the parent module's header. In short: the deployment's
// decisions service rates each tool that has no saved label, and the answer
// only fills `suggested_label`; a person confirms it by setting the label.

use futures::stream::{self, StreamExt};
use serde_json::json;

use super::{MCP_SERVER_MANAGE, McpToolDefinition, McpToolLabel};
use super::{McpServerTool, editable_server_row, known_tools, list_tools};
use crate::domains::common::*;
use everruns_core::{DecisionQuestion, DecisionRequest, DecisionsService};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The question Tools in Shell ratings ask
/// (`crates/integrations/src/bashkit/tools_in_shell/ratings.rs`), so both
/// paths judge a tool the same way.
const QUESTION: &str = "Does calling this tool change, create, send or delete anything outside \
                        the caller's own session, such as records, messages, files or settings \
                        in another system? Answer from its name, description and input schema.";

/// A "yes" at or above this suggests `changes`; below it, `read_only`.
const CHANGES_THRESHOLD: f64 = 0.5;

/// Request metadata naming this caller, for attribution.
const PURPOSE: &str = "mcp_servers.tool_label_suggestion";

/// Ratings in flight at once for one server.
const CONCURRENT_RATINGS: usize = 4;

/// The label the decisions service suggests for `tool`, `None` when it could
/// not say.
pub(super) async fn rate(
    service: &dyn DecisionsService,
    tool: &McpToolDefinition,
) -> Option<McpToolLabel> {
    let request = DecisionRequest::new(json!({
        "tool": tool.name,
        "description": tool.description,
        "input_schema": tool.input_schema,
    }))
    .ask("changes", DecisionQuestion::noul(QUESTION))
    .with_metadata("purpose", PURPOSE);
    match service.evaluate(request).await {
        Ok(outcome) => {
            let yes = outcome.get("changes")?.probability_yes()?;
            Some(if yes >= CHANGES_THRESHOLD {
                McpToolLabel::Changes
            } else {
                McpToolLabel::ReadOnly
            })
        }
        Err(error) => {
            tracing::debug!(%error, tool = %tool.name, "MCP tool label suggestion unavailable");
            None
        }
    }
}

// ============================================================================
// SuggestMcpToolLabels
// ============================================================================

/// Suggest a risk label for every tool of an MCP server that has none.
#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct SuggestMcpToolLabels {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "suggest_mcp_tool_labels",
    category = "mcp_servers",
    description = "Ask the deployment's decision service to suggest read_only or changes for every tool of an MCP server that has no label yet. Suggestions are never applied: a person confirms one by setting the label.",
    method = "POST",
    path = "/v1/mcp-servers/{id}/tools/suggest-labels",
    policy = MCP_SERVER_MANAGE,
    positional = "id",
    http = plain,
    responses(
        (status = 404, description = "MCP server not found"),
        (status = 503, description = "No decision service is configured on this deployment")
    ),
    cli = CliRoute::new(&["mcp-servers"], "suggest-labels").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Get suggested risk labels for the unlabeled tools of an MCP server", "everruns mcp-servers suggest-labels mcp_01h9",)]),
)]
impl Command for SuggestMcpToolLabels {
    type Output = Vec<McpServerTool>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<McpServerTool>, CommandError> {
        let service = ctx
            .decisions
            .clone()
            .filter(|service| service.is_configured())
            .ok_or_else(|| {
                CommandError::unavailable(
                    "Suggesting labels needs a decision service, which is not configured on \
                     this deployment",
                )
                .with_code("decisions_not_configured")
            })?;
        let row = editable_server_row(ctx, &self.id).await?;
        let server_id = row.id.uuid();
        let labeled: std::collections::HashSet<String> = ctx
            .db
            .list_mcp_tool_labels(ctx.org_id(), &[server_id])
            .await?
            .into_iter()
            .filter(|label| label.label.is_some())
            .map(|label| label.tool_name)
            .collect();
        let unlabeled: Vec<McpToolDefinition> = known_tools(ctx, &row)
            .await?
            .into_iter()
            .filter(|tool| !labeled.contains(&tool.name))
            .collect();
        let ratings: Vec<(String, Option<McpToolLabel>)> = stream::iter(unlabeled)
            .map(|tool| {
                let service = service.clone();
                async move { (tool.name.clone(), rate(service.as_ref(), &tool).await) }
            })
            .buffer_unordered(CONCURRENT_RATINGS)
            .collect()
            .await;
        for (tool_name, suggestion) in ratings {
            // A failed rating keeps whatever suggestion the tool had.
            let Some(suggestion) = suggestion else {
                continue;
            };
            ctx.db
                .set_mcp_tool_suggestion(ctx.org_id(), server_id, &tool_name, suggestion.as_str())
                .await?;
        }
        list_tools(ctx, &row).await
    }
}
