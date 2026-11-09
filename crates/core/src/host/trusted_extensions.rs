//! Trusted host extensions: the platform services an embedded host forwards
//! to the turn, grouped by concern (data plane v2 step 2,
//! `knowledge/execution/data-plane-v2/platform-on-engine.md`).
//!
//! Decision: one bundle on [`HostBackends`](crate::host::HostBackends), not a
//! field per service, and no Framework builder method. Only a host that
//! builds the runtime's backends itself (a `SessionHosting` profile, through
//! its activation hook) can set it, so a Framework application keeps the
//! surface and the rules it has. Every slot is optional; an empty bundle
//! leaves [`InProcessRuntime`](crate::host::InProcessRuntime) exactly as it
//! was. Each slot forwards to the existing
//! [`RuntimeHostAdapter`](crate::host::RuntimeHostAdapter) method of the same
//! concern, so the turn uses it the way a worker turn already does.
//!
//! Decision: an embedded runtime serves one organization, and one session
//! per activation, so slots hold the service itself. Slots whose adapter
//! method also names the agent or the input take a function of those.

use std::sync::Arc;

use everruns_contracts::hosted_mcp::HostedMcpResolver;
use everruns_contracts::knowledge::KnowledgeRuntime;
use everruns_contracts::typed_id::{AgentId, MessageId};

use crate::delegation_services::{SessionCreationAuthority, SubagentSpawnStore};
use crate::engine::tool_detach::ToolDetacher;
use crate::host::WaitsOnPerson;
use crate::tool_execution::{BudgetChecker, OutboundToolRateLimiter, PaymentAuthority};
use crate::{
    DecisionsService, EgressService, UtilityLlmService, agents_api_store::AgentsApiStore,
    connection_services::ProviderCredentialStore, durability::DurableToolResultStore,
    file_services::FileResolver, image_services::ImageArtifactStore, image_services::ImageResolver,
    session_services::LeasedResourceStore, session_services::SessionResourceRegistry,
};

/// A service chosen per agent (budgets and payment are per agent).
pub type PerAgent<T> = Arc<dyn Fn(Option<AgentId>) -> Option<Arc<T>> + Send + Sync>;

/// A service chosen per agent and admitted input.
pub type PerInput<T> = Arc<dyn Fn(Option<AgentId>, MessageId) -> Option<Arc<T>> + Send + Sync>;

/// Platform services a trusted host forwards to the turn. Stability:
/// experimental.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct HostExtensions {
    /// Services the runtime's composition otherwise fixes.
    pub composition: CompositionExtensions,
    /// Who may spend, pay, create sessions and call out.
    pub authorities: AuthorityExtensions,
    /// Platform stores and resolvers.
    pub stores: StoreExtensions,
    /// Turn hooks.
    pub hooks: HookExtensions,
}

/// Replace a service the runtime's composition fixes. Stability:
/// experimental.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct CompositionExtensions {
    /// Outbound HTTP for web tools (SSRF guard, rate limits).
    pub egress: Option<Arc<dyn EgressService>>,
    /// The utility model behind titles, summaries and checks.
    pub utility_llm: Option<Arc<dyn UtilityLlmService>>,
    /// Typed decisions for capability internals.
    pub decisions: Option<Arc<dyn DecisionsService>>,
    /// Built-in capabilities the host replaces by id, with the platform's
    /// own version (for example a filesystem or web fetch over platform
    /// services).
    ///
    /// Decision: the only exception to the rule that a host capability never
    /// shadows a built-in (TM-FS-021), and only here, where a Framework
    /// application cannot reach. Listing one is the declaration; one whose id
    /// the runtime has no capability for fails the build, so a typo never
    /// adds a capability silently.
    pub replaced_capabilities: Vec<Arc<dyn crate::Capability>>,
}

/// Authorities the turn asks before it acts. Stability: experimental.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct AuthorityExtensions {
    /// Budget checks before model and tool calls.
    pub budget: Option<PerAgent<dyn BudgetChecker>>,
    /// Payment for paid tools.
    pub payment: Option<PerAgent<dyn PaymentAuthority>>,
    /// Creating child sessions (subagents, teams).
    pub session_creation: Option<Arc<dyn SessionCreationAuthority>>,
    /// The organization's outbound tool-call rate limiter.
    pub outbound_rate_limiter: Option<Arc<dyn OutboundToolRateLimiter>>,
}

/// Platform stores and resolvers. Stability: experimental.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct StoreExtensions {
    /// Provider credentials stored by the organization.
    pub provider_credentials: Option<Arc<dyn ProviderCredentialStore>>,
    /// Leased resources (sandboxes).
    pub leased_resources: Option<Arc<dyn LeasedResourceStore>>,
    /// Claimed and settled tool results, so a resumed act does not run a
    /// call twice.
    pub durable_tool_results: Option<Arc<dyn DurableToolResultStore>>,
    /// Resources a session holds.
    pub session_resources: Option<Arc<dyn SessionResourceRegistry>>,
    /// Subagent spawn handles, for reattach after a crash.
    pub subagent_spawns: Option<Arc<dyn SubagentSpawnStore>>,
    /// Images by reference.
    pub images: Option<Arc<dyn ImageResolver>>,
    /// Files by reference.
    pub files: Option<Arc<dyn FileResolver>>,
    /// Images tools produce.
    pub image_artifacts: Option<Arc<dyn ImageArtifactStore>>,
    /// The Agents API record store.
    pub agents_api: Option<Arc<dyn AgentsApiStore>>,
}

/// Turn hooks. Stability: experimental.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct HookExtensions {
    /// Hosted MCP servers (prefix lookup).
    pub hosted_mcp: Option<PerInput<dyn HostedMcpResolver>>,
    /// Knowledge for automatic context: a turn whose `knowledge` capability
    /// is `automatic` gets retrieved evidence added to its input.
    pub knowledge: Option<Arc<dyn KnowledgeRuntime>>,
    /// Hands a long tool call to the host so the turn can park.
    pub tool_detacher: Option<Arc<dyn ToolDetacher>>,
    /// Tool calls that wait on a person, so resume may run them again; adds
    /// to the runtime's own predicate.
    pub waits_on_person: Option<WaitsOnPerson>,
}

/// Whether the turn's `knowledge` capability is configured `automatic`.
pub fn automatic_knowledge(capabilities: &[crate::AgentCapabilityConfig]) -> bool {
    capabilities.iter().any(|capability| {
        capability.capability_id() == "knowledge"
            && capability
                .config_value()
                .get("automatic")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
    })
}

/// Add automatic Knowledge evidence to a turn's input when its `knowledge`
/// capability is configured `automatic`: one retrieval for the latest user
/// message, appended as a user message. A retrieval failure leaves the
/// input as it was; file access still works. Shared by the worker host and
/// [`HookExtensions::knowledge`].
pub async fn add_automatic_knowledge(
    knowledge: &dyn KnowledgeRuntime,
    capabilities: &[crate::AgentCapabilityConfig],
    session: everruns_contracts::typed_id::SessionId,
    input: MessageId,
    messages: &mut Vec<crate::RuntimeMessage>,
) {
    if !automatic_knowledge(capabilities) {
        return;
    }
    let query = messages
        .iter()
        .rev()
        .find(|message| message.role == crate::RuntimeMessageRole::User)
        .and_then(|message| message.text())
        .unwrap_or("")
        .to_string();
    match knowledge.automatic_context(session, input, query).await {
        Ok(Some(evidence)) => messages.push(crate::RuntimeMessage::user(evidence)),
        Ok(None) => {}
        Err(_) => tracing::warn!(
            session_id = %session,
            "Automatic Knowledge retrieval unavailable; file access remains available"
        ),
    }
}

#[cfg(test)]
#[path = "trusted_extensions_tests.rs"]
mod tests;
