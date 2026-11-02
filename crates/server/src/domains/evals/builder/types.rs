// Eval Builder API types.

use crate::domains::evals::types::CreateEvalCaseRequest;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Where a request for proposals is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum EvalBuilderStatus {
    /// Reading the agent and its sessions.
    Running,
    /// Proposals are ready (possibly none).
    Done,
    /// The model call failed or its answer was unusable, or the run was
    /// interrupted.
    Failed,
    /// The organization has no default model the Eval Builder can use.
    Unavailable,
}

/// One request to the Eval Builder for an agent.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EvalBuilderRun {
    /// Run identifier.
    #[schema(example = "evalbuild_01933b5a000070008000000000000001")]
    pub id: String,
    /// Where the run is.
    pub status: EvalBuilderStatus,
    /// When it started.
    pub started_at: DateTime<Utc>,
    /// When it finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Model that wrote the proposals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "gpt-6.1-sol")]
    pub model: Option<String>,
    /// What the model call cost, when the provider reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 0.012)]
    pub cost_usd: Option<f64>,
    /// Production sessions it read.
    #[schema(example = 6)]
    pub sessions_read: i32,
    /// Proposals it made.
    #[schema(example = 5)]
    pub proposals: i32,
    /// Why it failed, in words for a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What part of the agent's job a proposed case covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalCategory {
    /// What the agent is for, done well.
    HappyPath,
    /// A situation the instructions cover that is easy to get wrong.
    EdgeCase,
    /// A way the agent went wrong in real sessions.
    Failure,
    /// Someone or something trying to misuse the agent: injected
    /// instructions, out-of-scope requests, abuse attempts.
    Adversarial,
}

impl ProposalCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HappyPath => "happy_path",
            Self::EdgeCase => "edge_case",
            Self::Failure => "failure",
            Self::Adversarial => "adversarial",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "happy_path" => Some(Self::HappyPath),
            "edge_case" => Some(Self::EdgeCase),
            "failure" => Some(Self::Failure),
            "adversarial" => Some(Self::Adversarial),
            _ => None,
        }
    }
}

/// Whether a proposal is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProposalStatus {
    /// Waiting for a person.
    Proposed,
    /// Added to a dataset as a case.
    Accepted,
    /// Turned down.
    Dismissed,
}

/// A case the Eval Builder suggests adding to one of the agent's datasets.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EvalCaseProposal {
    /// Proposal identifier.
    #[schema(example = "evalprop_01933b5a000070008000000000000001")]
    pub id: String,
    /// Agent the case is for.
    #[schema(example = "agent_01933b5a000070008000000000000001")]
    pub agent_id: String,
    /// Whether it is decided.
    pub status: ProposalStatus,
    /// Short name of the case.
    #[schema(example = "Refund over the approval limit")]
    pub name: String,
    /// What part of the job it covers.
    pub category: ProposalCategory,
    /// `simulated` when a model plays the user, otherwise `scripted`.
    #[schema(example = "simulated")]
    pub kind: String,
    /// Why the Eval Builder suggests it, in one or two sentences.
    pub reason: String,
    /// Sessions that inspired it, by public identifier.
    #[schema(example = json!(["session_01933b5a000070008000000000000001"]))]
    pub source_session_ids: Vec<String>,
    /// The case as it would be created.
    pub case: CreateEvalCaseRequest,
    /// When it was proposed.
    pub created_at: DateTime<Utc>,
    /// Note left when it was dismissed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dismiss_note: Option<String>,
}

/// An agent's open proposals and its latest Eval Builder run.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EvalProposalList {
    /// Open proposals, newest first.
    pub data: Vec<EvalCaseProposal>,
    /// The most recent run, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_run: Option<EvalBuilderRun>,
}

/// Request to add a proposal to a dataset.
#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct AcceptEvalProposalRequest {
    /// Dataset (eval) to add the case to.
    #[schema(example = "eval_01933b5a000070008000000000000001")]
    pub eval_id: String,
}

/// Request to turn a proposal down.
#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct DismissEvalProposalRequest {
    /// Why it is not wanted, kept so the builder does not offer it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "We do not support refunds over the limit at all.")]
    pub note: Option<String>,
}
