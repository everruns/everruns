// Rating suggestions for MCP tool risk labels.
//
// Decisions are in the parent module's header. In short: a decision model
// rates each tool that has no saved label, and the answer only fills
// `suggested_label`; a person confirms it by setting the label.
//
// Who rates: an org that answers deployment-owned decision checks itself
// (`system_decisions: organization`) has its own decision default rate, the
// way the Slack relevance check does. Any failure to resolve it is an error,
// never a fall back to the deployment (THREAT[TM-LLM-037]). No session exists
// here, so personal providers fail closed and usage goes to the structured
// log (THREAT[TM-LLM-049]). Every other org uses the deployment's service.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::DecisionOutcome;
use everruns_core::connection_services::{DecisionModelBinding, SystemDecisionModel};
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
    /// Prefixed public identifier. See [ID Schema](https://everruns.com/docs/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "suggest_mcp_tool_labels",
    category = "mcp_servers",
    description = "Ask the organization's decision model (or the deployment's decision service when the organization does not select its own) to suggest read_only or changes for every tool of an MCP server that has no label yet. Suggestions are never applied: a person confirms one by setting the label.",
    method = "POST",
    path = "/v1/mcp-servers/{id}/tools/suggest-labels",
    policy = MCP_SERVER_MANAGE,
    positional = "id",
    http = plain,
    responses(
        (status = 404, description = "MCP server not found"),
        (status = 503, description = "No decision model is available to rate the tools")
    ),
    cli = CliRoute::new(&["mcp-servers"], "suggest-labels").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Get suggested risk labels for the unlabeled tools of an MCP server", "everruns mcp-servers suggest-labels mcp_01h9",)]),
)]
impl Command for SuggestMcpToolLabels {
    type Output = Vec<McpServerTool>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<McpServerTool>, CommandError> {
        let service = decision_service(ctx).await?;
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

/// The service that rates for this org: its own decision default when it
/// selects one, otherwise the deployment's.
async fn decision_service(ctx: &Ctx) -> Result<Arc<dyn DecisionsService>, CommandError> {
    let org_selected = ctx
        .db
        .get_organization_settings(ctx.org_id())
        .await?
        .is_some_and(|settings| {
            crate::storage::SystemDecisions::from_db(&settings.system_decisions)
                == crate::storage::SystemDecisions::Organization
        });
    if org_selected {
        let unavailable = |reason: String| {
            CommandError::unavailable(format!(
                "Suggesting labels uses this organization's decision model, which is not \
                 usable: {reason}"
            ))
            .with_code("org_decision_model_unavailable")
        };
        let resolver = ctx
            .provider_service
            .as_ref()
            .and_then(|service| service.resolver())
            .ok_or_else(|| unavailable("models are not wired on this surface".into()))?;
        let binding = match resolver
            .resolve_system_decision_model(ctx.org_id(), None)
            .await
            .map_err(|error| unavailable(error.to_string()))?
        {
            SystemDecisionModel::Organization(binding) => binding,
            SystemDecisionModel::Deployment => {
                return Err(unavailable(
                    "the organization no longer selects its own decision model".into(),
                ));
            }
        };
        return Ok(Arc::new(OrgDecisions {
            binding,
            egress: ctx.capability_service.egress_service(),
        }));
    }
    ctx.decisions
        .clone()
        .filter(|service| service.is_configured())
        .ok_or_else(|| {
            CommandError::unavailable(
                "Suggesting labels needs a decision service, which is not configured on \
                 this deployment",
            )
            .with_code("decisions_not_configured")
        })
}

/// The org's decision default behind the deployment service's interface.
struct OrgDecisions {
    binding: DecisionModelBinding,
    egress: Arc<dyn everruns_core::EgressService>,
}

#[async_trait]
impl DecisionsService for OrgDecisions {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(
        &self,
        request: DecisionRequest,
    ) -> everruns_contracts::error::Result<DecisionOutcome> {
        everruns_integrations::typesafe::evaluate_unmetered(
            &self.binding,
            request,
            self.egress.as_ref(),
        )
        .await
        .map_err(everruns_contracts::error::AgentLoopError::tool)
    }
}
