//! Platform agents compiled to Engine declarations (data plane v2 step 4).
//!
//! The control plane is the platform's front end to the Engine: it compiles
//! what a turn runs on, a stored agent (or none, for a harness-only session)
//! with its harness chain and the organization's defaults, into one
//! [`RuntimeAgent`], the id-free declaration every Engine agent hydrates
//! from.
//!
//! Decisions:
//! - The declaration names things and carries no stored id. The model is a
//!   provider and model name, MCP credentials are `${BINDING}` placeholders,
//!   and capabilities are names. The host maps a session back to the
//!   platform's records when it needs them (budgets, connections, events).
//! - The harness chain and the organization's defaults are resolved here,
//!   once per revision, with the same rules a turn applied when it read its
//!   configuration (`handle_get_turn_context`): feature flags filter
//!   capabilities, the platform chat agent gets its page tools, and the
//!   model follows the agent, its model role, the harness, then the org.
//! - What depends on the turn rather than the agent (the author's own MCP
//!   servers, the session's overlay, history) is not in the declaration: the
//!   host and the Engine's live controls supply it per turn.

use everruns::hosting::RuntimeAgent;
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::runtime::agent_definition::AgentDefinition;
use everruns_contracts::runtime::config_layer::AgentConfigOverlay;
use everruns_contracts::runtime::harness_definition::HarnessDefinition;
use everruns_contracts::typed_id::{AgentId, HarnessId};
use serde_json::{Map, Value, json};

use crate::storage::StorageBackend;

pub mod sessions;

/// The layers a platform turn runs on, already filtered by the
/// organization's feature flags.
pub struct PlatformLayers {
    /// The harness, its chain merged.
    pub harness: HarnessDefinition,
    /// The agent layer; `None` for a harness-only session.
    pub agent: Option<AgentDefinition>,
    /// Extra agent fields the turn reads that the execution definition does
    /// not carry (sandbox policy).
    pub sandbox_policy: Option<Value>,
    /// The resolved model, by provider and model name.
    pub model: Option<ModelSpec>,
}

/// Compile `layers` into a declaration. Pure: equal layers give equal
/// declarations.
pub fn compile(layers: &PlatformLayers) -> anyhow::Result<RuntimeAgent> {
    let mut overlays = vec![AgentConfigOverlay::from(&layers.harness)];
    if let Some(agent) = &layers.agent {
        overlays.push(AgentConfigOverlay::from(agent));
    }
    let merged = AgentConfigOverlay::fold(overlays);
    let mut manifest = Map::new();
    manifest.insert("schema_version".into(), json!(1));
    match &layers.agent {
        Some(agent) => {
            manifest.insert("name".into(), json!(package_name(&agent.name)));
            let display_name = agent.display_name.as_deref().unwrap_or(&agent.name);
            manifest.insert("display_name".into(), json!(display_name));
            if let Some(description) = &agent.description {
                manifest.insert("description".into(), json!(description));
            }
            if agent.completion_check != Default::default() {
                manifest.insert(
                    "completion_check".into(),
                    serde_json::to_value(agent.completion_check)?,
                );
            }
        }
        None => {
            manifest.insert("name".into(), json!(package_name(&layers.harness.name)));
            manifest.insert("display_name".into(), json!(layers.harness.name));
        }
    }
    manifest.insert(
        "instructions".into(),
        json!(merged.system_prompt.unwrap_or_default()),
    );
    if let Some(model) = &layers.model {
        manifest.insert("model".into(), serde_json::to_value(model)?);
    }
    if !merged.capabilities.is_empty() {
        manifest.insert(
            "capabilities".into(),
            serde_json::to_value(&merged.capabilities)?,
        );
    }
    if !merged.initial_files.is_empty() {
        manifest.insert(
            "initial_files".into(),
            serde_json::to_value(&merged.initial_files)?,
        );
    }
    if !merged.tools.is_empty() {
        manifest.insert("tools".into(), serde_json::to_value(&merged.tools)?);
    }
    if !merged.mcp_servers.is_empty() {
        manifest.insert(
            "mcp_servers".into(),
            serde_json::to_value(everruns_core::agent_package::redact_mcp(
                &merged.mcp_servers,
            ))?,
        );
    }
    if let Some(network_access) = &merged.network_access {
        manifest.insert(
            "network_access".into(),
            serde_json::to_value(network_access)?,
        );
    }
    if let Some(max_iterations) = merged.max_iterations {
        manifest.insert("max_iterations".into(), json!(max_iterations));
    }
    if let Some(parallel) = merged.parallel_tool_calls {
        manifest.insert("parallel_tool_calls".into(), json!(parallel));
    }
    if let Some(max_output_tokens) = merged.max_output_tokens {
        manifest.insert("max_output_tokens".into(), json!(max_output_tokens));
    }
    if let Some(communication) = merged.communication
        && communication != Default::default()
    {
        manifest.insert("communication".into(), serde_json::to_value(communication)?);
    }
    if let Some(policy) = &layers.sandbox_policy {
        manifest.insert("sandbox_policy".into(), policy.clone());
    }
    // Through the package codec once more, so the stored form is canonical:
    // equal agents compile to equal bytes however their layers were written.
    let compiled = RuntimeAgent::new(Value::Object(manifest))
        .package()
        .and_then(|package| RuntimeAgent::from_package(&package))
        .map_err(|error| anyhow::anyhow!("platform agent does not compile: {error}"))?;
    Ok(compiled)
}

/// Resolve the layers of a turn on `harness_id` answered by `agent_id`, with
/// the rules the per-turn configuration read applies.
pub async fn resolve(
    db: &StorageBackend,
    org_id: i64,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
) -> anyhow::Result<PlatformLayers> {
    let mut agent = match agent_id {
        Some(agent_id) => Some(
            crate::domains::agents::queries::get_for_execution(db, org_id, &agent_id.to_string())
                .await?
                .ok_or_else(|| anyhow::anyhow!("agent {agent_id} not found"))?,
        ),
        None => None,
    };
    let mut harness = crate::domains::harnesses::queries::resolve_effective(db, org_id, harness_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("harness {harness_id} not found"))?;
    let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
        db,
        org_id,
        &crate::records::FeatureFlagPolicy::current(),
    )
    .await?;
    if let Some(agent) = agent.as_mut() {
        crate::platform_chat_agent::add_page_tools(agent, flags.chat_mode);
        let is_platform_chat = agent.is_built_in && agent.name == crate::platform_chat_agent::NAME;
        agent.capabilities.retain(|capability| {
            flags.is_agent_capability_enabled(capability.capability_id(), is_platform_chat)
        });
    }
    harness
        .capabilities
        .retain(|capability| flags.is_capability_enabled(capability.capability_id()));
    let harness_definition = harness.definition();
    let model_id = crate::domains::organizations::research_model::turn_model_id(
        db,
        org_id,
        None,
        agent.as_ref(),
        harness_definition.default_model_id,
    )
    .await?;
    let model_row = match model_id {
        Some(id) => db.get_model_with_provider(org_id, id.uuid()).await?,
        None => db.get_default_model(org_id).await?,
    };
    // The declaration names its provider by name, so the name must pick one
    // provider (the runtime resolves it the same way).
    if let Some(row) = &model_row {
        let key = everruns_contracts::runtime_provider::ProviderKey::new(&row.provider_name);
        let named = db
            .list_providers(org_id)
            .await?
            .iter()
            .filter(|provider| {
                everruns_contracts::runtime_provider::ProviderKey::new(&provider.name) == key
            })
            .count();
        anyhow::ensure!(
            named == 1,
            "provider name {} names {named} providers",
            row.provider_name
        );
    }
    Ok(PlatformLayers {
        harness: harness_definition,
        sandbox_policy: agent
            .as_ref()
            .and_then(|agent| agent.sandbox_policy.as_ref())
            .map(serde_json::to_value)
            .transpose()?,
        agent: agent.as_ref().map(|agent| agent.definition()),
        model: model_row.map(|row| ModelSpec::on(row.provider_name, row.model_id)),
    })
}

/// A platform name as a package name: lowercase letters, digits and single
/// hyphens, at most 255 characters. Platform names are free text ("Support
/// Bot"); the original stays the display name.
pub fn package_name(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 255 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "agent".to_string()
    } else {
        slug.to_string()
    }
}

/// The digest the publisher compares to skip no-ops: sha256 of the
/// declaration's canonical JSON (`serde_json::Value` keeps keys sorted).
pub fn digest(declaration: &RuntimeAgent) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_vec(&serde_json::to_value(declaration)?)?;
    Ok(hex::encode(Sha256::digest(&canonical)))
}

/// Compile `key` from the platform's records and publish it as the key's
/// next revision when it changed. Returns the current revision.
pub async fn publish(
    db: &StorageBackend,
    key: &crate::storage::EngineAgentKeyRow,
) -> anyhow::Result<String> {
    let layers = resolve(
        db,
        key.org_id,
        HarnessId::from_uuid(key.harness_id),
        key.agent_id.map(AgentId::from_uuid),
    )
    .await?;
    let declaration = compile(&layers)?;
    let digest = digest(&declaration)?;
    db.publish_engine_agent(
        &key.agent_key,
        &serde_json::to_value(&declaration)?,
        &digest,
    )
    .await
}

/// The key and current revision for sessions on `harness_id` answered by
/// `agent_id`, publishing the first revision on first use.
pub async fn key_for(
    db: &StorageBackend,
    org_id: i64,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
) -> anyhow::Result<(String, String)> {
    let key = db
        .engine_agent_key(org_id, harness_id.uuid(), agent_id.map(|id| id.uuid()))
        .await?;
    if let Some((revision, _)) = db.engine_agent_current(&key).await? {
        return Ok((key, revision));
    }
    let target = db
        .engine_agent_key_target(&key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("engine agent key {key} vanished"))?;
    let revision = publish(db, &target).await?;
    Ok((key, revision))
}

/// Drain up to `limit` queued edits: republish every key each one can
/// change. A failed key is logged and left on its previous revision; the
/// next edit or sweep republishes it.
pub async fn drain(db: &StorageBackend, limit: i64) -> anyhow::Result<usize> {
    let edits = db.take_engine_agent_publishes(limit).await?;
    let mut seen = std::collections::HashSet::new();
    for edit in &edits {
        // A harness edit or an org default reaches every key of the org.
        let agent = if edit.harness_id.is_some() {
            None
        } else {
            edit.agent_id
        };
        for key in db.engine_agent_keys_for_edit(edit.org_id, agent).await? {
            if !seen.insert(key.agent_key.clone()) {
                continue;
            }
            if let Err(error) = publish(db, &key).await {
                tracing::warn!(%error, agent_key = %key.agent_key, "engine agent not republished");
            }
        }
    }
    Ok(edits.len())
}

/// How often each server drains the publish queue. Queued edits apply from
/// the next turn that starts after this.
const DRAIN_EVERY: std::time::Duration = std::time::Duration::from_secs(1);
/// Most queued edits taken per drain.
const DRAIN_BATCH: i64 = 100;

/// Drain the publish queue on this server. Every replica runs it; rows are
/// taken with `SKIP LOCKED`, so each edit is published once.
pub async fn run_publisher(db: std::sync::Arc<StorageBackend>) {
    let mut interval = tokio::time::interval(DRAIN_EVERY);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        loop {
            match drain(&db, DRAIN_BATCH).await {
                Ok(taken) if taken as i64 == DRAIN_BATCH => continue,
                Ok(_) => break,
                Err(error) => {
                    tracing::warn!(%error, "engine agent publish queue not drained");
                    break;
                }
            }
        }
    }
}

/// Durable schedule name of the publish sweep.
pub const PUBLISH_SWEEP_SCHEDULE: &str = "engine-agent-publish-sweep";
/// Activity type the publish sweep enqueues.
pub const PUBLISH_SWEEP_ACTIVITY: &str = "engine_agent_publish_sweep";

/// The backstop for edits that reach a definition without an entity change
/// (a feature flag rollout, a provider's model list): every ten minutes,
/// queue a republish of every organization that has Engine agents.
/// Unchanged keys cost a compile and a digest comparison.
pub fn sweep_job(
    db: std::sync::Arc<StorageBackend>,
) -> crate::background::cluster_jobs::ClusterJob {
    crate::background::cluster_jobs::ClusterJob::every(
        PUBLISH_SWEEP_SCHEDULE,
        PUBLISH_SWEEP_ACTIVITY,
        "Republishes Engine agent definitions whose platform configuration changed outside an edit.",
        std::time::Duration::from_secs(600),
        move || {
            let db = db.clone();
            Box::pin(async move {
                match db.engine_agent_orgs().await {
                    Ok(orgs) => {
                        for org_id in orgs {
                            if let Err(error) = db
                                .enqueue_engine_agent_publish(org_id, None, None, "sweep")
                                .await
                            {
                                tracing::warn!(%error, org_id, "engine agent sweep not queued");
                            }
                        }
                    }
                    Err(error) => tracing::warn!(%error, "engine agent sweep failed"),
                }
            })
        },
    )
}

#[cfg(test)]
mod tests;
