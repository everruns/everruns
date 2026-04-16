// Shared turn-context assembly for runtime hosts.
//
// Decision: context assembly belongs in core because both public in-process
// runtime hosts and worker-backed hosts need the same merged view of harness,
// agent, session, messages, model resolution, and RuntimeAgent construction.

use crate::AgentCapabilityConfig;
use crate::agent::Agent;
use crate::capabilities::{
    COMPACTION_CAPABILITY_ID, CapabilityRegistry, CompactionConfig, SystemPromptContext,
    resolve_capability_configs,
};
use crate::config_layer::AgentConfigOverlay;
use crate::error::{AgentLoopError, Result};
use crate::harness::Harness;
use crate::message::{Message, MessageRole};
use crate::message_filter::MessageQuery;
use crate::message_retriever::MessageRetriever;
use crate::runtime_agent::RuntimeAgent;
use crate::runtime_agent::RuntimeAgentBuilder;
use crate::session::Session;
use crate::tool_types::ToolDefinition;
use crate::traits::{
    AgentStore, HarnessStore, LlmProviderStore, ModelWithProvider, SessionFileStore, SessionStore,
};
use crate::typed_id::{AgentId, HarnessId, ModelId, SessionId};
use std::sync::Arc;

/// Public snapshot of the assembled turn context used by reason-phase hosts.
#[derive(Debug, Clone)]
pub struct AssembledTurnContext {
    /// Full root-to-leaf harness chain.
    pub harness_chain: Vec<Harness>,
    /// Optional agent attached to the session.
    pub agent: Option<Agent>,
    /// Session being executed.
    pub session: Session,
    /// Effective overlay after merging harness chain → agent → session.
    pub effective_overlay: AgentConfigOverlay,
    /// Capability configs after dependency resolution.
    pub resolved_capability_configs: Vec<AgentCapabilityConfig>,
    /// Conversation messages after capability message filters are applied.
    pub messages: Vec<Message>,
    /// Fully assembled runtime agent for the current turn.
    pub runtime_agent: RuntimeAgent,
    /// Resolved model/provider pair used for the turn.
    pub model_with_provider: ModelWithProvider,
    /// The resolved model ID when a concrete configured model was selected.
    pub resolved_model_id: Option<ModelId>,
    /// Locale resolved from message controls/metadata or session defaults.
    pub resolved_locale: Option<String>,
    /// Compaction config extracted from merged capabilities, if present.
    pub compaction_config: Option<CompactionConfig>,
}

/// Assemble the shared reason-phase context for a turn.
#[allow(clippy::too_many_arguments)]
pub async fn assemble_turn_context(
    harness_store: &dyn HarnessStore,
    agent_store: &dyn AgentStore,
    session_store: &dyn SessionStore,
    message_retriever: &dyn MessageRetriever,
    provider_store: &dyn LlmProviderStore,
    capability_registry: &CapabilityRegistry,
    session_id: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    mcp_tool_definitions: &[ToolDefinition],
    file_store: Option<Arc<dyn SessionFileStore>>,
) -> Result<AssembledTurnContext> {
    assemble_turn_context_with_mode(
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        capability_registry,
        session_id,
        harness_id,
        agent_id,
        mcp_tool_definitions,
        file_store,
        ContextAssemblyMode::RequireMessages,
    )
    .await
}

/// Assemble the current turn context for inspection without requiring messages.
///
/// This is intended for embedders who need to inspect the merged harness/agent/session
/// configuration before the first user message is stored.
#[allow(clippy::too_many_arguments)]
pub async fn inspect_turn_context(
    harness_store: &dyn HarnessStore,
    agent_store: &dyn AgentStore,
    session_store: &dyn SessionStore,
    message_retriever: &dyn MessageRetriever,
    provider_store: &dyn LlmProviderStore,
    capability_registry: &CapabilityRegistry,
    session_id: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    mcp_tool_definitions: &[ToolDefinition],
    file_store: Option<Arc<dyn SessionFileStore>>,
) -> Result<AssembledTurnContext> {
    assemble_turn_context_with_mode(
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        capability_registry,
        session_id,
        harness_id,
        agent_id,
        mcp_tool_definitions,
        file_store,
        ContextAssemblyMode::AllowEmptyMessages,
    )
    .await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContextAssemblyMode {
    RequireMessages,
    AllowEmptyMessages,
}

#[allow(clippy::too_many_arguments)]
async fn assemble_turn_context_with_mode(
    harness_store: &dyn HarnessStore,
    agent_store: &dyn AgentStore,
    session_store: &dyn SessionStore,
    message_retriever: &dyn MessageRetriever,
    provider_store: &dyn LlmProviderStore,
    capability_registry: &CapabilityRegistry,
    session_id: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    mcp_tool_definitions: &[ToolDefinition],
    file_store: Option<Arc<dyn SessionFileStore>>,
    mode: ContextAssemblyMode,
) -> Result<AssembledTurnContext> {
    let harness_chain = harness_store.get_harness_chain(harness_id).await?;
    if harness_chain.is_empty() {
        return Err(AgentLoopError::harness_not_found(harness_id));
    }

    let agent = if let Some(agent_id) = agent_id {
        Some(
            agent_store
                .get_agent(agent_id)
                .await?
                .ok_or_else(|| AgentLoopError::agent_not_found(agent_id))?,
        )
    } else {
        None
    };

    let session = session_store
        .get_session(session_id)
        .await?
        .ok_or_else(|| AgentLoopError::session_not_found(session_id))?;

    let effective_overlay = {
        let mut layers: Vec<AgentConfigOverlay> =
            harness_chain.iter().map(AgentConfigOverlay::from).collect();
        if let Some(ref agent) = agent {
            layers.push(AgentConfigOverlay::from(agent));
        }
        layers.push(AgentConfigOverlay::from(&session));
        AgentConfigOverlay::fold(layers)
    };

    let message_filters = crate::capabilities::collect_message_filters_only(
        &effective_overlay.capabilities,
        capability_registry,
    );
    let mut query = MessageQuery::new(session_id);
    message_filters.apply_message_filters(&mut query);
    let mut messages = message_retriever.load_filtered(query).await?;
    message_filters.apply_post_load_filters(&mut messages);
    if messages.is_empty() && matches!(mode, ContextAssemblyMode::RequireMessages) {
        return Err(AgentLoopError::NoMessages);
    }

    let controls_model_id = messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .and_then(|message| message.controls.as_ref())
        .and_then(|controls| controls.model_id);

    let (model_with_provider, resolved_model_id) = resolve_model_with_provider(
        provider_store,
        controls_model_id,
        effective_overlay.default_model_id,
    )
    .await?;

    let resolved_capability_configs =
        resolve_capability_configs(&effective_overlay.capabilities, capability_registry)
            .unwrap_or_else(|error| {
                tracing::warn!(
                    error = ?error,
                    "failed to resolve capability configs; falling back to overlay capabilities"
                );
                effective_overlay.capabilities.clone()
            });

    let resolved_locale = extract_locale_override(&messages).or_else(|| session.locale.clone());
    let prompt_ctx = SystemPromptContext {
        session_id,
        locale: resolved_locale.clone(),
        file_store,
    };

    let compaction_config = effective_overlay
        .capabilities
        .iter()
        .find(|cap| cap.capability_id() == COMPACTION_CAPABILITY_ID)
        .map(|cap| CompactionConfig::from_json(&cap.config));

    let runtime_agent = build_runtime_agent(
        &session,
        &effective_overlay,
        &resolved_capability_configs,
        capability_registry,
        &prompt_ctx,
        mcp_tool_definitions,
        &model_with_provider,
    )
    .await?;

    Ok(AssembledTurnContext {
        harness_chain,
        agent,
        session,
        effective_overlay,
        resolved_capability_configs,
        messages,
        runtime_agent,
        model_with_provider,
        resolved_model_id,
        resolved_locale,
        compaction_config,
    })
}

async fn build_runtime_agent(
    session: &Session,
    effective_overlay: &AgentConfigOverlay,
    resolved_capability_configs: &[AgentCapabilityConfig],
    capability_registry: &CapabilityRegistry,
    prompt_ctx: &SystemPromptContext,
    mcp_tool_definitions: &[ToolDefinition],
    model_with_provider: &ModelWithProvider,
) -> Result<RuntimeAgent> {
    let mut runtime_agent = if let Some(ref blueprint_id) = session.blueprint_id {
        let blueprint = capability_registry.blueprint(blueprint_id).ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown blueprint: \"{blueprint_id}\". Session has blueprint_id set but blueprint not found in registry."
            )
        })?;

        let blueprint_model = match &blueprint.model {
            crate::capabilities::BlueprintModel::Fixed(model) => model.clone(),
            crate::capabilities::BlueprintModel::Default(model) => session
                .blueprint_config
                .as_ref()
                .and_then(|config| config.get("model"))
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
                .unwrap_or_else(|| model.clone()),
            crate::capabilities::BlueprintModel::Inherit => model_with_provider.model.clone(),
        };

        let mut prompt = blueprint.system_prompt.to_string();
        if let Some(ref config) = session.blueprint_config {
            prompt.push_str(&format!("\n\n<config>\n{}\n</config>", config));
        }

        RuntimeAgentBuilder::new()
            .system_prompt(&prompt)
            .tools(blueprint.tool_definitions())
            .model(&blueprint_model)
            .max_iterations(blueprint.max_turns.unwrap_or(20))
            .with_locale(prompt_ctx.locale.as_deref())
            .build()
    } else {
        let mut overlay_for_builder = effective_overlay.clone();
        let overlay_tools = std::mem::take(&mut overlay_for_builder.tools);

        RuntimeAgentBuilder::from_overlay(overlay_for_builder, capability_registry, prompt_ctx)
            .await
            .with_capability_configs(resolved_capability_configs, capability_registry, prompt_ctx)
            .await
            .with_locale(prompt_ctx.locale.as_deref())
            .tools(mcp_tool_definitions.iter().cloned())
            .tools(overlay_tools)
            .model(&model_with_provider.model)
            .build()
    };

    if crate::progress_reporting::session_uses_report_progress(&session.tags) {
        runtime_agent = crate::progress_reporting::apply_report_progress_mode(runtime_agent);
    }

    Ok(runtime_agent)
}

async fn resolve_model_with_provider(
    provider_store: &dyn LlmProviderStore,
    controls_model_id: Option<ModelId>,
    overlay_model_id: Option<ModelId>,
) -> Result<(ModelWithProvider, Option<ModelId>)> {
    for model_id in [controls_model_id, overlay_model_id].into_iter().flatten() {
        if let Some(model_with_provider) = provider_store.get_model_with_provider(model_id).await? {
            return Ok((model_with_provider, Some(model_id)));
        }
    }

    let model = provider_store.get_default_model().await?.ok_or_else(|| {
        AgentLoopError::llm(
            "No model configured: no model_id in controls or effective overlay, and no system default model is set",
        )
    })?;
    Ok((model, None))
}

fn extract_locale_override(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .and_then(|message| {
            message
                .controls
                .as_ref()
                .and_then(|controls| controls.locale.as_deref())
                .or_else(|| {
                    message
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.get("locale"))
                        .and_then(|value| value.as_str())
                })
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Agent, AgentStatus};
    use crate::capabilities::{CapabilityRegistry, TestMathCapability};
    use crate::harness::{Harness, HarnessStatus};
    use crate::memory::{
        InMemoryAgentStore, InMemoryHarnessStore, InMemoryLlmProviderStore,
        InMemoryMessageRetriever,
    };
    use crate::message::Controls;
    use crate::message_retriever::InputMessage;
    use crate::session::{Session, SessionStatus};
    use crate::typed_id::{AgentId, HarnessId};
    use chrono::Utc;
    use uuid::Uuid;

    fn harness(harness_id: HarnessId) -> Harness {
        Harness {
            id: harness_id,
            name: "math".into(),
            display_name: Some("Math".into()),
            description: None,
            system_prompt: "You are a math harness.".into(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("test_math")],
            initial_files: vec![],
            network_access: None,
            is_built_in: false,
            status: HarnessStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
        }
    }

    fn agent(agent_id: AgentId) -> Agent {
        Agent {
            public_id: agent_id,
            internal_id: Uuid::nil(),
            name: "math-agent".into(),
            display_name: Some("Math Agent".into()),
            description: None,
            system_prompt: "Use tools.".into(),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            max_iterations: Some(8),
            tools: vec![],
            status: AgentStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        }
    }

    fn session(session_id: SessionId, harness_id: HarnessId, agent_id: AgentId) -> Session {
        Session {
            id: session_id,
            organization_id: crate::DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id,
            agent_id: Some(agent_id),
            agent_identity_id: None,
            title: Some("ctx".into()),
            locale: Some("en-US".into()),
            preview: None,
            output_preview: None,
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
            status: SessionStatus::Started,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
            active_schedule_count: None,
            features: vec![],
            parent_session_id: None,
            subagent_name: None,
            subagent_task: None,
            subagent_status: None,
            blueprint_id: None,
            blueprint_config: None,
        }
    }

    #[tokio::test]
    async fn assembled_turn_context_builds_runtime_agent_and_messages() {
        let harness_id = "harness_00000000000000000000000000000081".parse().unwrap();
        let agent_id = "agent_00000000000000000000000000000081".parse().unwrap();
        let session_id = "session_00000000000000000000000000000081".parse().unwrap();

        let harness_store = InMemoryHarnessStore::new();
        harness_store.add_harness(harness(harness_id)).await;
        let agent_store = InMemoryAgentStore::new();
        agent_store.add_agent(agent(agent_id)).await;
        let session_store = crate::memory::InMemorySessionStore::new();
        session_store
            .add_session(session(session_id, harness_id, agent_id))
            .await;
        let message_store = InMemoryMessageRetriever::new();
        let mut input = InputMessage::user("What is 2 * 3?");
        input.controls = Some(Controls {
            model_id: None,
            reasoning: None,
            locale: Some("fr-FR".into()),
            hints: None,
        });
        message_store.add(session_id, input).await.unwrap();

        let provider_store = InMemoryLlmProviderStore::new();
        provider_store
            .set_default_model(ModelWithProvider {
                model: "llmsim-model".into(),
                provider_type: crate::llm_models::LlmProviderType::LlmSim,
                api_key: Some("fake-key".into()),
                base_url: None,
            })
            .await;

        let mut capability_registry = CapabilityRegistry::new();
        capability_registry.register(TestMathCapability);

        let assembled = assemble_turn_context(
            &harness_store,
            &agent_store,
            &session_store,
            &message_store,
            &provider_store,
            &capability_registry,
            session_id,
            harness_id,
            Some(agent_id),
            &[],
            None,
        )
        .await
        .unwrap();

        assert_eq!(assembled.messages.len(), 1);
        assert_eq!(assembled.resolved_locale.as_deref(), Some("fr-FR"));
        assert_eq!(assembled.runtime_agent.model, "llmsim-model");
        assert!(
            assembled
                .runtime_agent
                .tools
                .iter()
                .any(|tool| tool.name() == "multiply")
        );
    }

    #[tokio::test]
    async fn inspect_turn_context_allows_empty_message_history() {
        let harness_id = "harness_00000000000000000000000000000082".parse().unwrap();
        let agent_id = "agent_00000000000000000000000000000082".parse().unwrap();
        let session_id = "session_00000000000000000000000000000082".parse().unwrap();

        let harness_store = InMemoryHarnessStore::new();
        harness_store.add_harness(harness(harness_id)).await;
        let agent_store = InMemoryAgentStore::new();
        agent_store.add_agent(agent(agent_id)).await;
        let session_store = crate::memory::InMemorySessionStore::new();
        session_store
            .add_session(session(session_id, harness_id, agent_id))
            .await;
        let message_store = InMemoryMessageRetriever::new();

        let provider_store = InMemoryLlmProviderStore::new();
        provider_store
            .set_default_model(ModelWithProvider {
                model: "llmsim-model".into(),
                provider_type: crate::llm_models::LlmProviderType::LlmSim,
                api_key: Some("fake-key".into()),
                base_url: None,
            })
            .await;

        let mut capability_registry = CapabilityRegistry::new();
        capability_registry.register(TestMathCapability);

        let assembled = inspect_turn_context(
            &harness_store,
            &agent_store,
            &session_store,
            &message_store,
            &provider_store,
            &capability_registry,
            session_id,
            harness_id,
            Some(agent_id),
            &[],
            None,
        )
        .await
        .unwrap();

        assert!(assembled.messages.is_empty());
        assert_eq!(assembled.resolved_locale.as_deref(), Some("en-US"));
        assert_eq!(assembled.runtime_agent.model, "llmsim-model");
    }
}
