// Embedder-facing builders for core seed models.
// Decision: keep these in everruns-runtime so core domain structs stay literal
// data models while runtime owns the public embedding ergonomics.

use chrono::{DateTime, Utc};
use everruns_core::network_access::NetworkAccessList;
use everruns_core::{
    Agent, AgentCapabilityConfig, AgentId, AgentStatus, DEFAULT_ORG_PUBLIC_ID, Harness, HarnessId,
    HarnessStatus, ModelId, PrincipalId, ScopedMcpServers, Session, SessionId, SessionStatus,
    ToolDefinition,
};
use uuid::Uuid;

/// Builds a [`Harness`] with runtime-friendly defaults.
///
/// Use this when embedding Everruns directly and seeding a harness in code.
/// IDs and timestamps are generated at [`build`](Self::build) time unless
/// explicitly set.
#[derive(Debug, Clone)]
pub struct HarnessBuilder {
    id: HarnessId,
    name: String,
    display_name: Option<String>,
    description: Option<String>,
    system_prompt: String,
    parent_harness_id: Option<HarnessId>,
    default_model_id: Option<ModelId>,
    tags: Vec<String>,
    capabilities: Vec<AgentCapabilityConfig>,
    initial_files: Vec<everruns_core::InitialFile>,
    network_access: Option<NetworkAccessList>,
    mcp_servers: ScopedMcpServers,
    is_built_in: bool,
    status: HarnessStatus,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
}

impl HarnessBuilder {
    /// Create a harness builder from the required embedder-facing fields.
    pub fn new(name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            id: HarnessId::new(),
            name: name.into(),
            display_name: None,
            description: None,
            system_prompt: system_prompt.into(),
            parent_harness_id: None,
            default_model_id: None,
            tags: Vec::new(),
            capabilities: Vec::new(),
            initial_files: Vec::new(),
            network_access: None,
            mcp_servers: ScopedMcpServers::default(),
            is_built_in: false,
            status: HarnessStatus::Active,
            created_at: None,
            updated_at: None,
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

    pub fn parent_harness_id(mut self, parent_harness_id: HarnessId) -> Self {
        self.parent_harness_id = Some(parent_harness_id);
        self
    }

    pub fn default_model_id(mut self, default_model_id: ModelId) -> Self {
        self.default_model_id = Some(default_model_id);
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

    pub fn capability(mut self, capability: impl Into<AgentCapabilityConfig>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    pub fn with_capability(self, capability: impl Into<AgentCapabilityConfig>) -> Self {
        self.capability(capability)
    }

    pub fn capabilities<I, C>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<AgentCapabilityConfig>,
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

    pub fn mcp_servers(mut self, mcp_servers: ScopedMcpServers) -> Self {
        self.mcp_servers = mcp_servers;
        self
    }

    pub fn is_built_in(mut self, is_built_in: bool) -> Self {
        self.is_built_in = is_built_in;
        self
    }

    pub fn status(mut self, status: HarnessStatus) -> Self {
        self.status = status;
        self
    }

    pub fn created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = Some(created_at);
        self
    }

    pub fn updated_at(mut self, updated_at: DateTime<Utc>) -> Self {
        self.updated_at = Some(updated_at);
        self
    }

    /// Build the harness. Builders do not validate domain invariants.
    pub fn build(self) -> Harness {
        let created_at = self.created_at.unwrap_or_else(Utc::now);
        let updated_at = self.updated_at.unwrap_or(created_at);

        Harness {
            id: self.id,
            name: self.name,
            display_name: self.display_name,
            description: self.description,
            system_prompt: self.system_prompt,
            parent_harness_id: self.parent_harness_id,
            default_model_id: self.default_model_id,
            tags: self.tags,
            capabilities: self.capabilities,
            initial_files: self.initial_files,
            network_access: self.network_access,
            mcp_servers: self.mcp_servers,
            is_built_in: self.is_built_in,
            status: self.status,
            created_at,
            updated_at,
            archived_at: None,
            deleted_at: None,
        }
    }
}

/// Builds an [`Agent`] with runtime-friendly defaults.
#[derive(Debug, Clone)]
pub struct AgentBuilder {
    id: AgentId,
    name: String,
    display_name: Option<String>,
    description: Option<String>,
    system_prompt: String,
    default_model_id: Option<ModelId>,
    tags: Vec<String>,
    capabilities: Vec<AgentCapabilityConfig>,
    initial_files: Vec<everruns_core::InitialFile>,
    network_access: Option<NetworkAccessList>,
    max_iterations: Option<usize>,
    tools: Vec<ToolDefinition>,
    mcp_servers: ScopedMcpServers,
    status: AgentStatus,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
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
            tags: Vec::new(),
            capabilities: Vec::new(),
            initial_files: Vec::new(),
            network_access: None,
            max_iterations: None,
            tools: Vec::new(),
            mcp_servers: ScopedMcpServers::default(),
            status: AgentStatus::Active,
            created_at: None,
            updated_at: None,
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

    pub fn capability(mut self, capability: impl Into<AgentCapabilityConfig>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    pub fn with_capability(self, capability: impl Into<AgentCapabilityConfig>) -> Self {
        self.capability(capability)
    }

    pub fn capabilities<I, C>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<AgentCapabilityConfig>,
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

    pub fn status(mut self, status: AgentStatus) -> Self {
        self.status = status;
        self
    }

    pub fn created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = Some(created_at);
        self
    }

    pub fn updated_at(mut self, updated_at: DateTime<Utc>) -> Self {
        self.updated_at = Some(updated_at);
        self
    }

    /// Build the agent. Builders do not validate domain invariants.
    pub fn build(self) -> Agent {
        let created_at = self.created_at.unwrap_or_else(Utc::now);
        let updated_at = self.updated_at.unwrap_or(created_at);

        Agent {
            public_id: self.id,
            internal_id: Uuid::nil(),
            name: self.name,
            display_name: self.display_name,
            description: self.description,
            system_prompt: self.system_prompt,
            default_model_id: self.default_model_id,
            default_version_id: None,
            forked_from_agent_id: None,
            forked_from_version_id: None,
            root_agent_id: None,
            tags: self.tags,
            capabilities: self.capabilities,
            initial_files: self.initial_files,
            network_access: self.network_access,
            max_iterations: self.max_iterations,
            tools: self.tools,
            mcp_servers: self.mcp_servers,
            status: self.status,
            created_at,
            updated_at,
            archived_at: None,
            deleted_at: None,
            usage: None,
        }
    }
}

/// Builds a [`Session`] with runtime-friendly defaults.
#[derive(Debug, Clone)]
pub struct SessionBuilder {
    id: SessionId,
    organization_id: String,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    owner_principal_id: PrincipalId,
    title: Option<String>,
    locale: Option<String>,
    tags: Vec<String>,
    model_id: Option<ModelId>,
    capabilities: Vec<AgentCapabilityConfig>,
    tools: Vec<ToolDefinition>,
    mcp_servers: ScopedMcpServers,
    system_prompt: Option<String>,
    initial_files: Vec<everruns_core::InitialFile>,
    network_access: Option<NetworkAccessList>,
    max_iterations: Option<usize>,
    status: SessionStatus,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
}

impl SessionBuilder {
    /// Create a session builder for the required harness id.
    pub fn new(harness_id: HarnessId) -> Self {
        Self {
            id: SessionId::new(),
            organization_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id,
            agent_id: None,
            owner_principal_id: PrincipalId::from_seed(1),
            title: None,
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
            status: SessionStatus::Started,
            created_at: None,
            updated_at: None,
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

    pub fn owner_principal_id(mut self, owner_principal_id: PrincipalId) -> Self {
        self.owner_principal_id = owner_principal_id;
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
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

    pub fn capability(mut self, capability: impl Into<AgentCapabilityConfig>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    pub fn with_capability(self, capability: impl Into<AgentCapabilityConfig>) -> Self {
        self.capability(capability)
    }

    pub fn capabilities<I, C>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<AgentCapabilityConfig>,
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

    pub fn status(mut self, status: SessionStatus) -> Self {
        self.status = status;
        self
    }

    pub fn created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = Some(created_at);
        self
    }

    pub fn updated_at(mut self, updated_at: DateTime<Utc>) -> Self {
        self.updated_at = Some(updated_at);
        self
    }

    /// Build the session. Builders do not validate domain invariants.
    pub fn build(self) -> Session {
        let created_at = self.created_at.unwrap_or_else(Utc::now);
        let updated_at = self.updated_at.unwrap_or(created_at);

        Session {
            id: self.id,
            organization_id: self.organization_id,
            harness_id: self.harness_id,
            agent_id: self.agent_id,
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: self.owner_principal_id,
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            title: self.title,
            locale: self.locale,
            preview: None,
            output_preview: None,
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
            status: self.status,
            created_at,
            updated_at,
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
            active_schedule_count: None,
            features: Vec::new(),
            parent_session_id: None,
            subagent_name: None,
            subagent_task: None,
            subagent_status: None,
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
    pub fn with_capability(self, capability: impl Into<AgentCapabilityConfig>) -> Self {
        self.harness_capability(capability)
    }

    /// Add a harness-level capability.
    pub fn harness_capability(mut self, capability: impl Into<AgentCapabilityConfig>) -> Self {
        self.harness = self.harness.capability(capability);
        self
    }

    /// Add an agent-level capability.
    pub fn agent_capability(mut self, capability: impl Into<AgentCapabilityConfig>) -> Self {
        self.agent = self.agent.capability(capability);
        self
    }

    /// Add a session-level capability.
    pub fn session_capability(mut self, capability: impl Into<AgentCapabilityConfig>) -> Self {
        self.session = self.session.capability(capability);
        self
    }

    /// Configure session-scoped MCP servers (specs/runtime-mcp.md). Discovered
    /// and executed by the runtime alongside built-in tools.
    pub fn session_mcp_servers(mut self, mcp_servers: ScopedMcpServers) -> Self {
        self.session = self.session.mcp_servers(mcp_servers);
        self
    }

    pub fn harness_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.harness = self.harness.display_name(display_name);
        self
    }

    pub fn agent_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.agent = self.agent.display_name(display_name);
        self
    }

    pub fn harness_description(mut self, description: impl Into<String>) -> Self {
        self.harness = self.harness.description(description);
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
        let tag = tag.into();
        self.harness = self.harness.tag(tag.clone());
        self.agent = self.agent.tag(tag.clone());
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

    pub(crate) fn build(self) -> (Harness, Agent, Session, SessionId) {
        let session_id = self.session.session_id();
        (
            self.harness.build(),
            self.agent.build(),
            self.session.build(),
            session_id,
        )
    }
}
