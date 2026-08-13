//! OpenRouter workspace, model-scout, and server-tool integrations for Everruns.
//!
//! This crate keeps provider-workspace HTTP behavior outside the neutral
//! execution kernel while adapting it to Everruns capability, credential, and
//! egress contracts. The server-tool capability adapts agent configuration to
//! OpenRouter's provider-executed routing contract without teaching the core
//! registry about a concrete provider or capability ID.
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
mod server_tools;
mod workspace;

pub use model_scout::{
    MODEL_SCOUT_CAPABILITY_ID, ModelRanking, ModelScoutCapability, ProbeResult, ProbeTask,
    RouterUpdateProposal, compute_score, rank_results,
};
pub use server_tools::{
    OPENROUTER_SERVER_TOOLS_CAPABILITY_ID, OpenRouterServerToolsCapability,
    server_tools_from_config,
};
pub use workspace::{
    OPENROUTER_WORKSPACE_CAPABILITY_ID, OpenRouterKeyInfo, OpenRouterRateLimit,
    OpenRouterWorkspaceCapability, PolicyCompatibilityReport, WorkspacePolicyDrift,
    detect_policy_drift,
};
