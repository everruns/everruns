//! What a node brings to the agents its clients define.
//!
//! A session carries its agent's definition: an Everruns agent package
//! manifest (instructions, model, capabilities, tools, MCP servers), sent
//! by the client when it creates the session. The runtime knows no agents.
//! A node binary supplies only what a definition cannot carry: model
//! providers with their credentials, and the code behind the tools a
//! definition names. Every turn rebuilds the agent from the definition, so
//! any node of any cell can run any session.

use std::collections::BTreeMap;
use std::sync::Arc;

use everruns::hosting::services::{HostProviders, HostServices};
use everruns::{Agent, AgentPackage, FunctionTool, Model};
use serde_json::Value;

/// What a node or executor serves for orgs on the data plane: their model
/// providers (`providers.rs`) and MCP credentials (`mcp.rs`).
#[derive(Clone)]
pub struct OrgServices {
    pub providers: HostProviders,
    pub mcp: Arc<crate::mcp::OrgMcp>,
}

pub trait Host: Send + Sync + 'static {
    /// Build the agent `definition` describes, or say why this node cannot.
    fn agent(&self, definition: &Value) -> Result<Agent, String>;

    /// [`agent`](Self::agent), where `orgs` serves what orgs on the data
    /// plane bring: the model providers this host does not serve itself,
    /// and the credentials of their MCP servers.
    fn agent_with(&self, definition: &Value, orgs: Option<&OrgServices>) -> Result<Agent, String> {
        let _ = orgs;
        self.agent(definition)
    }

    /// The tools this node offers, as a definition's `tools` entries.
    fn tools(&self) -> Vec<Value> {
        Vec::new()
    }
}

type ToolFactory = Arc<dyn Fn() -> FunctionTool + Send + Sync>;
type ModelFactory = Arc<dyn Fn(&str) -> Option<Model> + Send + Sync>;

/// A host built from named tools and model providers.
///
/// A definition's `tools` are bound by name from the tools registered here;
/// its `capabilities` are the Framework's, the platform ones this crate
/// implements, or the hosted catalog the v1 worker registers;
/// its `model.provider` must be registered here.
#[derive(Default, Clone)]
pub struct StandardHost {
    tools: BTreeMap<String, ToolFactory>,
    providers: BTreeMap<String, ModelFactory>,
}

impl StandardHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer a tool to every definition that names it.
    pub fn tool(mut self, factory: impl Fn() -> FunctionTool + Send + Sync + 'static) -> Self {
        let name = everruns_core::tools::Tool::name(&factory()).to_string();
        self.tools.insert(name, Arc::new(factory));
        self
    }

    /// Serve the models of provider `key` (a definition's `model.provider`):
    /// `factory` gets the model id and returns the model with its provider,
    /// or `None` for a model it does not serve. Credentials stay on the node.
    pub fn provider(
        mut self,
        key: &str,
        factory: impl Fn(&str) -> Option<Model> + Send + Sync + 'static,
    ) -> Self {
        // Provider names match as the Engine matches them (`ProviderKey`:
        // trimmed, any case): the control plane names a provider by its
        // display name, such as "LlmSim".
        self.providers.insert(
            everruns_contracts::runtime_provider::ProviderKey::new(key)
                .as_str()
                .to_owned(),
            Arc::new(factory),
        );
        self
    }

    /// The tool schemas this host offers, for clients writing definitions.
    pub fn tool_definitions(&self) -> Vec<Value> {
        self.tools
            .values()
            .filter_map(|factory| {
                use everruns_core::tools::Tool as _;
                let tool = factory();
                // A tool definition is plain data with string keys, so it
                // always serializes; one that did not would be left out.
                serde_json::to_value(everruns_contracts::tool_types::ToolDefinition::function(
                    tool.name(),
                    tool.description(),
                    tool.parameters_schema(),
                ))
                .ok()
            })
            .collect()
    }
}

/// Parse and validate a definition as an Everruns agent package.
pub fn package(definition: &Value) -> Result<AgentPackage, String> {
    package_with_bindings(definition).map(|(package, _)| package)
}

/// [`package`], its MCP header placeholders taken out to be filled per call
/// (`mcp.rs`).
pub fn package_with_bindings(
    definition: &Value,
) -> Result<(AgentPackage, crate::mcp::Bindings), String> {
    let mut manifest: everruns::AgentManifest =
        serde_json::from_value(definition.clone()).map_err(|e| e.to_string())?;
    let bindings = crate::mcp::take_bindings(&mut manifest.mcp_servers);
    // Decision: a definition's bundled agents are members this agent hands
    // work to. In a cell a member is its own durable session, created by a
    // hand-off tool from the member's definition, not an in-process member
    // of this turn; so they do not reach the Everruns builder.
    manifest.agents.clear();
    manifest.members = Some(Vec::new());
    let package = AgentPackage::new(manifest).map_err(|e| e.to_string())?;
    Ok((package, bindings))
}

impl Host for StandardHost {
    fn agent(&self, definition: &Value) -> Result<Agent, String> {
        self.agent_with(definition, None)
    }

    fn agent_with(&self, definition: &Value, orgs: Option<&OrgServices>) -> Result<Agent, String> {
        let (package, bindings) = package_with_bindings(definition)?;
        let manifest = package.manifest();
        let mut builder = Agent::builder();
        for tool in &manifest.tools {
            let factory = self
                .tools
                .get(tool.name())
                .ok_or_else(|| format!("this node has no tool {}", tool.name()))?;
            builder = builder.tool(factory());
        }
        let model = manifest
            .model
            .as_ref()
            .ok_or("the definition names no model")?;
        // A provider this host serves itself wins; otherwise the org's own
        // provider, by its key, when the org runs on the data plane.
        let local = self
            .providers
            .get(
                everruns_contracts::runtime_provider::ProviderKey::new(model.provider.as_str())
                    .as_str(),
            )
            .and_then(|factory| factory(&model.model));
        // Decision: the `tool_approval` capability an agent names is served
        // here, its decisions kept in the session's storage
        // (`DurableToolApprover`), so a risky call parks the turn as a
        // tool marked for approval does. Registered, not activated, and only
        // for an agent that names it: a registered one would stand in for
        // the gate of tools marked for approval (`durable_approvals`) and
        // let them run unasked.
        let mut services = HostServices::new();
        if manifest
            .capabilities
            .iter()
            .any(|c| c.id() == everruns_core::builtins::TOOL_APPROVAL_CAPABILITY_ID)
        {
            services = services.register(
                everruns_core::builtins::ToolApprovalCapability::new(Arc::new(
                    everruns_core::builtins::DurableToolApprover,
                ))
                .with_reviewer(Arc::new(everruns_core::builtins::ModelToolReviewer)),
            );
        }
        // A managed sandbox the control plane pinned for the session
        // (`sandbox.rs`): its provider key and the workspace sync come from
        // the activation, so the capability is the platform's own.
        if manifest.capabilities.iter().any(|c| {
            c.id() == everruns_capabilities::session_sandbox::SESSION_SANDBOX_CAPABILITY_ID
        }) {
            services =
                services.register(everruns_capabilities::capabilities::SessionSandboxCapability);
        }
        // Schedules live in the session's cell state (`platform.rs`); the
        // activation's schedule store reaches them.
        if manifest.capabilities.iter().any(|c| {
            c.id() == everruns_capabilities::capabilities::session_schedule::SESSION_SCHEDULE_CAPABILITY_ID
        }) {
            services =
                services.register(everruns_capabilities::capabilities::SessionScheduleCapability);
        }
        // Subagents run as sessions of their own through the activation's
        // platform store (`subagents.rs`).
        if manifest
            .capabilities
            .iter()
            .any(|c| c.id() == everruns_capabilities::capabilities::SUBAGENTS_CAPABILITY_ID)
        {
            services = services.register(everruns_capabilities::capabilities::SubagentCapability);
        }
        services = hosted_capabilities(services, &manifest.capabilities);
        let model = match (local, orgs) {
            (Some(model), _) => model,
            (None, Some(orgs)) => {
                services = services.providers(orgs.providers.clone());
                Model::on(model.provider.as_str(), model.model.as_str())
            }
            (None, None) => {
                return Err(format!(
                    "this node does not serve {} on {}",
                    model.model, model.provider
                ));
            }
        };
        // An org's MCP servers get their credentials per call, from the
        // calling session's org (`mcp.rs`).
        if let Some(orgs) = orgs
            && !manifest.mcp_servers.is_empty()
        {
            services = services.shared_mcp_auth(Arc::new(orgs.mcp.for_agent(bindings)));
        }
        // Decision: a session's workspace is its own durable one in the
        // cell, so its file tools may write it, as the platform's workers
        // allow (hidden and sensitive paths stay denied by the policy).
        builder
            .host_services(services)
            .model(model)
            .package(&package)
            .map_err(|e| e.to_string())?
            .workspace_policy(everruns::WorkspacePolicy::read_write())
            // A tool that needs approval parks the turn until a person
            // answers through `POST /v1/sessions/{id}/tool-approvals`: a
            // cell never holds a turn open on a callback.
            .durable_approvals()
            .build()
            .map_err(|e| e.to_string())
    }

    fn tools(&self) -> Vec<Value> {
        self.tool_definitions()
    }
}

/// The hosted catalog the v1 worker registers (portable builtins, the
/// integrations, the hosted product capabilities), for this deployment's
/// grade. Built once per process.
fn hosted_registry() -> &'static everruns_core::capabilities::CapabilityRegistry {
    static REGISTRY: std::sync::OnceLock<everruns_core::capabilities::CapabilityRegistry> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        everruns_capabilities::integrations_catalog::oss_capability_registry_for_grade(
            everruns_core::DeploymentGrade::from_env(),
        )
    })
}

/// Register on `services` the hosted capabilities `named` reaches (with the
/// ones they depend on) that neither the Framework nor this host already
/// serves.
///
/// Decision (step 11.6b): a cell runs the capabilities a v1 worker runs,
/// from the same catalog, so an org's agents build the same on either
/// plane. Only what the agent names is registered: a registered
/// capability can change a turn even unnamed (as `tool_approval` would).
/// What a capability's tools need at call time (an org's keys, the control
/// plane's stores) is checked per capability by the matrix in
/// `cell-testkit/tests/capabilities.rs`.
fn hosted_capabilities(
    mut services: HostServices,
    named: &[everruns::CapabilityRef],
) -> HostServices {
    let registry = hosted_registry();
    let mut have: std::collections::BTreeSet<String> =
        services.capability_ids().into_iter().collect();
    let mut queue: Vec<String> = named.iter().map(|c| c.id().to_owned()).collect();
    while let Some(id) = queue.pop() {
        if have.contains(&id) || everruns::hosting::services::is_framework_capability(&id) {
            continue;
        }
        let Some(capability) = registry.get(&id) else {
            continue;
        };
        have.insert(id);
        queue.extend(capability.dependencies().into_iter().map(str::to_owned));
        services = services.register_shared(capability.clone());
    }
    services
}
