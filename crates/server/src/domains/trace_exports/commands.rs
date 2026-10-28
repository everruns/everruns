//! Routing participates in the same history, conflict guard and saved Undo as other page forms.
use super::{RoutingMode, RoutingPolicy};
use crate::domains::common::*;
use everruns_core::{Permission, Policy, Rule};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
pub const TRACING_MANAGE: Policy = Policy {
    id: "tracing.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};

async fn target(ctx: &Ctx, id: &str) -> Result<Option<Uuid>, CommandError> {
    if id == ctx.caller.org_public_id {
        return Ok(None);
    }
    let agent = id
        .parse::<everruns_contracts::typed_id::AgentId>()
        .map_err(|_| CommandError::not_found("Tracing policy"))?;
    let row = ctx
        .db
        .get_agent(ctx.org_id(), agent)
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    if row.is_built_in && row.name == crate::platform_chat_agent::NAME {
        return Err(CommandError::bad_request(
            "Personal Chat cannot export organization traces",
        ));
    }
    Ok(Some(agent.uuid()))
}
/// Identify the organization or agent whose routing policy should be read.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetTracingPolicy {
    /// Organization public ID or agent public ID whose routing policy is selected.
    #[schema(example = "agent_01933b5a000070008000000000000001")]
    pub id: String,
}
/// Safe organization or agent routing metadata returned by native commands.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct TracingPolicyMetadata {
    /// Organization public ID or agent public ID whose routing policy is selected.
    #[schema(example = "agent_01933b5a000070008000000000000001")]
    pub id: String,
    /// Use organization defaults, add selections, replace defaults, or disable customer exports.
    #[schema(example = "inherit")]
    pub mode: RoutingMode,
    /// Distinct destination identifiers from this organization; at most eight selections.
    pub destination_ids: Vec<Uuid>,
}
#[command(name="get_tracing_policy",http=plain,summary="Get tracing policy",params(("id"=String,Path,description="Tracing resource ID")),category="tracing",description="Read organization or agent trace routing without credentials.",method="GET",path="/v1/tracing/policies/{id}",policy=TRACING_MANAGE,cli=CliRoute::new(&["tracing","routing"],"get").with_examples(&[CliExample::new("Read an agent's external tracing policy","everruns tracing routing get --id agent_01933b5a000070008000000000000001")]))]
impl Command for GetTracingPolicy {
    type Output = TracingPolicyMetadata;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = target(ctx, &self.id).await?;
        let view = ctx.db.tracing_policy(ctx.org_id(), agent).await?;
        Ok(TracingPolicyMetadata {
            id: self.id,
            mode: if agent.is_none() {
                RoutingMode::Replace
            } else {
                view.policy.mode
            },
            destination_ids: view.policy.destination_ids,
        })
    }
}
/// Set future-turn routing with an optional optimistic revision guard.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateTracingPolicy {
    /// Organization public ID or agent public ID whose routing policy is selected.
    #[schema(example = "agent_01933b5a000070008000000000000001")]
    pub id: String,
    /// Use organization defaults, add selections, replace defaults, or disable customer exports.
    #[schema(example = "inherit")]
    pub mode: RoutingMode,
    /// Distinct destination identifiers from this organization; at most eight selections.
    pub destination_ids: Vec<Uuid>,
    #[serde(default)]
    /// Current policy revision; omit to use the revision read during this command.
    #[schema(example = 1)]
    pub expected_revision: Option<i64>,
}
#[command(name="update_tracing_policy",http=plain,params(("id"=String,Path,description="Tracing resource ID")),category="tracing",description="Set organization or agent destinations for future turns. Requires organization settings authority; never accepts credentials.",method="PATCH",path="/v1/tracing/policies/{id}",policy=TRACING_MANAGE,cli=CliRoute::new(&["tracing","routing"],"update").with_examples(&[CliExample::new("Stop an agent's external trace exports","everruns tracing routing update --id agent_01933b5a000070008000000000000001 --mode off --destination-ids '[]' --reason 'Disable external exports'")]))]
impl Command for UpdateTracingPolicy {
    type Output = TracingPolicyMetadata;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let agent = target(ctx, &self.id).await?;
        let p = RoutingPolicy {
            mode: self.mode,
            destination_ids: self.destination_ids,
        };
        p.validate().map_err(CommandError::bad_request)?;
        if agent.is_none() && p.mode != RoutingMode::Replace {
            return Err(CommandError::bad_request(
                "Organization defaults require selected destinations",
            ));
        }
        let revision = match self.expected_revision {
            Some(r) => r,
            None => ctx.db.tracing_policy(ctx.org_id(), agent).await?.revision,
        };
        if !ctx
            .db
            .save_tracing_policy(ctx.org_id(), agent, revision, &p)
            .await
            .map_err(|_| CommandError::bad_request("Unknown destination selection"))?
        {
            return Err(CommandError::conflict(
                "Routing changed; reload before saving",
            ));
        }
        Ok(TracingPolicyMetadata {
            id: self.id,
            mode: p.mode,
            destination_ids: p.destination_ids,
        })
    }
}
