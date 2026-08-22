//! A minimal coding agent built only on the public `everruns` library.
//!
//! The example opens provider-owned Git workspace heads, binds them directly
//! when creating a Framework session, and equips one Agent with owner-defined
//! typed capabilities. It deliberately contains no process-global workspace
//! state and no application-defined filesystem tools.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use everruns::{
    Agent, AgentBuilder, AgentInstructionsConfig, BashkitShell, DuckDuckGo, FileSystem,
    LocalConfig, LocalGitWorkspaceProvider, Model, Session, SessionId, Skills, StatelessTodoList,
    WebFetch, Workspace, WorkspaceHead, WorkspaceHeadAccess, WorkspacePolicy,
};

/// System prompt for the coding agent.
pub const SYSTEM_PROMPT: &str = "\
You are a terminal coding assistant operating in the selected Git workspace head. \
Use the Framework `read_file`, `write_file`, `edit_file`, `list_directory`, and \
related session filesystem tools with paths under `/workspace`. Prefer small, \
verifiable changes and explain what you did.";

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

    /// OpenAI model id.
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
        user_state_dir()
            .or_else(user_data_local_dir)
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

    pub fn model(&self) -> String {
        self.model
            .clone()
            .or_else(|| std::env::var("EVERRUNS_CLI_MODEL").ok())
            .unwrap_or_else(|| "gpt-5.6-terra".to_string())
    }

    pub fn offline(&self) -> bool {
        self.offline || std::env::var_os("OPENAI_API_KEY").is_none()
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
}

/// Return the head recorded by a session when it was explicitly created as shared.
pub fn shared_head(recorded: &Session) -> Result<WorkspaceHead> {
    let session_id = recorded.session_id();
    let head = recorded
        .workspace_head()
        .cloned()
        .ok_or_else(|| anyhow!("session {session_id} has no recorded workspace head"))?;
    if head.access() != WorkspaceHeadAccess::Shared {
        bail!(
            "session {session_id} records an isolated head; use --resume for that session or create a head with --shared"
        );
    }
    Ok(head)
}

/// Start a coding [`Agent`] builder with a model and typed capabilities.
pub fn coding_agent(model: Model) -> AgentBuilder {
    Agent::builder()
        .name("ercode")
        .instructions(SYSTEM_PROMPT)
        .model(model)
        .capability(FileSystem)
        .capability(BashkitShell::new())
        .capability(AgentInstructionsConfig::default())
        .capability(Skills)
        .capability(StatelessTodoList)
        .capability(WebFetch::new())
        .capability(DuckDuckGo)
}

/// Reads an environment variable, treating an empty value as unset.
fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// `$XDG_STATE_HOME`, else `~/.local/state`. `None` off XDG platforms, matching
/// the `dirs` crate this replaces; callers fall back to the data directory.
fn user_state_dir() -> Option<PathBuf> {
    if cfg!(any(target_os = "macos", windows)) {
        return None;
    }
    match env_path("XDG_STATE_HOME") {
        Some(path) if path.is_absolute() => Some(path),
        _ => env_path("HOME").map(|home| home.join(".local/state")),
    }
}

/// `$XDG_DATA_HOME`, else `~/.local/share`; `~/Library/Application Support` on
/// macOS and `%LOCALAPPDATA%` on Windows.
fn user_data_local_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        env_path("HOME").map(|home| home.join("Library").join("Application Support"))
    }
    #[cfg(windows)]
    {
        env_path("LOCALAPPDATA")
    }
    #[cfg(all(not(target_os = "macos"), not(windows)))]
    {
        match env_path("XDG_DATA_HOME") {
            Some(path) if path.is_absolute() => Some(path),
            _ => env_path("HOME").map(|home| home.join(".local/share")),
        }
    }
}
