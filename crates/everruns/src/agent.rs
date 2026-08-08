//! Value-first agent description (EVE-832).
//!
//! [`Agent::builder`] lets a library user describe an agent — instructions, a
//! model, optional tools and files — without constructing stored `Harness`,
//! `Agent`, `Session`, IDs, timestamps, statuses, registries, or a
//! `PlatformDefinition`. The builder validates the value-first configuration and
//! adapts it, inside [`AgentBuilder::build`], to the existing runtime builders.
//!
//! Running turns and multi-turn sessions are intentionally out of scope here;
//! this type only describes an agent and can materialize independent in-process
//! runtimes from that description.

use std::collections::HashSet;
use std::fmt;

use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::{AgentCapabilityConfig, DriverId, InitialFile, ResolvedModel, SessionId};
use everruns_runtime::{
    AgentBuilder as RuntimeAgentBuilder, HarnessBuilder, InProcessRuntime, InProcessRuntimeBuilder,
    SessionBuilder,
};

use crate::tool::{FunctionTool, IntoTool, Tool, validate_tool_name, validate_tool_schema};

/// How an [`Agent`] talks to a model.
///
/// A `Model` carries the driver selection and model configuration behind a
/// value-first surface, so the public builder never exposes `ResolvedModel`,
/// `DriverId`, or the simulator config. [`Model::simulated`] backs an offline
/// simulator; with the `openai` feature, an
/// [`OpenAI`](crate::providers::openai::OpenAI) configuration converts into a
/// `Model` that targets the real provider.
#[derive(Clone)]
pub struct Model {
    resolved: ResolvedModel,
    /// Present when the model is backed by the in-process LLM simulator.
    sim: Option<LlmSimConfig>,
}

impl Model {
    /// A deterministic in-process model that always replies with `response`.
    ///
    /// Backed by the `llmsim` driver, so an agent using it runs entirely
    /// offline — no credentials, no network.
    pub fn simulated(response: impl Into<String>) -> Self {
        Self {
            resolved: ResolvedModel {
                model: "llmsim-model".to_string(),
                provider_type: DriverId::LlmSim,
                api_key: Some("fake-key".to_string()),
                base_url: None,
                provider_metadata: None,
            },
            sim: Some(LlmSimConfig::fixed(response)),
        }
    }

    /// Build a `Model` that targets OpenAI's Responses API.
    ///
    /// Keeps `ResolvedModel`/`DriverId` off the public surface: the
    /// [`OpenAI`](crate::providers::openai::OpenAI) config is the value-first
    /// entry point, and `build_runtime` registers the OpenAI driver only for a
    /// model produced here.
    #[cfg(feature = "openai")]
    pub(crate) fn openai(config: crate::providers::openai::OpenAI) -> Self {
        let (model, api_key, base_url) = config.into_parts();
        Self {
            resolved: ResolvedModel {
                model,
                provider_type: DriverId::OpenAI,
                api_key: Some(api_key),
                base_url,
                provider_metadata: None,
            },
            sim: None,
        }
    }

    /// Whether this model needs the OpenAI driver registered on the runtime.
    #[cfg(feature = "openai")]
    fn is_openai(&self) -> bool {
        self.resolved.provider_type == DriverId::OpenAI
    }
}

#[cfg(test)]
impl Model {
    /// Test-only: a simulated model that records the provider-visible messages
    /// of every LLM call into `capture`, in call order. Lets session tests
    /// assert what history reached the provider on the second turn.
    pub(crate) fn simulated_capturing(
        response: impl Into<String>,
        capture: std::sync::Arc<std::sync::Mutex<Vec<Vec<everruns_core::LlmMessage>>>>,
    ) -> Self {
        let mut sim = LlmSimConfig::fixed(response);
        sim.message_capture = Some(capture);
        Self {
            resolved: ResolvedModel {
                model: "llmsim-model".to_string(),
                provider_type: DriverId::LlmSim,
                api_key: Some("fake-key".to_string()),
                base_url: None,
                provider_metadata: None,
            },
            sim: Some(sim),
        }
    }

    /// Test-only: a simulated model that emits the given per-turn tool-call
    /// sequence before replying with `response`. Lets a facade test drive the
    /// end-to-end tool-execution loop for a function tool.
    pub(crate) fn simulated_scripted(
        response: impl Into<String>,
        tool_call_sequence: Vec<Vec<everruns_core::ToolCall>>,
    ) -> Self {
        let sim = LlmSimConfig::fixed(response).with_tool_call_sequence(tool_call_sequence);
        Self {
            resolved: ResolvedModel {
                model: "llmsim-model".to_string(),
                provider_type: DriverId::LlmSim,
                api_key: Some("fake-key".to_string()),
                base_url: None,
                provider_metadata: None,
            },
            sim: Some(sim),
        }
    }
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Redact the resolved model's api key; report only the shape.
        f.debug_struct("Model")
            .field("model", &self.resolved.model)
            .field("provider_type", &self.resolved.provider_type)
            .field("simulated", &self.sim.is_some())
            .finish()
    }
}

/// Why an [`AgentBuilder`] could not produce an [`Agent`].
///
/// Stable, typed, and cheap to match on — no backend error leaks through.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BuildError {
    /// `instructions` was empty or only whitespace.
    BlankInstructions,
    /// No model was selected.
    MissingModel,
    /// A tool name is not a valid model-facing identifier.
    InvalidToolName {
        /// The rejected tool name.
        name: String,
        /// Why the name was rejected.
        reason: String,
    },
    /// A tool's JSON argument schema is invalid.
    InvalidToolSchema {
        /// The tool whose schema was rejected.
        name: String,
        /// Why the schema was rejected.
        reason: String,
    },
    /// Two tools were registered under the same name.
    DuplicateTool {
        /// The colliding tool name.
        name: String,
    },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::BlankInstructions => {
                write!(f, "agent instructions must not be blank")
            }
            BuildError::MissingModel => write!(f, "agent requires a model"),
            BuildError::InvalidToolName { name, reason } => {
                write!(f, "invalid tool name {name:?}: {reason}")
            }
            BuildError::InvalidToolSchema { name, reason } => {
                write!(f, "invalid JSON schema for tool {name:?}: {reason}")
            }
            BuildError::DuplicateTool { name } => {
                write!(f, "duplicate tool name {name:?}")
            }
        }
    }
}

impl std::error::Error for BuildError {}

/// The immutable, validated description of an agent.
///
/// Produced by [`AgentBuilder::build`]. It holds the value-first configuration
/// and materializes independent in-process runtimes from it, one per
/// [`session`](Agent::session); the underlying runtime composition is kept in
/// private fields.
#[derive(Clone, Debug)]
pub struct Agent {
    name: String,
    instructions: String,
    model: Model,
    capabilities: Vec<AgentCapabilityConfig>,
    function_tools: Vec<FunctionTool>,
    initial_files: Vec<InitialFile>,
    parallel_tool_calls: Option<bool>,
}

impl Agent {
    /// Start describing an agent.
    ///
    /// # Example
    ///
    /// ```
    /// use everruns::{Agent, Model};
    ///
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .model(Model::simulated("Sure."))
    ///     .name("assistant")
    ///     .tool("test_math")
    ///     .parallel_tool_calls(true)
    ///     .build()?;
    /// # Ok::<(), everruns::BuildError>(())
    /// ```
    pub fn builder() -> AgentBuilder {
        AgentBuilder::default()
    }

    /// Open a new, independent multi-turn [`Session`](crate::Session) with this
    /// agent.
    ///
    /// The session is lazy: the in-process runtime is assembled on the first
    /// [`Session::run`](crate::Session::run). Each session gets a fresh id and
    /// its own history, so two sessions from the same agent never share
    /// conversation state, and cloning an `Agent` never shares history.
    pub fn session(&self) -> crate::Session {
        crate::Session::new(self.clone(), SessionId::new())
    }

    /// Materialize a fresh in-process runtime for this agent, seeded with the
    /// given session id.
    ///
    /// Each call assembles a new `Harness`/`Agent`/`Session` composition, so
    /// distinct session ids yield independent sessions. This is the private
    /// seam [`Session`](crate::Session) builds on.
    pub(crate) async fn build_runtime(
        &self,
        session_id: SessionId,
    ) -> Result<InProcessRuntime, everruns_core::AgentLoopError> {
        let mut harness = HarnessBuilder::new(&self.name, &self.instructions)
            .capabilities(self.capabilities.clone());
        if let Some(parallel) = self.parallel_tool_calls {
            harness = harness.parallel_tool_calls(parallel);
        }
        for file in &self.initial_files {
            harness = harness.initial_file(file.clone());
        }
        let harness_id = harness.harness_id();
        let harness = harness.build();

        let mut agent = RuntimeAgentBuilder::new(&self.name, &self.instructions)
            .harness_id(harness_id)
            .capabilities(self.capabilities.clone());
        if let Some(parallel) = self.parallel_tool_calls {
            agent = agent.parallel_tool_calls(parallel);
        }
        let agent_id = agent.agent_id();
        let agent = agent.build();

        let mut session = SessionBuilder::new(harness_id)
            .id(session_id)
            .agent(agent_id)
            .capabilities(self.capabilities.clone());
        if let Some(parallel) = self.parallel_tool_calls {
            session = session.parallel_tool_calls(parallel);
        }
        for file in &self.initial_files {
            session = session.initial_file(file.clone());
        }
        let session = session.build();

        let mut builder = InProcessRuntimeBuilder::new()
            .harness(harness)
            .agent(agent)
            .session(session)
            .default_model(self.model.resolved.clone());
        // Register each function tool as a closure-backed, single-tool
        // capability so the runtime can execute the model's calls; the matching
        // capability ref was attached to the harness/agent/session above.
        for tool in &self.function_tools {
            builder = builder.capability(tool.clone().into_capability());
        }
        if let Some(sim) = &self.model.sim {
            builder = builder.llm_sim(sim.clone());
        }
        // A real OpenAI model needs its driver registered so a turn can reach the
        // provider; setting `default_model` alone is not enough. Register only for
        // an OpenAI model, so a simulated-model agent needs no provider wiring.
        #[cfg(feature = "openai")]
        if self.model.is_openai() {
            let mut registry = everruns_core::DriverRegistry::new();
            everruns_openai::register_driver(&mut registry);
            builder = builder.driver_registry(registry);
        }
        builder.build().await
    }
}

/// Fluent builder behind [`Agent::builder`].
///
/// Instructions and a model are required; everything else is optional. Blank
/// instructions or a missing model fail [`build`](Self::build) with a typed
/// [`BuildError`].
#[derive(Clone, Debug, Default)]
pub struct AgentBuilder {
    name: Option<String>,
    instructions: Option<String>,
    model: Option<Model>,
    capabilities: Vec<AgentCapabilityConfig>,
    tools: Vec<Tool>,
    initial_files: Vec<InitialFile>,
    parallel_tool_calls: Option<bool>,
}

impl AgentBuilder {
    /// Set the agent's system instructions. Required.
    pub fn instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = Some(instructions.into());
        self
    }

    /// Select the model the agent talks to. Required.
    ///
    /// Accepts a [`Model`] directly or anything convertible into one — for
    /// example an [`OpenAI`](crate::providers::openai::OpenAI) configuration
    /// under the `openai` feature.
    pub fn model(mut self, model: impl Into<Model>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set a human-readable name. Optional; defaults to `"agent"`.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add a tool the agent can call.
    ///
    /// Accepts anything that is [`IntoTool`](crate::IntoTool): a
    /// [`FunctionTool`](crate::FunctionTool) backed by an async function or
    /// closure, or a `&str`/`String` capability id for a capability-referenced
    /// tool. Tool names and JSON schemas are validated, and duplicate names
    /// rejected, at [`build`](Self::build).
    ///
    /// # Example
    ///
    /// ```
    /// use everruns::{Agent, FunctionTool, Model};
    /// use serde_json::json;
    ///
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .model(Model::simulated("done"))
    ///     .tool("test_math")
    ///     .tool(FunctionTool::new(
    ///         "roll",
    ///         "Roll a die.",
    ///         json!({ "type": "object", "properties": {} }),
    ///         |_args: serde_json::Value| async move { Ok::<_, String>(json!({ "value": 4 })) },
    ///     ))
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok::<(), everruns::BuildError>(())
    /// ```
    pub fn tool(mut self, tool: impl IntoTool) -> Self {
        self.tools.push(tool.into_tool());
        self
    }

    /// Add a capability the agent can use.
    pub fn capability(mut self, capability: impl Into<AgentCapabilityConfig>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    /// Prefer (or forbid) parallel tool calls within a single reasoning step.
    pub fn parallel_tool_calls(mut self, parallel: bool) -> Self {
        self.parallel_tool_calls = Some(parallel);
        self
    }

    /// Seed a file into the agent's initial workspace.
    pub fn initial_file(mut self, file: InitialFile) -> Self {
        self.initial_files.push(file);
        self
    }

    /// Validate the description and produce an [`Agent`].
    ///
    /// # Errors
    ///
    /// - [`BuildError::BlankInstructions`] if instructions are missing or only
    ///   whitespace.
    /// - [`BuildError::MissingModel`] if no model was set.
    /// - [`BuildError::InvalidToolName`] if a function tool's name is not a
    ///   valid model-facing identifier.
    /// - [`BuildError::InvalidToolSchema`] if a function tool's JSON schema is
    ///   not a valid arguments schema.
    /// - [`BuildError::DuplicateTool`] if two `.tool(..)` calls share a name.
    pub fn build(self) -> Result<Agent, BuildError> {
        let instructions = self.instructions.unwrap_or_default();
        if instructions.trim().is_empty() {
            return Err(BuildError::BlankInstructions);
        }
        let model = self.model.ok_or(BuildError::MissingModel)?;
        let name = self.name.unwrap_or_else(|| "agent".to_string());

        // Validate tools and split them into capability refs (attached to the
        // agent) and function tools (also registered on the runtime). Names
        // must be unique across all `.tool(..)` calls.
        let mut capabilities = self.capabilities;
        let mut function_tools = Vec::new();
        let mut seen_tool_names: HashSet<String> = HashSet::new();
        for tool in self.tools {
            let tool_name = tool.name().to_string();
            if !seen_tool_names.insert(tool_name.clone()) {
                return Err(BuildError::DuplicateTool { name: tool_name });
            }
            match tool {
                Tool::Capability(config) => capabilities.push(config),
                Tool::Function(function_tool) => {
                    validate_tool_name(function_tool.name()).map_err(|reason| {
                        BuildError::InvalidToolName {
                            name: tool_name.clone(),
                            reason,
                        }
                    })?;
                    validate_tool_schema(function_tool.schema()).map_err(|reason| {
                        BuildError::InvalidToolSchema {
                            name: tool_name.clone(),
                            reason,
                        }
                    })?;
                    capabilities.push(AgentCapabilityConfig::new(function_tool.name()));
                    function_tools.push(function_tool);
                }
            }
        }

        Ok(Agent {
            name,
            instructions,
            model,
            capabilities,
            function_tools,
            initial_files: self.initial_files,
            parallel_tool_calls: self.parallel_tool_calls,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use everruns_core::ToolCall;
    use serde_json::{Value, json};

    use super::*;
    use crate::FunctionTool;

    fn obj_schema() -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    #[test]
    fn build_rejects_invalid_tool_name() {
        let err = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("ok"))
            .tool(FunctionTool::new(
                "bad name",
                "desc",
                obj_schema(),
                |_: Value| async move { Ok::<_, String>(json!({})) },
            ))
            .build()
            .unwrap_err();
        assert!(
            matches!(err, BuildError::InvalidToolName { ref name, .. } if name == "bad name"),
            "got {err:?}"
        );
    }

    #[test]
    fn build_rejects_invalid_tool_schema() {
        let err = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("ok"))
            .tool(FunctionTool::new(
                "arr",
                "desc",
                json!({ "type": "array" }),
                |_: Value| async move { Ok::<_, String>(json!({})) },
            ))
            .build()
            .unwrap_err();
        assert!(
            matches!(err, BuildError::InvalidToolSchema { ref name, .. } if name == "arr"),
            "got {err:?}"
        );
    }

    #[test]
    fn build_rejects_duplicate_tool_names() {
        let make = || {
            FunctionTool::new("dup", "desc", obj_schema(), |_: Value| async move {
                Ok::<_, String>(json!({}))
            })
        };
        let err = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("ok"))
            .tool(make())
            .tool(make())
            .build()
            .unwrap_err();
        assert_eq!(
            err,
            BuildError::DuplicateTool {
                name: "dup".to_string()
            }
        );
    }

    #[tokio::test]
    async fn function_tool_executes_end_to_end() {
        // Capture what the handler received to prove args flowed in.
        let received: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let sink = received.clone();
        let tool = FunctionTool::new(
            "greet",
            "Greet a person by name.",
            json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"],
            }),
            move |args: Value| {
                let sink = sink.clone();
                async move {
                    *sink.lock().unwrap() = Some(args.clone());
                    let name = args["name"].as_str().unwrap_or("world");
                    Ok::<_, String>(json!({ "greeting": format!("Hello, {name}!") }))
                }
            },
        );

        let agent = Agent::builder()
            .instructions("Call greet when asked to greet someone.")
            .model(Model::simulated_scripted(
                "All done.",
                vec![
                    vec![ToolCall {
                        id: "call_greet_1".into(),
                        name: "greet".into(),
                        arguments: json!({ "name": "Ada" }),
                    }],
                    vec![],
                ],
            ))
            .tool(tool)
            .build()
            .expect("valid agent");

        let mut session = agent.session();
        let turn = session.run("Please greet Ada.").await.expect("turn runs");

        assert!(turn.success, "turn should succeed: {:?}", turn.error);
        assert_eq!(turn.tool_calls, 1, "the function tool must have executed");
        assert_eq!(turn.response, "All done.");
        assert_eq!(
            received.lock().unwrap().as_ref().expect("handler ran")["name"],
            json!("Ada"),
            "handler must receive the model's call arguments",
        );
    }

    #[tokio::test]
    async fn function_tool_handler_error_is_model_visible_not_a_panic() {
        let tool = FunctionTool::new(
            "always_fails",
            "Always returns an error.",
            obj_schema(),
            |_: Value| async move { Err::<Value, String>("boom".to_string()) },
        );

        let agent = Agent::builder()
            .instructions("Call the tool.")
            .model(Model::simulated_scripted(
                "Handled.",
                vec![
                    vec![ToolCall {
                        id: "call_fail_1".into(),
                        name: "always_fails".into(),
                        arguments: json!({}),
                    }],
                    vec![],
                ],
            ))
            .tool(tool)
            .build()
            .expect("valid agent");

        let mut session = agent.session();
        // The handler error becomes a tool result the model consumes; the turn
        // still completes rather than panicking.
        let turn = session.run("go").await.expect("turn runs");
        assert!(turn.success, "turn should recover from a tool error");
        assert_eq!(turn.tool_calls, 1);
    }

    #[test]
    fn build_rejects_blank_instructions() {
        let err = Agent::builder()
            .instructions("   ")
            .model(Model::simulated("hi"))
            .build()
            .unwrap_err();
        assert_eq!(err, BuildError::BlankInstructions);
    }

    #[test]
    fn build_rejects_missing_model() {
        let err = Agent::builder()
            .instructions("You are concise.")
            .build()
            .unwrap_err();
        assert_eq!(err, BuildError::MissingModel);
    }

    #[test]
    fn build_succeeds_with_simulator() {
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("Sure."))
            .name("assistant")
            .build()
            .expect("valid agent");
        assert_eq!(agent.name, "assistant");
    }

    #[cfg(feature = "openai")]
    #[test]
    fn openai_model_reports_provider_without_leaking_key() {
        use crate::providers::openai::OpenAI;

        let model = Model::openai(OpenAI::new("gpt-5-mini", "sk-super-secret"));
        assert!(model.is_openai());
        assert_eq!(model.resolved.model, "gpt-5-mini");
        assert!(model.sim.is_none(), "an OpenAI model uses no simulator");
        // The value-first `Debug` reports shape only, never the key.
        let rendered = format!("{model:?}");
        assert!(!rendered.contains("sk-super-secret"), "got {rendered}");
    }

    #[cfg(feature = "openai")]
    #[tokio::test]
    async fn openai_agent_builds_runtime_offline() {
        use crate::providers::openai::OpenAI;

        // Building the runtime registers the OpenAI driver and assembles the
        // in-process composition without any network call — the provider is only
        // contacted when a turn actually runs, which this test never does.
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(OpenAI::new("gpt-5-mini", "sk-test"))
            .build()
            .expect("valid agent");

        let runtime = agent
            .build_runtime(SessionId::new())
            .await
            .expect("openai runtime builds offline");
        let _ = runtime;
    }

    #[tokio::test]
    async fn build_runtime_seeds_the_requested_session_id() {
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("Sure."))
            .build()
            .expect("valid agent");

        let session_id = SessionId::new();
        let runtime = agent
            .build_runtime(session_id)
            .await
            .expect("runtime builds");
        // The seeded session id is usable directly: a caller can run a turn
        // against it without going through `default_session_id`.
        let result = runtime
            .run_turn(session_id, everruns_core::InputMessage::user("hi"))
            .await
            .expect("turn runs");
        assert!(result.success);
        assert_eq!(result.response, "Sure.");
    }
}
