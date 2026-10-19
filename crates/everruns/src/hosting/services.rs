//! Host services composed into an Engine-owned runtime (EVE-1296).
//!
//! An embedding host that already owns a curated set of capability
//! implementations, provider resolution, provider recovery budgets, MCP
//! credentials and event observation can hand them to the Framework without
//! constructing a runtime itself. [`HostServices`] is a validated, immutable
//! value attached to an Agent with
//! [`AgentBuilder::host_services`](crate::AgentBuilder::host_services). Engine
//! applies it every time it constructs that Agent's runtime: for frontend
//! sessions, every coherent activation, context queries and resumption, so a
//! host composition cannot be lost by rebuilding a session.
//!
//! What the services may contribute:
//!
//! - capability implementations written against the Core [`Capability`]
//!   contract, including [`PreToolUseHook`] guards, [`ModelViewProvider`]
//!   model views, slash-command contributions and capability instructions;
//! - provider resolution through a host [`ProviderStore`] and
//!   [`DriverRegistry`] ([`HostProviders`]);
//! - provider retry and stream-stall budgets;
//! - call-time MCP credentials (`mcp` feature);
//! - credential-free provider-request metadata and OpenRouter attribution
//!   ([`RequestMetadata`]), applied to every provider request the Agent's
//!   sessions make.
//!
//! Engine-level host observers are registered with
//! [`EngineBuilder::host_listener`](crate::EngineBuilder::host_listener).
//!
//! What they may not do, by construction:
//!
//! - replace Engine session authority, leases, admission or the turn runner:
//!   there is no runtime-builder callback and no runtime handle to drive;
//! - replace the Environment-selected session filesystem, workspace
//!   containment or egress transport: capability tools reach files only
//!   through the session filesystem in their [`ToolContext`];
//! - shadow a Framework capability, alias or tool: collisions are
//!   [`BuildError`](crate::BuildError)s, never registry overwrites;
//! - fall back to a provider the host did not configure.
//!
//! Stability: experimental, alongside [`super::SessionHosting`].
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//! use everruns::hosting::services::{
//!     Capability, HostServices, PolicyGate, PreToolUseDecision, PreToolUseHook, ToolCall,
//!     ToolContext, ToolDefinition, async_trait,
//! };
//! use everruns::{Agent, Model};
//!
//! struct NoDeletes;
//!
//! #[async_trait]
//! impl PreToolUseHook for NoDeletes {
//!     async fn before_exec(
//!         &self,
//!         tool_call: ToolCall,
//!         _definition: &ToolDefinition,
//!         _context: &ToolContext,
//!     ) -> PreToolUseDecision {
//!         if tool_call.name == "delete_file" {
//!             return PreToolUseDecision::Block {
//!                 tool_call,
//!                 reason: "deletes are disabled by host policy".into(),
//!                 user_message: None,
//!             };
//!         }
//!         PreToolUseDecision::Continue(tool_call)
//!     }
//! }
//!
//! struct Policy;
//!
//! impl Capability for Policy {
//!     fn id(&self) -> &str { "host_policy" }
//!     fn name(&self) -> &str { "Host policy" }
//!     fn description(&self) -> &str { "Blocks destructive tools." }
//!     fn system_prompt_addition(&self) -> Option<&str> {
//!         Some("Never delete files.")
//!     }
//!     fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn PreToolUseHook>> {
//!         vec![Arc::new(PolicyGate(NoDeletes))]
//!     }
//! }
//!
//! let agent = Agent::builder()
//!     .instructions("You edit files carefully.")
//!     .model(Model::simulated("done"))
//!     .host_services(HostServices::new().capability(Policy))
//!     .build()?;
//! # let _ = agent;
//! # Ok::<(), everruns::BuildError>(())
//! ```

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait as derive_async_trait;
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::runtime_provider::ProviderKey;
use everruns_contracts::typed_id::{ModelId, SessionId};
use serde_json::Value;

pub use super::request_metadata::{
    MAX_REQUEST_METADATA_ENTRIES, OPENROUTER_HTTP_REFERER_KEY, OPENROUTER_X_TITLE_KEY,
    RequestMetadata,
};
pub use async_trait::async_trait;
pub use everruns_contracts::driver_registry::{DriverRegistry, ProviderConfig, ProviderMetadata};
pub use everruns_contracts::error::{AgentLoopError, Result};
pub use everruns_contracts::llm_retry::LlmRetryConfig;
pub use everruns_contracts::model_spec::ModelSpec as HostModelSpec;
pub use everruns_contracts::runtime_provider::ProviderKey as HostProviderKey;
pub use everruns_contracts::tool_types::{ToolCall, ToolDefinition, ToolResult};
pub use everruns_contracts::typed_id::ModelId as HostModelId;
pub use everruns_core::capabilities::{
    Capability, CapabilityStatus, ModelViewContext, ModelViewProvider, RiskLevel,
    SystemPromptContext, ToolCallHook, ToolDefinitionHook,
};
pub use everruns_core::command::{
    CommandArg, CommandDescriptor, CommandExecutionContext, CommandResult, CommandSource,
    ExecuteCommandRequest,
};
pub use everruns_core::events::Event as HostEvent;
#[cfg(feature = "mcp")]
pub use everruns_core::mcp::{McpAuthProvider, McpAuthRequest, McpCredential};
pub use everruns_core::provider_resolution::ProviderStore;
pub use everruns_core::tool_context::ToolContext;
pub use everruns_core::tool_hooks::{
    PolicyGate, PostToolExecHook, PostToolExecHookPriority, PreToolUseDecision, PreToolUseHook,
};
pub use everruns_core::tools::{Tool, ToolExecutionResult};
pub use everruns_core::{EventListener as HostEventListener, RuntimeMessage, RuntimeMessageRole};
pub use everruns_core::{SessionFile, SessionFileSystem};

/// Error type of [`McpAuthProvider::authorization`]; build one with
/// `McpAuthError::msg(..)` or convert any standard error with `?`.
#[cfg(feature = "mcp")]
pub type McpAuthError = anyhow::Error;

/// Result type of [`McpAuthProvider::authorization`].
#[cfg(feature = "mcp")]
pub type McpAuthResult<T> = std::result::Result<T, McpAuthError>;

/// Host capability, provider and recovery services for an Agent's runtimes.
///
/// Build it once and attach it to every Agent that should run with the host's
/// composition. Values are cheap to clone and are shared, not copied, by each
/// runtime Engine constructs. See the [module documentation](self).
#[derive(Clone, Default)]
pub struct HostServices {
    pub(crate) capabilities: Vec<HostCapability>,
    pub(crate) providers: Option<HostProviders>,
    pub(crate) retry: Option<LlmRetryConfig>,
    pub(crate) stall_timeout: Option<Duration>,
    pub(crate) request_metadata: RequestMetadata,
    #[cfg(feature = "mcp")]
    pub(crate) mcp_auth: Option<Arc<dyn McpAuthProvider>>,
}

#[derive(Clone)]
pub(crate) struct HostCapability {
    pub(crate) implementation: Arc<dyn Capability>,
    /// Activation config, or `None` when the capability is only registered.
    pub(crate) activation: Option<Value>,
}

impl HostServices {
    /// Empty services; the Framework defaults apply to everything left unset.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register and activate a host capability with an empty configuration.
    pub fn capability(self, capability: impl Capability + 'static) -> Self {
        self.capability_with_config(capability, Value::Object(Default::default()))
    }

    /// Register and activate a host capability with a JSON object configuration.
    ///
    /// The capability's own `validate_config` runs when the Agent is built.
    pub fn capability_with_config(
        self,
        capability: impl Capability + 'static,
        config: Value,
    ) -> Self {
        self.shared_capability_with_config(Arc::new(capability), config)
    }

    /// Register and activate a capability value the host shares with other
    /// Agents or runtimes.
    pub fn shared_capability(self, capability: Arc<dyn Capability>) -> Self {
        self.shared_capability_with_config(capability, Value::Object(Default::default()))
    }

    /// [`shared_capability`](Self::shared_capability) with a JSON object configuration.
    pub fn shared_capability_with_config(
        mut self,
        capability: Arc<dyn Capability>,
        config: Value,
    ) -> Self {
        self.capabilities.push(HostCapability {
            implementation: capability,
            activation: Some(config),
        });
        self
    }

    /// Register a capability without activating it.
    ///
    /// The Agent activates it with an ordinary reference such as
    /// `.capability("my_capability")` or through its Harness, and the same
    /// implementation serves that activation.
    pub fn register(mut self, capability: impl Capability + 'static) -> Self {
        self.capabilities.push(HostCapability {
            implementation: Arc::new(capability),
            activation: None,
        });
        self
    }

    /// Resolve the Agent's provider through the host instead of an Agent-owned
    /// [`Provider`](crate::Provider).
    ///
    /// The Agent then selects its model with [`Model::on`](crate::Model::on)
    /// and must not also carry a provider.
    pub fn providers(mut self, providers: HostProviders) -> Self {
        self.providers = Some(providers);
        self
    }

    /// Override the bounded provider retry policy.
    pub fn provider_retry(mut self, config: LlmRetryConfig) -> Self {
        self.retry = Some(config);
        self
    }

    /// Override how long a provider stream may produce no output before the
    /// call is treated as stalled.
    pub fn provider_stall_timeout(mut self, timeout: Duration) -> Self {
        self.stall_timeout = Some(timeout);
        self
    }

    /// Attach credential-free metadata to every provider request.
    ///
    /// Replaces earlier metadata. Engine seeds it into every runtime it builds
    /// for the Agent, so turn model calls, out-of-band completions, context
    /// queries and rebuilt or resumed sessions all carry it. Entries are
    /// validated when the Agent is built; see [`RequestMetadata`].
    pub fn request_metadata(mut self, metadata: RequestMetadata) -> Self {
        self.request_metadata = metadata;
        self
    }

    /// The provider-request metadata these services attach.
    pub fn request_metadata_entries(&self) -> &RequestMetadata {
        &self.request_metadata
    }

    /// Acquire scoped MCP server credentials at call time.
    ///
    /// The provider is asked before every discovery and tool call, so a host
    /// that refreshes tokens serves the current one to each request.
    #[cfg(feature = "mcp")]
    pub fn mcp_auth(mut self, provider: impl McpAuthProvider + 'static) -> Self {
        self.mcp_auth = Some(Arc::new(provider));
        self
    }

    /// [`mcp_auth`](Self::mcp_auth) for a provider the host shares.
    #[cfg(feature = "mcp")]
    pub fn shared_mcp_auth(mut self, provider: Arc<dyn McpAuthProvider>) -> Self {
        self.mcp_auth = Some(provider);
        self
    }

    /// Ids of the registered host capabilities, in registration order.
    pub fn capability_ids(&self) -> Vec<String> {
        self.capabilities
            .iter()
            .map(|capability| capability.implementation.id().to_string())
            .collect()
    }

    /// Whether `id` names one of these host capabilities or an alias of one.
    pub(crate) fn resolves(&self, id: &str) -> Option<&str> {
        self.capabilities.iter().find_map(|capability| {
            let implementation = capability.implementation.as_ref();
            (implementation.id() == id || implementation.aliases().contains(&id))
                .then(|| implementation.id())
        })
    }

    /// Run a registered host capability's own config validation for an Agent
    /// reference that activates it.
    pub(crate) fn validate_reference(
        &self,
        id: &str,
        config: &Value,
    ) -> std::result::Result<(), crate::BuildError> {
        let Some(capability) = self
            .capabilities
            .iter()
            .find(|capability| capability.implementation.id() == id)
        else {
            return Ok(());
        };
        capability
            .implementation
            .validate_config(config)
            .map_err(|reason| crate::BuildError::InvalidCapability {
                id: id.to_string(),
                reason,
            })
    }

    /// Check identities, configs and tool names before an Agent is built.
    pub(crate) fn validate(
        &self,
        framework: &everruns_core::CapabilityRegistry,
        seen_tool_names: &mut HashSet<String>,
    ) -> std::result::Result<(), crate::BuildError> {
        self.request_metadata.validate()?;
        let mut seen = HashSet::new();
        for capability in &self.capabilities {
            let implementation = capability.implementation.as_ref();
            let id = implementation.id().to_string();
            everruns_contracts::validate_capability_id(&id).map_err(|error| {
                crate::BuildError::InvalidCapability {
                    id: id.clone(),
                    reason: error.reason(),
                }
            })?;
            // THREAT[TM-FS-021]: a host capability cannot shadow a Framework
            // implementation (for example the Environment-backed filesystem)
            // under its id or any alias.
            for name in std::iter::once(id.as_str()).chain(implementation.aliases()) {
                if framework.get(name).is_some() || !seen.insert(name.to_string()) {
                    return Err(crate::BuildError::DuplicateCapability {
                        id: name.to_string(),
                    });
                }
            }
            if let Some(config) = &capability.activation {
                everruns_contracts::validate_capability_config(&id, config).map_err(|error| {
                    crate::BuildError::InvalidCapability {
                        id: id.clone(),
                        reason: error.reason(),
                    }
                })?;
                implementation.validate_config(config).map_err(|reason| {
                    crate::BuildError::InvalidCapability {
                        id: id.clone(),
                        reason,
                    }
                })?;
            }
            for tool in implementation.tools() {
                let name = tool.name().to_string();
                crate::tool::validate_tool_name(&name).map_err(|reason| {
                    crate::BuildError::InvalidToolName {
                        name: name.clone(),
                        reason,
                    }
                })?;
                if !seen_tool_names.insert(name.clone()) {
                    return Err(crate::BuildError::DuplicateTool { name });
                }
            }
        }
        Ok(())
    }
}

impl fmt::Debug for HostServices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("HostServices");
        debug
            .field("capabilities", &self.capability_ids())
            .field("providers", &self.providers.is_some())
            .field("retry", &self.retry)
            .field("stall_timeout", &self.stall_timeout)
            .field("request_metadata", &self.request_metadata);
        #[cfg(feature = "mcp")]
        debug.field("mcp_auth", &self.mcp_auth.is_some());
        debug.finish()
    }
}

/// Host-owned provider resolution.
///
/// The [`ProviderStore`] owns endpoint and credential material and is asked
/// under the session's identity on every provider call, so rotated
/// credentials, endpoint metadata and live model selection reach the next call
/// without rebuilding the runtime. The [`DriverRegistry`] turns a resolved
/// [`ProviderConfig`] into a driver; providers registered on it directly are
/// served as they are.
///
/// A provider key neither of them knows is a configuration error. The
/// Framework never substitutes another provider or a default driver.
#[derive(Clone)]
pub struct HostProviders {
    pub(crate) store: Arc<dyn ProviderStore>,
    pub(crate) drivers: DriverRegistry,
}

impl HostProviders {
    /// Resolve providers through `store`, constructing drivers from `drivers`.
    pub fn new(store: impl ProviderStore + 'static, drivers: DriverRegistry) -> Self {
        Self::shared(Arc::new(store), drivers)
    }

    /// [`new`](Self::new) for a store the host shares.
    pub fn shared(store: Arc<dyn ProviderStore>, drivers: DriverRegistry) -> Self {
        Self { store, drivers }
    }

    /// Fail before execution when nothing can serve `provider`.
    pub(crate) async fn ensure_serves(&self, provider: &ProviderKey) -> Result<()> {
        if self.drivers.provider(provider).is_some()
            || self.store.get_provider_config(provider).await?.is_some()
        {
            return Ok(());
        }
        Err(unconfigured(provider))
    }
}

impl fmt::Debug for HostProviders {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostProviders")
            .field("direct_providers", &self.drivers.registered_provider_ids())
            .field("drivers", &self.drivers.registered_providers().len())
            .finish()
    }
}

fn unconfigured(provider: &ProviderKey) -> AgentLoopError {
    AgentLoopError::config(format!(
        "host provider {provider} is not configured; refusing to fall back to another provider"
    ))
}

/// Per-runtime provider store: the Agent's model is the runtime default, and
/// everything else is the host's.
pub(crate) struct HostProviderStore {
    providers: HostProviders,
    default_model: tokio::sync::RwLock<Option<ModelSpec>>,
}

impl HostProviderStore {
    pub(crate) fn new(providers: HostProviders) -> Self {
        Self {
            providers,
            default_model: tokio::sync::RwLock::new(None),
        }
    }

    fn checked(
        &self,
        provider: &ProviderKey,
        config: Option<ProviderConfig>,
    ) -> Result<Option<ProviderConfig>> {
        match config {
            Some(config) => Ok(Some(config)),
            // A directly registered provider resolves without store material.
            None if self.providers.drivers.provider(provider).is_some() => Ok(None),
            // THREAT[TM-LLM-051]: returning `None` here would let the runtime
            // guess an external driver under the same name, a silent fallback.
            None => Err(unconfigured(provider)),
        }
    }
}

#[derive_async_trait]
impl ProviderStore for HostProviderStore {
    async fn get_model_spec(&self, model_id: ModelId) -> Result<Option<ModelSpec>> {
        self.providers.store.get_model_spec(model_id).await
    }

    async fn get_default_model_spec(&self) -> Result<Option<ModelSpec>> {
        Ok(self.default_model.read().await.clone())
    }

    async fn get_provider_config_for_session(
        &self,
        provider: &ProviderKey,
        session: SessionId,
    ) -> Result<Option<ProviderConfig>> {
        let config = self
            .providers
            .store
            .get_provider_config_for_session(provider, session)
            .await?;
        self.checked(provider, config)
    }

    async fn get_provider_config(&self, provider: &ProviderKey) -> Result<Option<ProviderConfig>> {
        let config = self.providers.store.get_provider_config(provider).await?;
        self.checked(provider, config)
    }
}

#[derive_async_trait]
impl everruns_core::host::RuntimeProviderStore for HostProviderStore {
    async fn set_default_model_spec(&self, model: ModelSpec) -> Result<()> {
        *self.default_model.write().await = Some(model);
        Ok(())
    }
}
