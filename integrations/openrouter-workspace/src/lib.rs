//! OpenRouter workspace inspection and model-scout integrations for Everruns.
//!
//! This crate keeps provider-workspace HTTP behavior outside the neutral
//! execution kernel while adapting it to Everruns capability, credential, and
//! egress contracts.
//!
//! It is part of the [Everruns](https://everruns.com) ecosystem and is composed
//! by the hosted platform rather than the default Framework facade.
//!
//! # Example
//!
//! ```
//! use everruns_core::Capability;
//! use everruns_integrations_openrouter_workspace::OpenRouterWorkspaceCapability;
//!
//! assert_eq!(OpenRouterWorkspaceCapability.id(), "openrouter_workspace");
//! ```

use everruns_core::capabilities::{
    AgentBlueprint, BlueprintModel, Capability, CapabilityLocalization, CapabilityStatus, RiskLevel,
};
use everruns_core::*;

mod model_scout;
mod workspace;

pub use model_scout::{
    MODEL_SCOUT_CAPABILITY_ID, ModelRanking, ModelScoutCapability, ProbeResult, ProbeTask,
    RouterUpdateProposal, compute_score, rank_results,
};
pub use workspace::{
    OPENROUTER_WORKSPACE_CAPABILITY_ID, OpenRouterKeyInfo, OpenRouterRateLimit,
    OpenRouterWorkspaceCapability, PolicyCompatibilityReport, WorkspacePolicyDrift,
    detect_policy_drift,
};
