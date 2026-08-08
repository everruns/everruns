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
/// and materializes independent in-process runtimes from it, one per
/// [`session`](Agent::session); the underlying runtime composition is kept in
/// private fields.
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
        if let Some(sim) = &self.model.sim {
            builder = builder.llm_sim(sim.clone());
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
