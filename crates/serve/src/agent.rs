//! The declarative agent spec returned by `#[agent]` functions.
//!
//! Decision: `serve::Agent` is a description (model string, instructions,
//! tool filter), not an `everruns::Agent`. The host turns it into an
//! `everruns::Agent` per session, after it has resolved the model through the
//! gateway, attached discovered tools/skills/connections, and bound a `Cx`
//! for that session. Keeping the spec declarative is what lets the manifest
//! list model strings and lets `dev` hot-reload the instructions.

use std::sync::Arc;

use everruns::LlmSimConfig;

/// A Markdown file from `agent/`, embedded at build time. Create with
/// [`md!`](crate::md).
#[derive(Clone, Debug)]
pub struct Markdown {
    path: &'static str,
    embedded: &'static str,
    disk: &'static str,
}

impl Markdown {
    #[doc(hidden)]
    pub const fn __embedded(
        path: &'static str,
        embedded: &'static str,
        disk: &'static str,
    ) -> Self {
        Self {
            path,
            embedded,
            disk,
        }
    }

    /// Path relative to `agent/`.
    pub fn path(&self) -> &'static str {
        self.path
    }

    /// The text: from disk in `dev` (hot reload), else as embedded.
    pub fn load(&self, hot_reload: bool) -> String {
        if hot_reload && let Ok(text) = std::fs::read_to_string(self.disk) {
            return text;
        }
        self.embedded.to_string()
    }
}

/// Where an agent's always-on prompt comes from.
#[derive(Clone, Debug)]
pub enum Instructions {
    Text(String),
    Markdown(Markdown),
}

impl Instructions {
    pub(crate) fn load(&self, hot_reload: bool) -> String {
        match self {
            Instructions::Text(text) => text.clone(),
            Instructions::Markdown(markdown) => markdown.load(hot_reload),
        }
    }
}

impl From<&str> for Instructions {
    fn from(text: &str) -> Self {
        Instructions::Text(text.to_string())
    }
}

impl From<String> for Instructions {
    fn from(text: String) -> Self {
        Instructions::Text(text)
    }
}

impl From<Markdown> for Instructions {
    fn from(markdown: Markdown) -> Self {
        Instructions::Markdown(markdown)
    }
}

type Customize = Arc<dyn Fn(everruns::AgentBuilder) -> everruns::AgentBuilder + Send + Sync>;

/// An agent of this app. Build one with [`Agent::builder`] inside an
/// `#[agent]` function.
#[derive(Clone)]
pub struct Agent {
    pub(crate) model: String,
    pub(crate) instructions: Option<Instructions>,
    pub(crate) description: Option<String>,
    pub(crate) tools: Option<Vec<String>>,
    pub(crate) offline: Option<LlmSimConfig>,
    pub(crate) customize: Option<Customize>,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("model", &self.model)
            .field("instructions", &self.instructions)
            .field("tools", &self.tools)
            .finish_non_exhaustive()
    }
}

impl Agent {
    /// Start describing an agent.
    pub fn builder() -> AgentBuilder {
        AgentBuilder {
            agent: Agent {
                model: String::new(),
                instructions: None,
                description: None,
                tools: None,
                offline: None,
                customize: None,
            },
        }
    }

    /// The model string, e.g. `"anthropic/claude-sonnet-5"`.
    pub fn model(&self) -> &str {
        &self.model
    }
}

/// Builder for [`Agent`].
#[must_use]
pub struct AgentBuilder {
    agent: Agent,
}

impl AgentBuilder {
    /// A gateway model string, `provider/model`. The host resolves it; the
    /// app carries no provider keys. `"sim"` always uses the simulator.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.agent.model = model.into();
        self
    }

    /// The always-on prompt. Defaults to `agent/instructions.md` when that
    /// file exists.
    pub fn instructions(mut self, instructions: impl Into<Instructions>) -> Self {
        self.agent.instructions = Some(instructions.into());
        self
    }

    /// One line on what this agent does. Shown in the agent card; for a
    /// subagent it is the description of its `ask_<name>` tool.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.agent.description = Some(description.into());
        self
    }

    /// Restrict the agent to these discovered tools. By default an agent gets
    /// every `#[tool]` in the app.
    pub fn tools<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.agent.tools = Some(names.into_iter().map(Into::into).collect());
        self
    }

    /// The simulator script used when no model gateway is configured, so the
    /// app runs offline in `dev` and in evals. See [`crate::sim`].
    pub fn offline(mut self, script: LlmSimConfig) -> Self {
        self.agent.offline = Some(script);
        self
    }

    /// Escape hatch: adjust the underlying `everruns::AgentBuilder` (hooks,
    /// capabilities, iteration limits) after serve has configured it.
    pub fn customize(
        mut self,
        f: impl Fn(everruns::AgentBuilder) -> everruns::AgentBuilder + Send + Sync + 'static,
    ) -> Self {
        self.agent.customize = Some(Arc::new(f));
        self
    }

    /// Finish. Validation happens in `App::builder().discover()`, where every
    /// problem in the app is reported together.
    pub fn build(self) -> Agent {
        self.agent
    }
}
