//! Pure turn-context transformations over host-resolved execution inputs.
//!
//! Store access, lifecycle validation, model lookup, provider configuration,
//! and driver creation belong to `everruns-host`. The kernel receives the
//! neutral snapshot, already-filtered messages, a credential-free model spec,
//! and an opaque ready driver.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::agent_definition::AgentDefinition;
use crate::capabilities::{CapabilityRegistry, SystemPromptContext, resolve_capability_configs};
use crate::compaction_policy::CompactionPolicy;
use crate::config_layer::AgentConfigOverlay;
use crate::driver_registry::ChatDriver;
use crate::error::Result;
use crate::events::TokenUsage;
use crate::harness_definition::HarnessDefinition;
use crate::message::{Message, MessageRole};
use crate::provider::DriverId;
use crate::runtime_agent::{RuntimeAgent, RuntimeAgentBuilder};
use crate::session::ExecutionSession;
use crate::session_files::SessionFileSystem;
use crate::tool_types::ToolDefinition;
use crate::typed_id::{ModelId, SessionId};
use crate::{AgentCapabilityConfig, ResolvedExecutionSnapshot};

/// Narrow host seam used by callers that cannot preassemble a context before
/// invoking a reason atom. Implementations live outside the kernel.
#[async_trait::async_trait]
pub trait TurnContextResolver: Send + Sync {
    async fn resolve_turn_context(
        &self,
        request: TurnContextRequest,
    ) -> Result<AssembledTurnContext>;
}

#[derive(Debug, Clone)]
pub struct TurnContextRequest {
    /// Session being executed.
    pub session_id: SessionId,
    /// Harness expected by the scheduled turn.
    pub harness_id: crate::HarnessId,
    /// Agent expected by the scheduled turn, when any.
    pub agent_id: Option<crate::AgentId>,
    /// Host-discovered MCP tools available for this turn.
    pub mcp_tool_definitions: Vec<ToolDefinition>,
}

/// Credential-safe provider input prepared by a runtime host.
///
/// The model and provider identities are safe values. Authentication and
/// endpoint configuration are captured only inside the opaque driver.
#[derive(Clone)]
pub struct ResolvedModelExecution {
    /// Provider model name.
    pub model: String,
    /// Open provider account identity.
    pub provider: crate::ProviderKey,
    /// Registered driver integration kind.
    pub provider_type: DriverId,
    /// Ready provider driver. Credential-bearing construction state stays
    /// opaque and is redacted from Debug output.
    pub driver: Arc<dyn ChatDriver>,
}

impl std::fmt::Debug for ResolvedModelExecution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedModelExecution")
            .field("model", &self.model)
            .field("provider", &self.provider)
            .field("provider_type", &self.provider_type)
            .field("driver", &"<opaque>")
            .finish()
    }
}

/// Host-resolved values consumed by the pure context assembler.
#[derive(Debug, Clone)]
pub struct ResolvedTurnContextInput {
    /// Effective, secret-free execution snapshot.
    pub snapshot: ResolvedExecutionSnapshot,
    /// Already-filtered model-visible history.
    pub messages: Vec<Message>,
    /// Highest canonical history sequence represented by `messages`.
    pub message_source_sequence: Option<i64>,
    /// Credential-safe model identity and opaque ready driver.
    pub model: ResolvedModelExecution,
    /// Configured model ID selected for this turn, when any.
    pub resolved_model_id: Option<ModelId>,
    /// Host-discovered MCP tool definitions.
    pub mcp_tool_definitions: Vec<ToolDefinition>,
}

/// Credential-safe context consumed by kernel reason execution.
#[derive(Debug, Clone)]
pub struct AssembledTurnContext {
    /// Effective, secret-free execution snapshot.
    pub snapshot: ResolvedExecutionSnapshot,
    /// Capability configurations after dependency expansion.
    pub resolved_capability_configs: Vec<AgentCapabilityConfig>,
    /// Filtered conversation history visible to the model.
    pub messages: Vec<Message>,
    /// Highest canonical history sequence represented by `messages`.
    pub message_source_sequence: Option<i64>,
    /// Fully assembled runtime agent for this turn.
    pub runtime_agent: RuntimeAgent,
    /// Credential-safe model identity and opaque ready driver.
    pub model: ResolvedModelExecution,
    /// Configured model ID selected for this turn, when any.
    pub resolved_model_id: Option<ModelId>,
    /// Locale selected from message controls or snapshot defaults.
    pub resolved_locale: Option<String>,
    /// Capability-owned compaction policy, when configured.
    pub compaction_policy: Option<Arc<dyn CompactionPolicy>>,
    /// Effective embedder metadata folded by the host loading seam.
    pub embedder_metadata: BTreeMap<String, String>,
}

impl AssembledTurnContext {
    /// Session correlation ID without exposing a session record.
    pub fn session_id(&self) -> SessionId {
        self.snapshot.session_id
    }

    /// Cumulative usage projected into the execution snapshot.
    pub fn cumulative_usage(&self) -> Option<TokenUsage> {
        self.snapshot.cumulative_usage.clone()
    }
}

/// Capability resolution over a neutral execution snapshot.
#[derive(Debug, Clone)]
pub struct ResolvedRuntimeCapabilities {
    /// Effective configuration overlay.
    pub effective_overlay: AgentConfigOverlay,
    /// Capability configurations after dependency expansion.
    pub resolved_capability_configs: Vec<AgentCapabilityConfig>,
}

/// Build a kernel context from values already resolved by a host.
pub async fn assemble_resolved_turn_context(
    input: ResolvedTurnContextInput,
    capability_registry: &CapabilityRegistry,
    file_store: Option<Arc<dyn SessionFileSystem>>,
) -> Result<AssembledTurnContext> {
    let ResolvedTurnContextInput {
        snapshot,
        messages,
        message_source_sequence,
        model,
        resolved_model_id,
        mcp_tool_definitions,
    } = input;

    let ResolvedRuntimeCapabilities {
        effective_overlay,
        resolved_capability_configs,
    } = resolve_snapshot_capabilities(&snapshot, capability_registry);

    let resolved_locale = extract_locale_override(&messages).or_else(|| snapshot.locale.clone());
    let file_store =
        file_store.map(|fs| crate::mount_fs::scoped_prompt_file_store(fs, snapshot.workspace_id));
    let prompt_ctx = SystemPromptContext {
        session_id: snapshot.session_id,
        locale: resolved_locale.clone(),
        file_store,
        model: Some(model.model.clone()),
    };
    let compaction_policy = effective_overlay.capabilities.iter().find_map(|config| {
        capability_registry
            .get(config.capability_id())?
            .compaction_policy(config.config_value())
    });
    let runtime_agent = build_runtime_agent(
        &snapshot,
        effective_overlay,
        capability_registry,
        &prompt_ctx,
        &mcp_tool_definitions,
        &model.model,
    )
    .await?;
    let embedder_metadata = snapshot.embedder_metadata.clone();

    Ok(AssembledTurnContext {
        snapshot,
        resolved_capability_configs,
        messages,
        message_source_sequence,
        runtime_agent,
        model,
        resolved_model_id,
        resolved_locale,
        compaction_policy,
        embedder_metadata,
    })
}

/// Resolve capabilities and reconstruct the effective overlay represented by
/// the already-folded snapshot.
pub fn resolve_snapshot_capabilities(
    snapshot: &ResolvedExecutionSnapshot,
    capability_registry: &CapabilityRegistry,
) -> ResolvedRuntimeCapabilities {
    let effective_overlay = AgentConfigOverlay {
        system_prompt: snapshot.instructions.clone(),
        capabilities: snapshot.capabilities.clone(),
        initial_files: snapshot.initial_files.clone(),
        network_access: snapshot.network_access.clone(),
        default_model_id: snapshot.default_model_id,
        tools: snapshot.tools.clone(),
        max_iterations: snapshot.max_iterations,
        parallel_tool_calls: snapshot.parallel_tool_calls,
        mcp_servers: Default::default(),
    };
    let resolved_capability_configs =
        resolve_capability_configs(&effective_overlay.capabilities, capability_registry)
            .unwrap_or_else(|error| {
                tracing::warn!(
                    error = ?error,
                    "failed to resolve capability configs; falling back to snapshot capabilities"
                );
                effective_overlay.capabilities.clone()
            });
    ResolvedRuntimeCapabilities {
        effective_overlay,
        resolved_capability_configs,
    }
}

/// Pure overlay/capability transformation retained for callers that are
/// projecting loaded definitions at their own host boundary.
pub fn resolve_runtime_capabilities(
    harness: &HarnessDefinition,
    agent: Option<&AgentDefinition>,
    session: &ExecutionSession,
    capability_registry: &CapabilityRegistry,
) -> ResolvedRuntimeCapabilities {
    let effective_overlay = AgentConfigOverlay::fold(
        [AgentConfigOverlay::from(harness)]
            .into_iter()
            .chain(agent.into_iter().map(AgentConfigOverlay::from))
            .chain([AgentConfigOverlay::from(session)]),
    );
    let resolved_capability_configs =
        resolve_capability_configs(&effective_overlay.capabilities, capability_registry)
            .unwrap_or_else(|error| {
                tracing::warn!(error = ?error, "failed to resolve capability configs");
                effective_overlay.capabilities.clone()
            });
    ResolvedRuntimeCapabilities {
        effective_overlay,
        resolved_capability_configs,
    }
}

async fn build_runtime_agent(
    snapshot: &ResolvedExecutionSnapshot,
    mut effective_overlay: AgentConfigOverlay,
    capability_registry: &CapabilityRegistry,
    prompt_ctx: &SystemPromptContext,
    mcp_tool_definitions: &[ToolDefinition],
    model: &str,
) -> Result<RuntimeAgent> {
    let mut runtime_agent = if let Some(ref blueprint_id) = snapshot.blueprint_id {
        let blueprint = capability_registry.blueprint(blueprint_id).ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown blueprint: \"{blueprint_id}\". Snapshot references a blueprint absent from the registry."
            )
        })?;
        let blueprint_model = match &blueprint.model {
            crate::capabilities::BlueprintModel::Fixed(model) => model.clone(),
            crate::capabilities::BlueprintModel::Default(default) => snapshot
                .blueprint_config
                .as_ref()
                .and_then(|config| config.get("model"))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| default.clone()),
            crate::capabilities::BlueprintModel::Inherit => model.to_owned(),
        };
        let mut prompt = blueprint.system_prompt.to_string();
        if let Some(ref config) = snapshot.blueprint_config {
            prompt.push_str(&format!("\n\n<config>\n{}\n</config>", config));
        }
        RuntimeAgentBuilder::new()
            .system_prompt(&prompt)
            .tools(blueprint.tool_definitions())
            .model(&blueprint_model)
            .max_iterations(blueprint.max_turns.unwrap_or(20))
            .network_access(effective_overlay.network_access.clone())
            .with_locale(prompt_ctx.locale.as_deref())
            .build()
    } else {
        let overlay_tools = std::mem::take(&mut effective_overlay.tools);
        RuntimeAgentBuilder::from_overlay(effective_overlay, capability_registry, prompt_ctx)
            .await
            .with_locale(prompt_ctx.locale.as_deref())
            .tools(mcp_tool_definitions.iter().cloned())
            .tools(overlay_tools)
            .model(model)
            .build()
    };
    if crate::progress_reporting::session_uses_report_progress(&snapshot.tags) {
        runtime_agent = crate::progress_reporting::apply_report_progress_mode(runtime_agent);
    }
    Ok(runtime_agent)
}

fn extract_locale_override(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .and_then(|message| message.controls.as_ref())
        .and_then(|controls| controls.locale.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CredentialCapturingDriver {
        _secret: String,
    }

    #[async_trait::async_trait]
    impl ChatDriver for CredentialCapturingDriver {
        async fn chat_completion_stream(
            &self,
            _endpoint: &everruns_provider::ProviderEndpoint,
            _messages: Vec<crate::LlmMessage>,
            _config: &crate::LlmCallConfig,
        ) -> crate::Result<crate::LlmResponseStream> {
            unreachable!("debug-surface test never invokes the driver")
        }
    }

    #[test]
    fn resolved_model_execution_debug_is_credential_safe() {
        let secret = "credential-marker-that-must-not-leak";
        let resolved = ResolvedModelExecution {
            model: "model-name".into(),
            provider: crate::ProviderKey::new("provider-account"),
            provider_type: DriverId::OpenAI,
            driver: Arc::new(CredentialCapturingDriver {
                _secret: secret.into(),
            }),
        };

        let debug = format!("{resolved:?}");
        assert!(debug.contains("provider-account"));
        assert!(debug.contains("<opaque>"));
        assert!(!debug.contains(secret));
    }
}
