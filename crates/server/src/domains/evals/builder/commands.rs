// Eval Builder commands: ask for proposals, list them, accept one into a
// dataset, dismiss one.

use super::types::*;
use super::{EvalProposalId, LIST_LIMIT, STALE_AFTER, present_proposal, present_run};
use crate::domains::common::*;
use crate::domains::evals::queries as q;
use crate::domains::evals::record::EvalCase;
use crate::domains::evals::types::CreateEvalCaseRequest;
use everruns_contracts::typed_id::AgentId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Most characters in a dismiss note.
const NOTE_CHARS: usize = 500;

fn require_evals_enabled(ctx: &Ctx) -> Result<(), CommandError> {
    if ctx.feature_flags.evals {
        Ok(())
    } else {
        Err(CommandError::feature_not_enabled("evals"))
    }
}

/// The agent, checked to belong to the caller's org.
async fn agent_in_org(ctx: &Ctx, agent_id: &str) -> Result<AgentId, CommandError> {
    let id: AgentId = agent_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))?;
    ctx.db
        .get_agent(ctx.caller.org_id, id)
        .await?
        .filter(|a| a.deleted_at.is_none())
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    Ok(id)
}

fn parse_proposal_id(id: &str) -> Result<EvalProposalId, CommandError> {
    id.parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid proposal ID: {e}")))
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ProposeEvalCases {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
}

#[command(
    name = "propose_eval_cases",
    category = "evals",
    summary = "Ask the Eval Builder for cases",
    description = "Ask the Eval Builder to propose eval cases for an agent from its instructions, its recent sessions and the gaps in its datasets. Runs in the background on the organization's default model; nothing is added until a person accepts a proposal.",
    method = "POST",
    cli = CliRoute::new(&["evals", "proposals"], "create").with_examples(&[CliExample::new("Ask for cases that cover what the support agent got wrong this week", "everruns evals proposals create --agent-id agent_01h9")]),
    path = "/v1/agents/{agent_id}/eval-proposals",
    policy = crate::domains::evals::EVAL_RUN,
    http = created,
    responses((status = 409, description = "A run for this agent is still going")),
)]
impl Command for ProposeEvalCases {
    type Output = EvalBuilderRun;

    async fn execute(self, ctx: &Ctx) -> Result<EvalBuilderRun, CommandError> {
        require_evals_enabled(ctx)?;
        let agent_id = agent_in_org(ctx, &self.agent_id).await?;
        let Some(proposer) = q::service(ctx).proposer() else {
            return Err(CommandError::unavailable(
                "The Eval Builder is not available on this deployment",
            ));
        };
        let Some(run) = ctx
            .db
            .start_eval_builder_run(
                ctx.caller.org_id,
                agent_id.uuid(),
                ctx.caller.user_id,
                STALE_AFTER.as_secs() as i64,
            )
            .await?
        else {
            return Err(CommandError::conflict(
                "The Eval Builder is already proposing cases for this agent",
            ));
        };
        let presented = present_run(&run);
        super::spawn_build(ctx.db.clone(), proposer, run);
        Ok(presented)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct ListEvalProposals {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
}

#[command(
    name = "list_eval_proposals",
    category = "evals",
    summary = "List the Eval Builder's proposals for an agent",
    description = "List an agent's open eval case proposals, newest first, with the Eval Builder's latest run.",
    method = "GET",
    cli = CliRoute::new(&["evals", "proposals"], "list").with_examples(&[CliExample::new("Review the cases proposed for an agent", "everruns evals proposals list --agent-id agent_01h9")]),
    path = "/v1/agents/{agent_id}/eval-proposals",
    policy = crate::domains::evals::EVAL_VIEW,
    http = plain,
)]
impl Command for ListEvalProposals {
    type Output = EvalProposalList;

    async fn execute(self, ctx: &Ctx) -> Result<EvalProposalList, CommandError> {
        require_evals_enabled(ctx)?;
        let agent_id = agent_in_org(ctx, &self.agent_id).await?;
        let rows = ctx
            .db
            .list_eval_case_proposals(ctx.caller.org_id, agent_id.uuid(), "proposed", LIST_LIMIT)
            .await?;
        let data = rows
            .iter()
            .map(present_proposal)
            .collect::<anyhow::Result<Vec<_>>>()
            .map_err(classify_anyhow)?;
        let latest_run = ctx
            .db
            .latest_eval_builder_run(ctx.caller.org_id, agent_id.uuid())
            .await?
            .as_ref()
            .map(present_run);
        Ok(EvalProposalList { data, latest_run })
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct AcceptEvalProposal {
    /// Proposal's prefixed public identifier.
    pub proposal_id: String,
    #[serde(flatten)]
    pub req: AcceptEvalProposalRequest,
}

#[command(
    name = "accept_eval_proposal",
    category = "evals",
    summary = "Add a proposed case to a dataset",
    description = "Add an Eval Builder proposal to a dataset as a new case. The dataset must target the proposal's agent.",
    method = "POST",
    cli = CliRoute::new(&["evals", "proposals"], "accept").with_examples(&[CliExample::new("Add a proposed case to the agent's main dataset", "everruns evals proposals accept --proposal-id evalprop_01h9 --eval-id eval_01h9 --reason 'Cover refunds over the limit'")]),
    path = "/v1/eval-proposals/{proposal_id}/accept",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = created,
    request_body(AcceptEvalProposalRequest),
    responses((status = 409, description = "The proposal was already decided")),
)]
impl Command for AcceptEvalProposal {
    type Output = EvalCase;

    async fn execute(self, ctx: &Ctx) -> Result<EvalCase, CommandError> {
        require_evals_enabled(ctx)?;
        let proposal_id = parse_proposal_id(&self.proposal_id)?;
        let eval_id = q::parse_eval_id(&self.req.eval_id)?;
        let row = ctx
            .db
            .get_eval_case_proposal(ctx.caller.org_id, proposal_id.uuid())
            .await?
            .ok_or_else(|| CommandError::not_found("Proposal"))?;
        if row.status != "proposed" {
            return Err(CommandError::conflict("This proposal was already decided"));
        }
        // Only into a dataset for the same agent: a case written for one
        // agent's job means nothing to another.
        let eval = ctx
            .db
            .get_eval_by_public_id(ctx.caller.org_id, &eval_id.to_string())
            .await?
            .filter(|e| e.status != "deleted")
            .ok_or_else(|| CommandError::not_found("Eval"))?;
        let agent_public = AgentId::from_uuid(row.agent_id).to_string();
        let targets_agent = eval
            .target
            .as_ref()
            .and_then(|t| t.get("agent_id"))
            .and_then(|a| a.as_str())
            == Some(agent_public.as_str());
        if !targets_agent {
            return Err(CommandError::bad_request(
                "That dataset does not target the proposal's agent",
            ));
        }
        let case: CreateEvalCaseRequest =
            serde_json::from_value(row.case.clone()).map_err(|e| anyhow::anyhow!(e))?;
        let created = q::service(ctx)
            .create_case(&ctx.caller, &eval_id.to_string(), case)
            .await
            .map_err(classify_anyhow)?;
        if !ctx
            .db
            .decide_eval_case_proposal(
                ctx.caller.org_id,
                row.id,
                "accepted",
                ctx.caller.user_id,
                Some(eval.id),
                Some(created.internal_id),
                None,
            )
            .await?
        {
            // Decided concurrently: the transaction rolls the case back.
            return Err(CommandError::conflict("This proposal was already decided"));
        }
        Ok(created)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct DismissEvalProposal {
    /// Proposal's prefixed public identifier.
    pub proposal_id: String,
    #[serde(flatten)]
    pub req: DismissEvalProposalRequest,
}

#[command(
    name = "dismiss_eval_proposal",
    category = "evals",
    summary = "Turn a proposed case down",
    description = "Dismiss an Eval Builder proposal, optionally saying why. The Eval Builder does not offer it again.",
    method = "POST",
    cli = CliRoute::new(&["evals", "proposals"], "dismiss").with_examples(&[CliExample::new("Turn down a case that does not fit the agent", "everruns evals proposals dismiss --proposal-id evalprop_01h9 --note 'Out of scope for this agent'")]),
    path = "/v1/eval-proposals/{proposal_id}/dismiss",
    policy = crate::domains::evals::EVAL_MANAGE,
    http = plain,
    request_body(DismissEvalProposalRequest),
    responses((status = 409, description = "The proposal was already decided")),
)]
impl Command for DismissEvalProposal {
    type Output = EvalCaseProposal;

    async fn execute(self, ctx: &Ctx) -> Result<EvalCaseProposal, CommandError> {
        require_evals_enabled(ctx)?;
        let proposal_id = parse_proposal_id(&self.proposal_id)?;
        let note = self
            .req
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty());
        if note.is_some_and(|n| n.chars().count() > NOTE_CHARS) {
            return Err(CommandError::bad_request(format!(
                "The note can be at most {NOTE_CHARS} characters"
            )));
        }
        if !ctx
            .db
            .decide_eval_case_proposal(
                ctx.caller.org_id,
                proposal_id.uuid(),
                "dismissed",
                ctx.caller.user_id,
                None,
                None,
                note,
            )
            .await?
        {
            return match ctx
                .db
                .get_eval_case_proposal(ctx.caller.org_id, proposal_id.uuid())
                .await?
            {
                Some(_) => Err(CommandError::conflict("This proposal was already decided")),
                None => Err(CommandError::not_found("Proposal")),
            };
        }
        let row = ctx
            .db
            .get_eval_case_proposal(ctx.caller.org_id, proposal_id.uuid())
            .await?
            .ok_or_else(|| CommandError::not_found("Proposal"))?;
        present_proposal(&row).map_err(classify_anyhow)
    }
}
