//! Organization research model: the org's strongest model for hard reasoning
//! and long research, and the agent model role that follows it.
//!
//! Decision: an unset `research_model_id` means Automatic and is resolved when
//! used, never stored, so a new flagship reaches every research agent as soon
//! as the provider recommends it. Automatic picks the first usable model in
//! this order: Anthropic's recommended research model, OpenAI's, any other
//! connected provider's, then the org default (the ordinary fallback chain).
//! An explicit selection is used as stored, like the other org defaults.

use crate::storage::{ModelWithProviderRow, StorageBackend};
use anyhow::Result;
use everruns_contracts::driver_registry::DriverRegistry;
use everruns_contracts::provider::DriverId;
use everruns_contracts::typed_id::ModelId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// An organization model an agent follows instead of pinning a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelRole {
    /// The organization research model (Settings › Models › Defaults).
    Research,
}

impl AgentModelRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Research => "research",
        }
    }

    pub fn from_db(value: Option<&str>) -> Option<Self> {
        match value? {
            "research" => Some(Self::Research),
            _ => None,
        }
    }
}

/// Provider types tried first, in order. Anything else follows in the order
/// the org lists its models.
const PREFERRED_PROVIDERS: [&str; 2] = ["anthropic", "openai"];

/// Whether a model can serve turns now: enabled, a chat model on an active,
/// shared provider with credentials.
fn usable(model: &ModelWithProviderRow) -> bool {
    model.enabled
        && model.provider_status == "active"
        && (model.provider_api_key_set || model.provider_type == "llmsim")
        && model.provider_type != "chatgpt"
        && crate::domains::models::catalog::require_chat(model.provider_metadata.as_ref()).is_ok()
}

/// Pick the Automatic research model from an org's models: the first usable
/// model whose id is its provider's recommended research model, preferring
/// Anthropic, then OpenAI, then other providers in listing order.
pub fn automatic_pick(
    registry: &DriverRegistry,
    models: &[ModelWithProviderRow],
) -> Option<ModelId> {
    let recommended = |model: &ModelWithProviderRow| {
        model
            .provider_type
            .parse::<DriverId>()
            .ok()
            .and_then(|driver| registry.recommended_models(&driver))
            .is_some_and(|picks| picks.research_model == model.model_id)
    };
    let rank = |model: &ModelWithProviderRow| {
        PREFERRED_PROVIDERS
            .iter()
            .position(|kind| *kind == model.provider_type)
            .unwrap_or(PREFERRED_PROVIDERS.len())
    };
    models
        .iter()
        .enumerate()
        .filter(|(_, model)| usable(model) && recommended(model))
        .min_by_key(|(index, model)| (rank(model), *index))
        .map(|(_, model)| model.id)
}

/// The organization research model: the explicit selection, else the
/// Automatic pick. `None` means the caller falls back to the org default.
pub async fn org_research_model_id(db: &StorageBackend, org_id: i64) -> Result<Option<ModelId>> {
    if let Some(id) = db
        .get_organization_settings(org_id)
        .await?
        .and_then(|settings| settings.research_model_id)
    {
        return Ok(Some(id));
    }
    let models = db.list_all_models(org_id).await?;
    Ok(automatic_pick(
        &crate::domains::common::default_driver_registry(),
        &models,
    ))
}

/// What the research model resolves to for display: the research model, else
/// the org default model that turns would fall back to.
pub async fn effective_research_model_id(
    db: &StorageBackend,
    org_id: i64,
) -> Result<Option<ModelId>> {
    if let Some(id) = org_research_model_id(db, org_id).await? {
        return Ok(Some(id));
    }
    Ok(db.get_default_model(org_id).await?.map(|model| model.id))
}

/// The model an agent's model role resolves to, if the agent follows one.
pub async fn agent_role_model_id(
    db: &StorageBackend,
    org_id: i64,
    role: Option<AgentModelRole>,
) -> Result<Option<ModelId>> {
    match role {
        Some(AgentModelRole::Research) => org_research_model_id(db, org_id).await,
        None => Ok(None),
    }
}

/// The model a turn runs on before the organization default: the session's,
/// the agent's pinned model or model role, the harness default, then the
/// built-in Chat agent's org Chat model. `None` means the org default.
pub async fn turn_model_id(
    db: &StorageBackend,
    org_id: i64,
    session_model: Option<ModelId>,
    agent: Option<&crate::domains::agents::record::Agent>,
    harness_model: Option<ModelId>,
) -> Result<Option<ModelId>> {
    if let Some(id) = session_model.or(agent.and_then(|a| a.default_model_id)) {
        return Ok(Some(id));
    }
    if let Some(id) = agent_role_model_id(db, org_id, agent.and_then(|a| a.model_role)).await? {
        return Ok(Some(id));
    }
    if let Some(id) = harness_model {
        return Ok(Some(id));
    }
    match agent {
        Some(agent) => {
            crate::platform_chat_agent::org_chat_model_id(
                db,
                org_id,
                &agent.name,
                agent.is_built_in,
            )
            .await
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests;
