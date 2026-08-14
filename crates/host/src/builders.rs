// Host-facing builders for core seed models.
// Decision: keep these in everruns-host so core domain structs stay literal
// data models while the host owns low-level embedding ergonomics. The runtime
// compatibility crate only re-exports these builders.

use std::collections::HashMap;

use everruns_capability::plugin_capability_id;
use everruns_core::network_access::NetworkAccessList;
use everruns_core::{
    AgentDefinition, DEFAULT_ORG_PUBLIC_ID, ExecutionSession, HarnessDefinition, ScopedMcpServers,
    SessionExecutionState,
};
pub use everruns_provider::driver_registry::{
    OPENROUTER_HTTP_REFERER_METADATA_KEY, OPENROUTER_X_TITLE_METADATA_KEY,
};
use everruns_provider::tool_types::ToolDefinition;
use everruns_provider::typed_id::{AgentId, HarnessId, ModelId, SessionId, WorkspaceId};

/// A portable harness definition seeded under an embedder-chosen id.
///
/// EVE-881: the Framework host carries no stored Harness persistence records.
/// The definition itself is id-free neutral configuration; the id exists only
/// to key the association with sessions (`Session::harness_id`).
#[derive(Debug, Clone)]
pub struct SeededHarness {
    /// Id sessions reference via `harness_id`.
    pub id: HarnessId,
    /// Portable execution configuration seeded under `id`.
    pub definition: HarnessDefinition,
}

/// Builds a portable [`HarnessDefinition`] with runtime-friendly defaults,
/// paired with the id it is seeded under.
///
/// EVE-881: the embedded host seeds neutral execution configuration only.
/// Stored Harness persistence records (lifecycle status, hierarchy, display
/// metadata, timestamps) live in `everruns-platform` and are a hosted
/// control-plane concern.
#[derive(Debug, Clone)]
pub struct HarnessBuilder {
    id: HarnessId,
    name: String,
    system_prompt: String,
    default_model_id: Option<ModelId>,
    capabilities: Vec<everruns_capability::CapabilityRef>,
    initial_files: Vec<everruns_core::InitialFile>,
    network_access: Option<NetworkAccessList>,
    parallel_tool_calls: Option<bool>,
    mcp_servers: ScopedMcpServers,
    embedder_metadata: HashMap<String, String>,
}

impl HarnessBuilder {
    /// Create a harness builder from the required embedder-facing fields.
    pub fn new(name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            id: HarnessId::new(),
            name: name.into(),
            system_prompt: system_prompt.into(),
            default_model_id: None,
            capabilities: Vec::new(),
            initial_files: Vec::new(),
            network_access: None,
            parallel_tool_calls: None,
            mcp_servers: ScopedMcpServers::default(),
            embedder_metadata: HashMap::new(),
        }
    }

    /// Set a stable harness id instead of generating one.
    pub fn id(mut self, id: HarnessId) -> Self {
        self.id = id;
        self
    }

    /// Return the id currently assigned to this builder.
    pub fn harness_id(&self) -> HarnessId {
        self.id
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn system_prompt(mut self, system_prompt: impl Into<String>) -> Self {
        self.system_prompt = system_prompt.into();
        self
    }

    pub fn default_model_id(mut self, default_model_id: ModelId) -> Self {
        self.default_model_id = Some(default_model_id);
        self
    }

    pub fn capability(mut self, capability: impl Into<everruns_capability::CapabilityRef>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    pub fn with_capability(
        self,
        capability: impl Into<everruns_capability::CapabilityRef>,
    ) -> Self {
        self.capability(capability)
    }

    pub fn capabilities<I, C>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<everruns_capability::CapabilityRef>,
    {
        self.capabilities
            .extend(capabilities.into_iter().map(Into::into));
        self
    }

    pub fn initial_file(mut self, file: everruns_core::InitialFile) -> Self {
        self.initial_files.push(file);
        self
    }

    pub fn network_access(mut self, network_access: NetworkAccessList) -> Self {
        self.network_access = Some(network_access);
        self
    }

    /// Set the request-level parallel tool calling preference (EVE-598).
    pub fn parallel_tool_calls(mut self, parallel_tool_calls: bool) -> Self {
        self.parallel_tool_calls = Some(parallel_tool_calls);
        self
    }

    pub fn mcp_servers(mut self, mcp_servers: ScopedMcpServers) -> Self {
        self.mcp_servers = mcp_servers;
        self
    }

    pub fn metadata_entry(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.embedder_metadata.insert(key.into(), value.into());
        self
    }

    pub fn metadata_entries<I, K, V>(mut self, entries: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.embedder_metadata
            .extend(entries.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// Set OpenRouter attribution headers for LLM calls made by this harness.
    ///
    /// The values flow through harness embedder metadata and are sent by the
    /// OpenRouter driver as `HTTP-Referer` and `X-Title`.
    pub fn openrouter_attribution(
        mut self,
        http_referer: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        self.embedder_metadata.insert(
            OPENROUTER_HTTP_REFERER_METADATA_KEY.to_string(),
            http_referer.into(),
        );
        self.embedder_metadata
            .insert(OPENROUTER_X_TITLE_METADATA_KEY.to_string(), title.into());
        self
    }

    /// Build the seeded harness. Builders do not validate domain invariants.
    pub fn build(self) -> SeededHarness {
        SeededHarness {
            id: self.id,
            definition: HarnessDefinition {
                name: self.name,
                // Empty/whitespace-only builder prompt means the harness
                // contributes no base prompt.
                system_prompt: (!self.system_prompt.trim().is_empty())
                    .then_some(self.system_prompt),
                default_model_id: self.default_model_id,
                capabilities: self.capabilities,
                initial_files: self.initial_files,
                network_access: self.network_access,
                parallel_tool_calls: self.parallel_tool_calls,
                mcp_servers: self.mcp_servers,
                embedder_metadata: self.embedder_metadata,
            },
        }
    }
}

/// Builds a portable [`AgentDefinition`] with runtime-friendly defaults.
///
/// EVE-877: the embedded host seeds authored execution configuration only.
/// Stored Agent persistence records (lifecycle status, versioning, timestamps)
/// live in `everruns-platform` and are a hosted-control-plane concern.
#[derive(Debug, Clone)]
pub struct AgentBuilder {
    id: AgentId,
    name: String,
    display_name: Option<String>,
    description: Option<String>,
    system_prompt: String,
    default_model_id: Option<ModelId>,
    capabilities: Vec<everruns_capability::CapabilityRef>,
    initial_files: Vec<everruns_core::InitialFile>,
    network_access: Option<NetworkAccessList>,
    max_iterations: Option<usize>,
    parallel_tool_calls: Option<bool>,
    tools: Vec<ToolDefinition>,
    mcp_servers: ScopedMcpServers,
}

impl AgentBuilder {
    /// Create an agent builder from the required embedder-facing fields.
    pub fn new(name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            id: AgentId::new(),
            name: name.into(),
            display_name: None,
            description: None,
            system_prompt: system_prompt.into(),
            default_model_id: None,
            capabilities: Vec::new(),
            initial_files: Vec::new(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            tools: Vec::new(),
            mcp_servers: ScopedMcpServers::default(),
        }
    }

    /// Set a stable agent id instead of generating one.
    pub fn id(mut self, id: AgentId) -> Self {
        self.id = id;
        self
    }

    /// Return the id currently assigned to this builder.
    pub fn agent_id(&self) -> AgentId {
        self.id
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn system_prompt(mut self, system_prompt: impl Into<String>) -> Self {
        self.system_prompt = system_prompt.into();
        self
    }

    pub fn default_model_id(mut self, default_model_id: ModelId) -> Self {
        self.default_model_id = Some(default_model_id);
        self
    }

    pub fn capability(mut self, capability: impl Into<everruns_capability::CapabilityRef>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    pub fn with_capability(
        self,
        capability: impl Into<everruns_capability::CapabilityRef>,
    ) -> Self {
        self.capability(capability)
    }

    pub fn capabilities<I, C>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<everruns_capability::CapabilityRef>,
    {
        self.capabilities
            .extend(capabilities.into_iter().map(Into::into));
        self
    }

    pub fn initial_file(mut self, file: everruns_core::InitialFile) -> Self {
        self.initial_files.push(file);
        self
    }

    pub fn network_access(mut self, network_access: NetworkAccessList) -> Self {
        self.network_access = Some(network_access);
        self
    }

    pub fn max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = Some(max_iterations);
        self
    }

    /// Set the request-level parallel tool calling preference (EVE-598).
    pub fn parallel_tool_calls(mut self, parallel_tool_calls: bool) -> Self {
        self.parallel_tool_calls = Some(parallel_tool_calls);
        self
    }

    pub fn tool(mut self, tool: ToolDefinition) -> Self {
        self.tools.push(tool);
        self
    }

    pub fn tools<I>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = ToolDefinition>,
    {
        self.tools.extend(tools);
        self
    }

    pub fn mcp_servers(mut self, mcp_servers: ScopedMcpServers) -> Self {
        self.mcp_servers = mcp_servers;
        self
    }

    /// Build the agent definition. Builders do not validate domain invariants.
    pub fn build(self) -> AgentDefinition {
        AgentDefinition {
            id: self.id,
            name: self.name,
            display_name: self.display_name,
            description: self.description,
            system_prompt: self.system_prompt,
            default_model_id: self.default_model_id,
            capabilities: self.capabilities,
            initial_files: self.initial_files,
            network_access: self.network_access,
            max_iterations: self.max_iterations,
            parallel_tool_calls: self.parallel_tool_calls,
            tools: self.tools,
            mcp_servers: self.mcp_servers,
        }
    }
}

/// Builds a portable [`ExecutionSession`] with runtime-friendly defaults.
///
/// EVE-882: the embedded host seeds the neutral execution view only. The
/// stored Session persistence record (source facets, participants, ownership
/// summaries, timestamps, UI metadata) lives in `everruns-platform` and is a
/// hosted control-plane concern.
#[derive(Debug, Clone)]
pub struct SessionBuilder {
    id: SessionId,
    workspace_id: Option<WorkspaceId>,
    organization_id: String,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    title: Option<String>,
    goal: Option<String>,
    locale: Option<String>,
    tags: Vec<String>,
    model_id: Option<ModelId>,
    capabilities: Vec<everruns_capability::CapabilityRef>,
    tools: Vec<ToolDefinition>,
    mcp_servers: ScopedMcpServers,
    system_prompt: Option<String>,
    initial_files: Vec<everruns_core::InitialFile>,
    network_access: Option<NetworkAccessList>,
    max_iterations: Option<usize>,
    parallel_tool_calls: Option<bool>,
    status: SessionExecutionState,
}

impl SessionBuilder {
    /// Create a session builder for the required harness id.
    pub fn new(harness_id: HarnessId) -> Self {
        Self {
            id: SessionId::new(),
            workspace_id: None,
            organization_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id,
            agent_id: None,
            title: None,
            goal: None,
            locale: None,
            tags: Vec::new(),
            model_id: None,
            capabilities: Vec::new(),
            tools: Vec::new(),
            mcp_servers: ScopedMcpServers::default(),
            system_prompt: None,
            initial_files: Vec::new(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            status: SessionExecutionState::Started,
        }
    }

    /// Set a stable session id instead of generating one.
    pub fn id(mut self, id: SessionId) -> Self {
        self.id = id;
        self
    }

    /// Return the id currently assigned to this builder.
    pub fn session_id(&self) -> SessionId {
        self.id
    }

    /// Bind execution filesystem scoping to an explicit logical workspace.
    pub fn workspace(mut self, workspace_id: WorkspaceId) -> Self {
        self.workspace_id = Some(workspace_id);
        self
    }

    pub fn organization_id(mut self, organization_id: impl Into<String>) -> Self {
        self.organization_id = organization_id.into();
        self
    }

    pub fn harness(mut self, harness_id: HarnessId) -> Self {
        self.harness_id = harness_id;
        self
    }

    pub fn agent(mut self, agent_id: AgentId) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn goal(mut self, goal: impl Into<String>) -> Self {
        self.goal = Some(goal.into());
        self
    }

    pub fn locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    pub fn tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }

    pub fn model_id(mut self, model_id: ModelId) -> Self {
        self.model_id = Some(model_id);
        self
    }

    pub fn capability(mut self, capability: impl Into<everruns_capability::CapabilityRef>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    pub fn with_capability(
        self,
        capability: impl Into<everruns_capability::CapabilityRef>,
    ) -> Self {
        self.capability(capability)
    }

    pub fn capabilities<I, C>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<everruns_capability::CapabilityRef>,
    {
        self.capabilities
            .extend(capabilities.into_iter().map(Into::into));
        self
    }

    pub fn tool(mut self, tool: ToolDefinition) -> Self {
        self.tools.push(tool);
        self
    }

    pub fn tools<I>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = ToolDefinition>,
    {
        self.tools.extend(tools);
        self
    }

    pub fn mcp_servers(mut self, mcp_servers: ScopedMcpServers) -> Self {
        self.mcp_servers = mcp_servers;
        self
    }

    pub fn system_prompt(mut self, system_prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(system_prompt.into());
        self
    }

    pub fn initial_file(mut self, file: everruns_core::InitialFile) -> Self {
        self.initial_files.push(file);
        self
    }

    pub fn network_access(mut self, network_access: NetworkAccessList) -> Self {
        self.network_access = Some(network_access);
        self
    }

    pub fn max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = Some(max_iterations);
        self
    }

    /// Set the request-level parallel tool calling preference (EVE-598).
    pub fn parallel_tool_calls(mut self, parallel_tool_calls: bool) -> Self {
        self.parallel_tool_calls = Some(parallel_tool_calls);
        self
    }

    pub fn status(mut self, status: SessionExecutionState) -> Self {
        self.status = status;
        self
    }

    /// Build the session. Builders do not validate domain invariants.
    pub fn build(self) -> ExecutionSession {
        ExecutionSession {
            id: self.id,
            workspace_id: self
                .workspace_id
                .unwrap_or_else(|| WorkspaceId::from_uuid((self.id).uuid())),
            organization_id: self.organization_id,
            harness_id: self.harness_id,
            agent_id: self.agent_id,
            title: self.title,
            goal: self.goal,
            locale: self.locale,
            tags: self.tags,
            model_id: self.model_id,
            capabilities: self.capabilities,
            tools: self.tools,
            mcp_servers: self.mcp_servers,
            system_prompt: self.system_prompt,
            initial_files: self.initial_files,
            hints: None,
            network_access: self.network_access,
            max_iterations: self.max_iterations,
            parallel_tool_calls: self.parallel_tool_calls,
            status: self.status,
            usage: None,
            parent_session_id: None,
            forked_from_session_id: None,
            blueprint_id: None,
            blueprint_config: None,
        }
    }
}

/// High-level builder for seeding one harness, one agent, and one session.
///
/// This is the compact path used by embedders that want a runnable runtime
/// without constructing each core model separately.
#[derive(Debug, Clone)]
pub struct SingleSessionBuilder {
    harness: HarnessBuilder,
    agent: AgentBuilder,
    session: SessionBuilder,
}

impl Default for SingleSessionBuilder {
    fn default() -> Self {
        let harness = HarnessBuilder::new("embedded-harness", "");
        let agent = AgentBuilder::new("embedded-agent", "");
        let session = SessionBuilder::new(harness.harness_id()).agent(agent.agent_id());
        Self {
            harness,
            agent,
            session,
        }
    }
}

impl SingleSessionBuilder {
    /// Configure the seeded harness. Mutates the existing `HarnessBuilder`
    /// in place so previously configured fields (e.g. `network_access`) are
    /// preserved regardless of call order.
    pub fn harness(mut self, name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        let harness_id = self.harness.harness_id();
        self.harness = self.harness.name(name).system_prompt(system_prompt);
        self.session = self.session.harness(harness_id);
        self
    }

    /// Configure the seeded agent. Mutates the existing `AgentBuilder` in
    /// place so previously configured fields (e.g. `network_access`) are
    /// preserved regardless of call order.
    pub fn agent(mut self, name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        let agent_id = self.agent.agent_id();
        self.agent = self.agent.name(name).system_prompt(system_prompt);
        self.session = self.session.agent(agent_id);
        self
    }

    /// Add a harness-level capability.
    pub fn with_capability(
        self,
        capability: impl Into<everruns_capability::CapabilityRef>,
    ) -> Self {
        self.harness_capability(capability)
    }

    /// Add a harness-level capability.
    pub fn harness_capability(
        mut self,
        capability: impl Into<everruns_capability::CapabilityRef>,
    ) -> Self {
        self.harness = self.harness.capability(capability);
        self
    }

    /// Add an agent-level capability.
    pub fn agent_capability(
        mut self,
        capability: impl Into<everruns_capability::CapabilityRef>,
    ) -> Self {
        self.agent = self.agent.capability(capability);
        self
    }

    /// Enable a previously loaded plugin on the seeded agent.
    ///
    /// Adds a `plugin:{name}` capability ref to the agent with an empty config.
    /// The hydrated definition must be supplied separately via
    /// [`InProcessRuntimeBuilder::with_plugin_dir`] — this method only records
    /// the capability ref on the agent so the capability is active for this
    /// session. The builder looks up the hydrated config at build time from the
    /// `plugin_capability_configs` accumulated by `with_plugin_dir` calls.
    ///
    /// If you need to pass the fully hydrated `everruns_capability::CapabilityRef` (e.g.
    /// from [`InProcessRuntimeBuilder::plugin_capability`]), use
    /// [`Self::agent_capability`] directly.
    pub fn agent_plugin(mut self, name: &str) -> Self {
        self.agent = self.agent.capability(plugin_capability_id(name));
        self
    }

    /// Add a session-level capability.
    pub fn session_capability(
        mut self,
        capability: impl Into<everruns_capability::CapabilityRef>,
    ) -> Self {
        self.session = self.session.capability(capability);
        self
    }

    /// Configure session-scoped MCP servers (knowledge/integrations/runtime-mcp.md). Discovered
    /// and executed by the runtime alongside built-in tools.
    pub fn session_mcp_servers(mut self, mcp_servers: ScopedMcpServers) -> Self {
        self.session = self.session.mcp_servers(mcp_servers);
        self
    }

    pub fn agent_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.agent = self.agent.display_name(display_name);
        self
    }

    pub fn openrouter_attribution(
        mut self,
        http_referer: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        self.harness = self.harness.openrouter_attribution(http_referer, title);
        self
    }

    pub fn agent_description(mut self, description: impl Into<String>) -> Self {
        self.agent = self.agent.description(description);
        self
    }

    pub fn session_title(mut self, title: impl Into<String>) -> Self {
        self.session = self.session.title(title);
        self
    }

    pub fn locale(mut self, locale: impl Into<String>) -> Self {
        self.session = self.session.locale(locale);
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        // Harness/agent definitions carry no tags (EVE-877, EVE-881); session
        // tags drive execution metadata.
        self.session = self.session.tag(tag);
        self
    }

    pub fn session_model_id(mut self, model_id: ModelId) -> Self {
        self.session = self.session.model_id(model_id);
        self
    }

    pub fn harness_default_model_id(mut self, model_id: ModelId) -> Self {
        self.harness = self.harness.default_model_id(model_id);
        self
    }

    pub fn agent_default_model_id(mut self, model_id: ModelId) -> Self {
        self.agent = self.agent.default_model_id(model_id);
        self
    }

    pub fn agent_max_iterations(mut self, max_iterations: usize) -> Self {
        self.agent = self.agent.max_iterations(max_iterations);
        self
    }

    pub fn session_max_iterations(mut self, max_iterations: usize) -> Self {
        self.session = self.session.max_iterations(max_iterations);
        self
    }

    pub fn agent_tool(mut self, tool: ToolDefinition) -> Self {
        self.agent = self.agent.tool(tool);
        self
    }

    pub fn session_tool(mut self, tool: ToolDefinition) -> Self {
        self.session = self.session.tool(tool);
        self
    }

    pub fn harness_initial_file(mut self, file: everruns_core::InitialFile) -> Self {
        self.harness = self.harness.initial_file(file);
        self
    }

    pub fn agent_initial_file(mut self, file: everruns_core::InitialFile) -> Self {
        self.agent = self.agent.initial_file(file);
        self
    }

    pub fn session_initial_file(mut self, file: everruns_core::InitialFile) -> Self {
        self.session = self.session.initial_file(file);
        self
    }

    pub fn harness_network_access(mut self, network_access: NetworkAccessList) -> Self {
        self.harness = self.harness.network_access(network_access);
        self
    }

    pub fn agent_network_access(mut self, network_access: NetworkAccessList) -> Self {
        self.agent = self.agent.network_access(network_access);
        self
    }

    pub fn session_network_access(mut self, network_access: NetworkAccessList) -> Self {
        self.session = self.session.network_access(network_access);
        self
    }

    pub fn harness_id(&self) -> HarnessId {
        self.harness.harness_id()
    }

    pub fn agent_id(&self) -> AgentId {
        self.agent.agent_id()
    }

    /// Pin the seeded session's id. When unset, the underlying
    /// `SessionBuilder` generates a fresh `SessionId` at build time.
    ///
    /// Useful for embedders that need the id ahead of build — e.g. the
    /// `examples/coding-cli` JSONL session log uses `<id>.jsonl` as the
    /// filename and must open the file before the runtime exists.
    pub fn session_id(mut self, id: SessionId) -> Self {
        self.session = self.session.id(id);
        self
    }

    pub(crate) fn build(self) -> (SeededHarness, AgentDefinition, ExecutionSession, SessionId) {
        let session_id = self.session.session_id();
        (
            self.harness.build(),
            self.agent.build(),
            self.session.build(),
            session_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_builder_openrouter_attribution_adds_metadata_keys() {
        let harness = HarnessBuilder::new("app", "prompt")
            .openrouter_attribution("https://app.example", "Example App")
            .build();

        assert_eq!(
            harness
                .definition
                .embedder_metadata
                .get(OPENROUTER_HTTP_REFERER_METADATA_KEY)
                .map(String::as_str),
            Some("https://app.example")
        );
        assert_eq!(
            harness
                .definition
                .embedder_metadata
                .get(OPENROUTER_X_TITLE_METADATA_KEY)
                .map(String::as_str),
            Some("Example App")
        );
    }

    #[test]
    fn single_session_builder_openrouter_attribution_configures_harness() {
        let (harness, _agent, _session, _session_id) = SingleSessionBuilder::default()
            .openrouter_attribution("https://single.example", "Single App")
            .build();

        assert_eq!(
            harness
                .definition
                .embedder_metadata
                .get(OPENROUTER_HTTP_REFERER_METADATA_KEY)
                .map(String::as_str),
            Some("https://single.example")
        );
        assert_eq!(
            harness
                .definition
                .embedder_metadata
                .get(OPENROUTER_X_TITLE_METADATA_KEY)
                .map(String::as_str),
            Some("Single App")
        );
    }
}
