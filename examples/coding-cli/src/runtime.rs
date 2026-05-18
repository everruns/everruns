// Runtime construction: wires `InProcessRuntime` with a real-disk
// `SessionFileStore` so the built-in `agent_instructions`, `file_system`,
// and `skills` capabilities operate against the embedder's actual workspace.
// Only the `bash` tool is custom — it shells out to the host instead of
// running against the VFS.

use crate::approval::ApprovalGate;
use crate::file_store_decorators::{ApprovalGatingFileStore, WriteBlocklistFileStore};
use crate::tools::{BashTool, Workspace};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use everruns_core::capabilities::{
    AGENT_INSTRUCTIONS_CAPABILITY_ID, AgentInstructionsCapability, Capability, CapabilityStatus,
    CurrentTimeCapability, FileSystemCapability, INFINITY_CONTEXT_CAPABILITY_ID,
    InfinityContextCapability, LoopDetectionCapability, PROMPT_CACHING_CAPABILITY_ID,
    PromptCachingCapability, SKILLS_CAPABILITY_ID, SkillsCapability, StatelessTodoListCapability,
};
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::memory::{
    InMemoryAgentStore, InMemoryEventEmitter, InMemoryHarnessStore, InMemoryLlmProviderStore,
    InMemoryMemoryStore, InMemoryMessageRetriever,
};
use everruns_core::tools::Tool;
use everruns_core::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::{
    Agent, AgentCapabilityConfig, AgentStatus, CapabilityRegistry, Harness, HarnessStatus,
    ModelWithProvider, PlatformDefinition, PrincipalId, Session, SessionStatus,
};
use everruns_runtime::{
    InMemorySessionStorageStore, InMemorySessionStore, InProcessRuntime, InProcessRuntimeBuilder,
    RealDiskFileStore, RuntimeBackends, RuntimeFileStore,
};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

// ---------- coding-cli's only custom capability: the bash tool ----------

struct CodingBashCapability {
    workspace: Workspace,
    gate: Arc<ApprovalGate>,
}

impl Capability for CodingBashCapability {
    fn id(&self) -> &str {
        "coding_cli_bash"
    }
    fn name(&self) -> &str {
        "Coding CLI Bash"
    }
    fn description(&self) -> &str {
        "Shell command execution rooted at the host workspace. Requires user approval."
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn category(&self) -> Option<&str> {
        Some("Examples")
    }
    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "Use `bash` for shell commands when needed (tests, lint, git status, etc.). \
             Prefer the filesystem tools for file reads and edits. Both ask the user for approval.",
        )
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(BashTool::new(
            self.workspace.clone(),
            self.gate.clone(),
        ))]
    }
}

// ---------- provider selection ----------

#[derive(Clone, Debug)]
pub enum ProviderChoice {
    Anthropic { model: String },
    OpenAi { model: String },
    Sim,
}

impl ProviderChoice {
    /// Pick a default based on env vars. CLI flags override this in `main`.
    /// OpenAI is preferred when both keys are present so the out-of-the-box
    /// default model stays `gpt-5.5`.
    pub fn from_env() -> Self {
        if std::env::var("OPENAI_API_KEY")
            .ok()
            .is_some_and(|v| !v.is_empty())
        {
            return Self::OpenAi {
                model: std::env::var("EVERRUNS_CLI_MODEL")
                    .unwrap_or_else(|_| "gpt-5.5".to_string()),
            };
        }
        if std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .is_some_and(|v| !v.is_empty())
        {
            return Self::Anthropic {
                model: std::env::var("EVERRUNS_CLI_MODEL")
                    .unwrap_or_else(|_| "claude-sonnet-4-5".to_string()),
            };
        }
        Self::Sim
    }

    pub fn label(&self) -> String {
        match self {
            Self::Anthropic { model } => format!("anthropic/{model}"),
            Self::OpenAi { model } => format!("openai/{model}"),
            Self::Sim => "llmsim (no API key)".to_string(),
        }
    }
}

// ---------- runtime bundle ----------

pub struct RuntimeBundle {
    pub runtime: Arc<InProcessRuntime>,
    pub session_id: SessionId,
    pub workspace_root: PathBuf,
    pub provider_label: String,
    pub instruction_files: Vec<String>,
    pub tool_names: Vec<String>,
}

pub async fn build(
    workspace_root: PathBuf,
    provider: ProviderChoice,
    gate: Arc<ApprovalGate>,
) -> Result<RuntimeBundle> {
    let canonical_root = std::fs::canonicalize(&workspace_root)
        .with_context(|| format!("canonicalize workspace: {}", workspace_root.display()))?;
    let workspace = Workspace::new(canonical_root.clone());

    // Build the FileStore stack: ApprovalGating(WriteBlocklist(RealDisk)).
    let disk = Arc::new(RealDiskFileStore::new(&canonical_root)?);
    let blocklisted = Arc::new(WriteBlocklistFileStore::new(disk));
    let gated = Arc::new(ApprovalGatingFileStore::new(blocklisted, gate.clone()));
    let file_store: Arc<dyn RuntimeFileStore> = gated;

    // The rest of the backends stay in memory.
    let event_emitter = InMemoryEventEmitter::new();
    let backends = RuntimeBackends {
        harness_store: Arc::new(InMemoryHarnessStore::new()),
        agent_store: Arc::new(InMemoryAgentStore::new()),
        session_store: Arc::new(InMemorySessionStore::new()),
        message_store: Arc::new(InMemoryMessageRetriever::new()),
        provider_store: Arc::new(InMemoryLlmProviderStore::new()),
        event_emitter: Arc::new(event_emitter.clone()),
        event_collector: Some(Arc::new(event_emitter)),
        file_store,
        storage_store: Arc::new(InMemorySessionStorageStore::new()),
        memory_store: Arc::new(InMemoryMemoryStore::new()),
    };

    // Register a curated set of built-in capabilities (no opinionated bundle
    // — we want a tight, predictable surface for the coding-CLI) plus our
    // bash capability.
    //
    // Filesystem-anchored (all read via the FileStore stack we built above,
    // so they target the real workspace transparently):
    //   * agent_instructions   — re-reads AGENTS.md every turn
    //   * session_file_system  — read/write/edit/list/grep/delete/stat tools
    //   * skills               — discovers SKILL.md under /.agents/skills/
    //
    // Non-filesystem, but useful for a coding agent:
    //   * infinity_context     — keeps long sessions usable; adds query_history
    //   * current_time         — date/time tool, useful for git/time-anchored work
    //   * stateless_todo_list  — write_todos tool for multi-step tasks
    //   * loop_detection       — safety net against repeated identical tool calls
    //   * prompt_caching       — Anthropic prompt caching; free token savings
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(AgentInstructionsCapability);
    capabilities.register(FileSystemCapability);
    capabilities.register(SkillsCapability);
    capabilities.register(InfinityContextCapability);
    capabilities.register(CurrentTimeCapability);
    capabilities.register(StatelessTodoListCapability);
    capabilities.register(LoopDetectionCapability);
    capabilities.register(PromptCachingCapability::new());
    capabilities.register(CodingBashCapability {
        workspace: workspace.clone(),
        gate,
    });

    let mut driver_registry = DriverRegistry::new();
    let mut llm_sim: Option<LlmSimConfig> = None;
    let default_model = match &provider {
        ProviderChoice::Anthropic { model } => {
            let key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| anyhow!("ANTHROPIC_API_KEY not set"))?;
            everruns_anthropic::register_driver(&mut driver_registry);
            ModelWithProvider {
                model: model.clone(),
                provider_type: LlmProviderType::Anthropic,
                api_key: Some(key),
                base_url: None,
            }
        }
        ProviderChoice::OpenAi { model } => {
            let key =
                std::env::var("OPENAI_API_KEY").map_err(|_| anyhow!("OPENAI_API_KEY not set"))?;
            everruns_openai::register_driver(&mut driver_registry);
            ModelWithProvider {
                model: model.clone(),
                provider_type: LlmProviderType::Openai,
                api_key: Some(key),
                base_url: None,
            }
        }
        ProviderChoice::Sim => {
            llm_sim = Some(
                LlmSimConfig::fixed(
                    "I'm running in offline mode (llmsim — no API key set). \
                     Set ANTHROPIC_API_KEY or OPENAI_API_KEY for real responses.",
                )
                .with_model("llmsim-coding-cli"),
            );
            ModelWithProvider {
                model: "llmsim-coding-cli".into(),
                provider_type: LlmProviderType::LlmSim,
                api_key: Some("fake-key".into()),
                base_url: None,
            }
        }
    };

    let platform = PlatformDefinition::new(capabilities, driver_registry);

    let harness_id = HarnessId::new();
    let agent_id = AgentId::new();
    let session_id = SessionId::new();

    let mut builder = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .backends(backends)
        .default_model(default_model)
        .harness(coding_harness(harness_id))
        .agent(coding_agent(agent_id))
        .session(coding_session(
            session_id,
            harness_id,
            agent_id,
            &canonical_root,
        ));
    if let Some(cfg) = llm_sim {
        builder = builder.llm_sim(cfg);
    }
    let runtime = builder.build().await?;

    let context = runtime.load_context(session_id).await?;
    let tool_names = context
        .runtime_agent
        .tools
        .iter()
        .map(|t| t.name().to_string())
        .collect();

    // Probe for AGENTS.md / CLAUDE.md / .agents.md on disk for the startup
    // banner only — the agent itself sees content via AgentInstructionsCapability,
    // which re-reads /AGENTS.md every turn through the runtime FileStore.
    let instruction_files = ["AGENTS.md", "CLAUDE.md", ".agents.md"]
        .iter()
        .filter(|f| canonical_root.join(f).exists())
        .map(|s| s.to_string())
        .collect();

    Ok(RuntimeBundle {
        runtime: Arc::new(runtime),
        session_id,
        workspace_root: canonical_root,
        provider_label: provider.label(),
        instruction_files,
        tool_names,
    })
}

fn coding_harness(id: HarnessId) -> Harness {
    Harness {
        id,
        name: "coding-cli".into(),
        display_name: Some("Coding CLI".into()),
        description: Some("Embedded terminal coding agent.".into()),
        system_prompt: "You are a precise, terse coding assistant. \
            Make minimal, surgical changes. Always explain non-obvious decisions briefly."
            .into(),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec!["example".into(), "coding".into()],
        capabilities: vec![
            AgentCapabilityConfig::new(AGENT_INSTRUCTIONS_CAPABILITY_ID),
            AgentCapabilityConfig::new("session_file_system"),
            AgentCapabilityConfig::new(SKILLS_CAPABILITY_ID),
            AgentCapabilityConfig::new(INFINITY_CONTEXT_CAPABILITY_ID),
            AgentCapabilityConfig::new("current_time"),
            AgentCapabilityConfig::new("stateless_todo_list"),
            AgentCapabilityConfig::new("loop_detection"),
            AgentCapabilityConfig::new(PROMPT_CACHING_CAPABILITY_ID),
            AgentCapabilityConfig::new("coding_cli_bash"),
        ],
        mcp_servers: Default::default(),
        initial_files: vec![],
        network_access: None,
        is_built_in: false,
        status: HarnessStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        archived_at: None,
        deleted_at: None,
    }
}

fn coding_agent(id: AgentId) -> Agent {
    Agent {
        public_id: id,
        internal_id: Uuid::nil(),
        name: "coding-agent".into(),
        display_name: Some("Coding Agent".into()),
        description: Some("Reads, edits, and runs commands inside a project workspace.".into()),
        system_prompt: "Default to investigation before edits. Cite file paths and line numbers \
                        when discussing code. Run tests when you change behavior."
            .into(),
        default_model_id: None,
        default_version_id: None,
        forked_from_agent_id: None,
        forked_from_version_id: None,
        root_agent_id: None,
        tags: vec!["example".into()],
        capabilities: vec![],
        mcp_servers: Default::default(),
        initial_files: vec![],
        network_access: None,
        max_iterations: Some(20),
        tools: vec![],
        status: AgentStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        archived_at: None,
        deleted_at: None,
        usage: None,
    }
}

fn coding_session(
    id: SessionId,
    harness_id: HarnessId,
    agent_id: AgentId,
    root: &std::path::Path,
) -> Session {
    Session {
        id,
        organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
        harness_id,
        agent_id: Some(agent_id),
        agent_version_id: None,
        agent_identity_id: None,
        owner_principal_id: PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        owner: None,
        effective_owner: None,
        title: Some(format!("coding-cli @ {}", root.display())),
        locale: None,
        preview: None,
        output_preview: None,
        tags: vec!["coding-cli".into()],
        model_id: None,
        capabilities: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        system_prompt: None,
        initial_files: vec![],
        hints: None,
        network_access: None,
        max_iterations: None,
        status: SessionStatus::Started,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_at: None,
        finished_at: None,
        usage: None,
        is_pinned: None,
        active_schedule_count: None,
        features: vec![],
        parent_session_id: None,
        subagent_name: None,
        subagent_task: None,
        subagent_status: None,
        blueprint_id: None,
        blueprint_config: None,
    }
}
