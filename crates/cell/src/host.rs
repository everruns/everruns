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

use everruns::{Agent, AgentPackage, FunctionTool, Model};
use serde_json::Value;

pub trait Host: Send + Sync + 'static {
    /// Build the agent `definition` describes, or say why this node cannot.
    fn agent(&self, definition: &Value) -> Result<Agent, String>;

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
        self.providers.insert(key.into(), Arc::new(factory));
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
        let model = self
            .providers
            .get(model.provider.as_str())
            .and_then(|factory| factory(&model.model))
            .ok_or_else(|| {
                format!(
                    "this node does not serve {} on {}",
                    model.model, model.provider
                )
            })?;
        builder
            .model(model)
            .package(&package)
            .map_err(|e| e.to_string())?
            .build()
            .map_err(|e| e.to_string())
    }

    fn tools(&self) -> Vec<Value> {
        self.tool_definitions()
    }
}
