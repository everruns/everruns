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

use std::fmt;

use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::{AgentCapabilityConfig, DriverId, InitialFile, ResolvedModel, SessionId};
use everruns_runtime::{
    AgentBuilder as RuntimeAgentBuilder, HarnessBuilder, InProcessRuntime, InProcessRuntimeBuilder,
    SessionBuilder,
};

/// How an [`Agent`] talks to a model.
///
/// A `Model` carries the driver selection and model configuration behind a
/// value-first surface, so the public builder never exposes `ResolvedModel`,
/// `DriverId`, or the simulator config. Today the only constructor is
/// [`Model::simulated`]; real providers arrive in a later change.
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
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::BlankInstructions => {
                write!(f, "agent instructions must not be blank")
            }
            BuildError::MissingModel => write!(f, "agent requires a model"),
        }
    }
}

impl std::error::Error for BuildError {}

/// The immutable, validated description of an agent.
///
/// Produced by [`AgentBuilder::build`]. It holds the value-first configuration
/// and can materialize independent in-process runtimes from it; the underlying
/// runtime composition is kept in private fields.
// The stored configuration and `build_runtime` seam are exercised by this
// crate's tests and consumed by the forthcoming session API (EVE-831); they are
// intentionally private and not otherwise read in a non-test build yet.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Agent {
    name: String,
    instructions: String,
    model: Model,
    capabilities: Vec<AgentCapabilityConfig>,
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

    /// Materialize a fresh, independent in-process runtime for this agent.
    ///
    /// Each call assembles a new `Harness`/`Agent`/`Session` composition with
    /// freshly generated ids, so two calls yield two independent sessions. This
    /// is the private seam the public session API builds on; running turns is
    /// out of scope for this type.
    #[allow(dead_code)] // Consumed by the forthcoming session API (EVE-831).
    async fn build_runtime(
        &self,
    ) -> Result<(InProcessRuntime, SessionId), everruns_core::AgentLoopError> {
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
            .agent(agent_id)
            .capabilities(self.capabilities.clone());
        if let Some(parallel) = self.parallel_tool_calls {
            session = session.parallel_tool_calls(parallel);
        }
        for file in &self.initial_files {
            session = session.initial_file(file.clone());
        }
        let session_id = session.session_id();
        let session = session.build();

        let mut builder = InProcessRuntimeBuilder::new()
            .harness(harness)
            .agent(agent)
            .session(session)
            .default_model(self.model.resolved.clone());
        if let Some(sim) = &self.model.sim {
            builder = builder.llm_sim(sim.clone());
        }
        let runtime = builder.build().await?;
        Ok((runtime, session_id))
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
    pub fn model(mut self, model: Model) -> Self {
        self.model = Some(model);
        self
    }

    /// Set a human-readable name. Optional; defaults to `"agent"`.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add a tool the agent can call, referenced by capability id.
    ///
    /// Tools are wired through the capability system, so this is a
    /// tool-flavored alias of [`capability`](Self::capability).
    pub fn tool(mut self, tool: impl Into<AgentCapabilityConfig>) -> Self {
        self.capabilities.push(tool.into());
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
    /// Returns [`BuildError::BlankInstructions`] if instructions are missing or
    /// only whitespace, or [`BuildError::MissingModel`] if no model was set.
    pub fn build(self) -> Result<Agent, BuildError> {
        let instructions = self.instructions.unwrap_or_default();
        if instructions.trim().is_empty() {
            return Err(BuildError::BlankInstructions);
        }
        let model = self.model.ok_or(BuildError::MissingModel)?;
        let name = self.name.unwrap_or_else(|| "agent".to_string());

        Ok(Agent {
            name,
            instructions,
            model,
            capabilities: self.capabilities,
            initial_files: self.initial_files,
            parallel_tool_calls: self.parallel_tool_calls,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn one_agent_creates_two_independent_sessions() {
        let agent = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("Sure."))
            .build()
            .expect("valid agent");

        let (_first, a) = agent.build_runtime().await.expect("first runtime");
        let (_second, b) = agent.build_runtime().await.expect("second runtime");

        assert_ne!(a, b, "each session must be independent");
    }
}
