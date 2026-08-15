//! A minimal coding agent built only on the public `everruns` library.
//!
//! The example opens provider-owned Git workspace heads, binds them through a
//! Framework [`Environment`], and uses the built-in `session_file_system`
//! capability. It deliberately contains no process-global workspace state and
//! no application-defined filesystem tools.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use everruns::{
    Agent, AgentBuilder, Environment, LocalConfig, LocalGitWorkspaceProvider, Session, SessionId,
    Workspace, WorkspaceHead, WorkspaceHeadAccess, WorkspacePolicy,
};

/// System prompt for the coding agent.
pub const SYSTEM_PROMPT: &str = "\
You are a terminal coding assistant operating in the selected Git workspace head. \
Use the Framework `read_file`, `write_file`, `edit_file`, `list_directory`, and \
related session filesystem tools with paths under `/workspace`. Prefer small, \
verifiable changes and explain what you did.";

/// Public provider-driver selection for the example.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ProviderChoice {
    Openai,
    Anthropic,
    Openrouter,
    Ollama,
    Llmsim,
}

impl ProviderChoice {
    pub fn default_model(self) -> &'static str {
        match self {
            Self::Openai => "gpt-5.5",
            Self::Anthropic => "claude-sonnet-4-5",
            Self::Openrouter => "openai/gpt-5.2",
            Self::Ollama => "llama3.2",
            Self::Llmsim => "llmsim-coding-cli",
        }
    }
}

/// Command-line interface shared by the binary and its help-contract tests.
#[derive(Clone, Parser, Debug)]
#[command(
    name = "ercode",
    version,
    about = "Everruns coding CLI on durable Framework workspace heads",
    long_about = "Run a coding agent in a provider-owned Git workspace head. New heads are \
isolated by default and survive process exit. Resume reopens the exact head recorded for a typed \
session; shared-head mode is an explicit opt-in for a second session on an existing shared head.",
    after_long_help = "Examples:\n  \
ercode --offline --head feature --base main \"inspect the repository\"\n  \
ercode --offline --head left --base main\n  \
ercode --offline --resume session_0123456789abcdef0123456789abcdef\n  \
ercode --offline --head team --shared\n  \
ercode --offline --shared-head session_0123456789abcdef0123456789abcdef"
)]
pub struct Cli {
    /// Trusted local Git repository for a new head (default: current directory).
    #[arg(
        short = 'C',
        long = "cwd",
        value_name = "REPOSITORY",
        conflicts_with_all = ["resume", "shared_head"]
    )]
    cwd: Option<PathBuf>,

    /// Persistent Framework state (default: the OS user state directory).
    #[arg(long, value_name = "DIRECTORY")]
    state_dir: Option<PathBuf>,

    /// Create a new named head (isolated unless --shared is also present).
    #[arg(long, value_name = "NAME", conflicts_with_all = ["resume", "shared_head"])]
    head: Option<String>,

    /// Git revision used as the base for a new --head.
    #[arg(long, value_name = "REVISION", requires = "head")]
    base: Option<String>,

    /// Create the new --head as explicitly shared instead of isolated.
    #[arg(long, requires = "head")]
    shared: bool,

    /// Start a new session on the shared head recorded by this typed session id.
    #[arg(
        long,
        value_name = "SESSION_ID",
        conflicts_with_all = ["head", "resume"]
    )]
    shared_head: Option<SessionId>,

    /// Resume this typed session on its exact recorded workspace head.
    #[arg(
        long,
        value_name = "SESSION_ID",
        conflicts_with_all = ["head", "shared_head"]
    )]
    resume: Option<SessionId>,

    /// Deny workspace writes; new isolated heads are read/write by default.
    #[arg(long)]
    read_only: bool,

    /// Provider driver. Auto-detected from configured environment variables.
    #[arg(long, value_enum)]
    provider: Option<ProviderChoice>,

    /// Provider-visible model id (defaults per selected provider).
    #[arg(short = 'm', long)]
    model: Option<String>,

    /// Reasoning effort for providers that support it.
    #[arg(long, value_name = "EFFORT")]
    reasoning_effort: Option<String>,

    /// Run fully offline with the deterministic simulator (no credentials or network).
    #[arg(long)]
    offline: bool,

    /// Ask before write/edit/delete and shell tools (interactive mode only).
    #[arg(long)]
    ask: bool,

    /// Run one prompt non-interactively and print only the final answer.
    #[arg(short = 'p', long, value_name = "PROMPT", conflicts_with = "prompt")]
    print: Option<String>,

    /// MCP configuration file (default: <repository>/.mcp.json when present).
    #[arg(long, value_name = "FILE")]
    mcp_config: Option<PathBuf>,

    /// One-shot prompt. Omit to start an interactive REPL on the selected head.
    #[arg(value_name = "PROMPT", trailing_var_arg = true)]
    prompt: Vec<String>,
}

impl Cli {
    pub fn repository(&self) -> Result<PathBuf> {
        self.cwd
            .clone()
            .map(Ok)
            .unwrap_or_else(|| std::env::current_dir().context("resolve current directory"))
    }

    pub fn state_dir(&self) -> Result<PathBuf> {
        if let Some(path) = &self.state_dir {
            return Ok(path.clone());
        }
        dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .map(|root| root.join("everruns").join("ercode"))
            .ok_or_else(|| anyhow!("cannot resolve an OS user state directory; pass --state-dir"))
    }

    pub fn session_mode(&self) -> SessionMode {
        if let Some(session_id) = self.resume {
            SessionMode::Resume { session_id }
        } else if let Some(recorded_session) = self.shared_head {
            SessionMode::SharedHead { recorded_session }
        } else {
            SessionMode::NewHead {
                name: self.head.clone().unwrap_or_else(|| "ercode".to_string()),
                base: self.base.clone(),
                shared: self.shared,
            }
        }
    }

    pub fn workspace_policy(&self) -> WorkspacePolicy {
        if self.read_only {
            WorkspacePolicy::read_only()
        } else {
            WorkspacePolicy::read_write()
        }
    }

    pub fn provider(&self) -> ProviderChoice {
        if self.offline {
            return ProviderChoice::Llmsim;
        }
        if let Some(provider) = self.provider {
            return provider;
        }
        if std::env::var_os("OPENAI_API_KEY").is_some() {
            ProviderChoice::Openai
        } else if std::env::var_os("ANTHROPIC_API_KEY").is_some() {
            ProviderChoice::Anthropic
        } else if std::env::var_os("OPENROUTER_API_KEY").is_some() {
            ProviderChoice::Openrouter
        } else if std::env::var_os("OLLAMA_BASE_URL").is_some()
            || std::env::var_os("OLLAMA_API_KEY").is_some()
        {
            ProviderChoice::Ollama
        } else {
            ProviderChoice::Llmsim
        }
    }

    pub fn model(&self) -> String {
        self.model
            .clone()
            .or_else(|| std::env::var("EVERRUNS_CLI_MODEL").ok())
            .unwrap_or_else(|| self.provider().default_model().to_string())
    }

    pub fn offline(&self) -> bool {
        self.offline
    }

    pub fn reasoning_effort(&self) -> Option<&str> {
        self.reasoning_effort.as_deref()
    }

    pub fn prompt(&self) -> &[String] {
        &self.prompt
    }

    pub fn prompt_text(&self) -> Option<String> {
        self.print
            .clone()
            .or_else(|| (!self.prompt.is_empty()).then(|| self.prompt.join(" ")))
    }

    pub fn ask(&self) -> bool {
        self.ask && self.print.is_none()
    }

    pub fn mcp_config(&self) -> Option<PathBuf> {
        self.mcp_config.clone().or_else(|| {
            self.repository()
                .ok()
                .map(|root| root.join(".mcp.json"))
                .filter(|path| path.is_file())
        })
    }
}

/// Mutually exclusive ways to select the session and workspace head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionMode {
    NewHead {
        name: String,
        base: Option<String>,
        shared: bool,
    },
    SharedHead {
        recorded_session: SessionId,
    },
    Resume {
        session_id: SessionId,
    },
}

/// Public-API wrapper for one trusted local Git repository and its durable state.
pub struct CodingWorkspace {
    provider: Arc<LocalGitWorkspaceProvider>,
    workspace: Option<Workspace>,
    local: LocalConfig,
}

impl CodingWorkspace {
    /// Open durable Framework state without selecting a new repository.
    ///
    /// Typed resume and shared-head reuse reopen the recorded workspace through
    /// its opaque provider binding, so they intentionally use this constructor.
    pub fn from_state(state_dir: impl AsRef<Path>) -> Result<Self> {
        let state_dir = state_dir.as_ref();
        let provider = Arc::new(
            LocalGitWorkspaceProvider::new(state_dir.join("workspace-provider"))
                .context("initialize the local Git workspace provider")?,
        );
        Ok(Self {
            provider,
            workspace: None,
            local: LocalConfig::new(state_dir.join("runtime")),
        })
    }

    /// Open a trusted Git repository through the public local workspace provider.
    pub async fn open(repository: impl AsRef<Path>, state_dir: impl AsRef<Path>) -> Result<Self> {
        let mut context = Self::from_state(state_dir)?;
        let locator = repository.as_ref().to_str().ok_or_else(|| {
            anyhow!("local Git repository path must be valid UTF-8 for durable resume")
        })?;
        let workspace = Workspace::open(context.provider.clone(), locator)
            .await
            .context("open the trusted local Git repository")?;
        context.workspace = Some(workspace);
        Ok(context)
    }

    /// Attach durable local session state, the workspace provider, and policy.
    pub fn configure_agent(&self, builder: AgentBuilder, policy: WorkspacePolicy) -> AgentBuilder {
        builder
            .local(self.local.clone())
            .workspace_provider(self.provider.clone())
            .workspace_policy(policy)
    }

    /// Create a provider-owned head. Dropping the handle never destroys it.
    pub async fn create_head(
        &self,
        name: impl Into<String>,
        base: Option<&str>,
        shared: bool,
    ) -> Result<WorkspaceHead> {
        let workspace = self
            .workspace
            .as_ref()
            .ok_or_else(|| anyhow!("creating a head requires a trusted local Git repository"))?;
        let mut builder = workspace.head(name);
        if let Some(base) = base {
            builder = builder.from_revision(base);
        }
        if shared {
            builder = builder.shared();
        }
        builder.create().await.context("create Git workspace head")
    }

    /// Permanently bind a new session to the selected head before execution.
    pub async fn start_session(&self, agent: &Agent, head: WorkspaceHead) -> Result<Session> {
        let environment = Environment::builder()
            .workspace(head)
            .build()
            .context("build the workspace environment")?;
        agent
            .session()
            .environment(environment)
            .start()
            .await
            .context("bind the session to its workspace head")
    }

    /// Resume a typed session on its exact persisted head.
    pub async fn resume_session(&self, agent: &Agent, session_id: SessionId) -> Result<Session> {
        agent
            .resume(session_id)
            .await
            .with_context(|| format!("resume session {session_id} on its recorded head"))
    }

    /// Start a distinct session on an existing head that was created as shared.
    pub async fn start_shared_session(
        &self,
        agent: &Agent,
        recorded_session: SessionId,
    ) -> Result<Session> {
        let recorded = self.resume_session(agent, recorded_session).await?;
        let head = recorded
            .workspace_head()
            .cloned()
            .ok_or_else(|| anyhow!("session {recorded_session} has no recorded workspace head"))?;
        if head.access() != WorkspaceHeadAccess::Shared {
            bail!(
                "session {recorded_session} records an isolated head; use --resume for that session or create a head with --shared"
            );
        }
        self.start_session(agent, head).await
    }
}

/// Start a coding [`Agent`] builder using Framework filesystem tools.
pub fn agent_builder() -> AgentBuilder {
    Agent::builder()
        .instructions(SYSTEM_PROMPT)
        .capability("session_file_system")
        .capability("bashkit_shell")
        .capability("agent_instructions")
        .capability("skills")
        .capability("infinity_context")
        .capability("stateless_todo_list")
        .capability("loop_detection")
        .capability("prompt_caching")
        .capability("tool_output_persistence")
        .capability("web_fetch")
        .capability("duckduckgo")
}
