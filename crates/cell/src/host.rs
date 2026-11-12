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

pub trait Host: Send + Sync + 'static {
    /// Build the agent `definition` describes, or say why this node cannot.
    fn agent(&self, definition: &Value) -> Result<Agent, String>;

    /// [`agent`](Self::agent), where `orgs` serves the model providers of
    /// orgs on the data plane (`providers.rs`) that this host does not
    /// serve itself.
    fn agent_with(
        &self,
        definition: &Value,
        orgs: Option<&HostProviders>,
    ) -> Result<Agent, String> {
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
/// its `capabilities` must be ones the Everruns Framework itself provides;
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
    let mut manifest: everruns::AgentManifest =
        serde_json::from_value(definition.clone()).map_err(|e| e.to_string())?;
    // Decision: a definition's bundled agents are members this agent hands
    // work to. In a cell a member is its own durable session, created by a
    // hand-off tool from the member's definition, not an in-process member
    // of this turn; so they do not reach the Everruns builder.
    manifest.agents.clear();
    manifest.members = Some(Vec::new());
    AgentPackage::new(manifest).map_err(|e| e.to_string())
}

impl Host for StandardHost {
    fn agent(&self, definition: &Value) -> Result<Agent, String> {
        self.agent_with(definition, None)
    }

    fn agent_with(
        &self,
        definition: &Value,
        orgs: Option<&HostProviders>,
    ) -> Result<Agent, String> {
        let package = package(definition)?;
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
            services = services.register(everruns_core::builtins::ToolApprovalCapability::new(
                Arc::new(everruns_core::builtins::DurableToolApprover),
            ));
        }
        let model = match (local, orgs) {
            (Some(model), _) => model,
            (None, Some(orgs)) => {
                services = services.providers(orgs.clone());
                Model::on(model.provider.as_str(), model.model.as_str())
            }
            (None, None) => {
                return Err(format!(
                    "this node does not serve {} on {}",
                    model.model, model.provider
                ));
            }
        };
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
