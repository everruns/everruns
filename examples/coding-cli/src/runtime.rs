// Runtime construction: wires `InProcessRuntime` through a platform
// `SessionFileSystemFactory` so the built-in `agent_instructions`,
// `file_system`, and `skills` capabilities operate against the embedder's
// actual workspace. Only the `bash` tool is custom — it shells out to the host
// instead of running against the VFS.

use crate::approval::ApprovalGate;
use crate::file_store_decorators::{ApprovalGatingFileStore, WriteBlocklistFileStore};
use crate::tools::{BashTool, Workspace};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use everruns_core::capabilities::{
    AGENT_INSTRUCTIONS_CAPABILITY_ID, AgentInstructionsCapability, Capability, CapabilityStatus,
    FileSystemCapability, INFINITY_CONTEXT_CAPABILITY_ID, InfinityContextCapability,
    LoopDetectionCapability, PROMPT_CACHING_CAPABILITY_ID, PromptCachingCapability,
    SKILLS_CAPABILITY_ID, SkillsCapability, StatelessTodoListCapability, WebFetchCapability,
};
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::tools::Tool;
use everruns_core::typed_id::SessionId;
use everruns_core::{
    AgentCapabilityConfig, CapabilityRegistry, ModelWithProvider, PlatformDefinition,
    SessionFileSystem, SessionFileSystemFactory, SessionFileSystemFactoryContext,
};
use everruns_integrations_duckduckgo::DuckDuckGoCapability;
use everruns_runtime::{
    InProcessRuntime, InProcessRuntimeBuilder, RealDiskFileStore, RuntimeBackends,
    RuntimeProviderStore,
};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

const HARNESS_PROMPT: &str = "You are a precise, terse coding assistant. \
    Make minimal, surgical changes. Always explain non-obvious decisions briefly.";

const AGENT_PROMPT: &str = "Default to investigation before edits. Cite file paths and line numbers \
     when discussing code. Run tests when you change behavior.";

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

struct CodingCliSessionFileSystemFactory {
    root: PathBuf,
    gate: Arc<ApprovalGate>,
}

#[async_trait]
impl SessionFileSystemFactory for CodingCliSessionFileSystemFactory {
    fn name(&self) -> &'static str {
        "CodingCliSessionFileSystemFactory"
    }

    async fn create_session_file_system(
        &self,
        _context: SessionFileSystemFactoryContext,
    ) -> everruns_core::Result<Arc<dyn SessionFileSystem>> {
        let disk: Arc<dyn SessionFileSystem> = Arc::new(RealDiskFileStore::new(&self.root)?);
        let blocklisted: Arc<dyn SessionFileSystem> = Arc::new(WriteBlocklistFileStore::new(disk));
        Ok(Arc::new(ApprovalGatingFileStore::new(
            blocklisted,
            self.gate.clone(),
        )))
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
            Self::Sim => "llmsim/llmsim-coding-cli".to_string(),
        }
    }

    pub fn model_suggestions() -> &'static [&'static str] {
        &[
            "openai/gpt-5.5",
            "openai/gpt-5.4",
            "openai/gpt-5.4-mini",
            "openai/gpt-5.3-codex",
            "openai/gpt-5.2",
            "anthropic/claude-sonnet-4-5",
            "anthropic/claude-opus-4-5",
            "anthropic/claude-haiku-4-5",
            "anthropic/claude-sonnet-4-6",
            "anthropic/claude-opus-4-6",
            "llmsim/llmsim-coding-cli",
        ]
    }

    fn resolve_model_spec(&self, spec: &str) -> Result<Self> {
        let spec = spec.trim();
        if let Some((provider, model)) = spec.split_once('/') {
            return Self::from_provider_model(provider, model);
        }
        self.with_current_provider_model(spec.to_string())
    }

    fn from_provider_model(provider: &str, model: &str) -> Result<Self> {
        let model = model.trim();
        if model.is_empty() {
            return Err(anyhow!("model id is required"));
        }
        match provider.trim().to_ascii_lowercase().as_str() {
            "anthropic" => Ok(Self::Anthropic {
                model: model.to_string(),
            }),
            "openai" => Ok(Self::OpenAi {
                model: model.to_string(),
            }),
            "llmsim" | "sim" => {
                if model == "llmsim-coding-cli" {
                    Ok(Self::Sim)
                } else {
                    Err(anyhow!("offline llmsim only supports llmsim-coding-cli"))
                }
            }
            other => Err(anyhow!(
                "unknown provider {other}; expected openai, anthropic, or llmsim"
            )),
        }
    }

    fn with_current_provider_model(&self, model: String) -> Result<Self> {
        match self {
            Self::Anthropic { .. } => Ok(Self::Anthropic { model }),
            Self::OpenAi { .. } => Ok(Self::OpenAi { model }),
            Self::Sim => {
                if model == "llmsim-coding-cli" {
                    Ok(Self::Sim)
                } else {
                    Err(anyhow!("offline llmsim only supports llmsim-coding-cli"))
                }
            }
        }
    }

    fn model_with_provider(&self) -> Result<ModelWithProvider> {
        match self {
            ProviderChoice::Anthropic { model } => {
                let key = std::env::var("ANTHROPIC_API_KEY")
                    .map_err(|_| anyhow!("ANTHROPIC_API_KEY not set"))?;
                Ok(ModelWithProvider {
                    model: model.clone(),
                    provider_type: LlmProviderType::Anthropic,
                    api_key: Some(key),
                    base_url: None,
                })
            }
            ProviderChoice::OpenAi { model } => {
                let key = std::env::var("OPENAI_API_KEY")
                    .map_err(|_| anyhow!("OPENAI_API_KEY not set"))?;
                Ok(ModelWithProvider {
                    model: model.clone(),
                    provider_type: LlmProviderType::Openai,
                    api_key: Some(key),
                    base_url: None,
                })
            }
            ProviderChoice::Sim => Ok(ModelWithProvider {
                model: "llmsim-coding-cli".into(),
                provider_type: LlmProviderType::LlmSim,
                api_key: Some("fake-key".into()),
                base_url: None,
            }),
        }
    }
}

// ---------- runtime bundle ----------

pub struct RuntimeBundle {
    pub runtime: Arc<InProcessRuntime>,
    pub session_id: SessionId,
    pub workspace_root: PathBuf,
    pub instruction_files: Vec<String>,
    pub tool_names: Vec<String>,
    provider: RwLock<ProviderChoice>,
    provider_store: Arc<dyn RuntimeProviderStore>,
}

impl RuntimeBundle {
    pub fn provider_label(&self) -> String {
        self.provider
            .read()
            .expect("provider lock poisoned")
            .label()
    }

    pub fn model_suggestions(&self) -> &'static [&'static str] {
        ProviderChoice::model_suggestions()
    }

    pub async fn set_model(&self, model: &str) -> Result<String> {
        let model = model.trim();
        if model.is_empty() {
            return Err(anyhow!("model id is required"));
        }
        let next = self
            .provider
            .read()
            .expect("provider lock poisoned")
            .resolve_model_spec(model)?;
        self.provider_store
            .set_default_model(next.model_with_provider()?)
            .await?;
        let label = next.label();
        *self.provider.write().expect("provider lock poisoned") = next;
        Ok(label)
    }
}

pub async fn build(
    workspace_root: PathBuf,
    provider: ProviderChoice,
    gate: Arc<ApprovalGate>,
) -> Result<RuntimeBundle> {
    let canonical_root = std::fs::canonicalize(&workspace_root)
        .with_context(|| format!("canonicalize workspace: {}", workspace_root.display()))?;
    let workspace = Workspace::new(canonical_root.clone());

    // Non-filesystem backends stay in memory; the session filesystem is owned
    // by the platform factory below.
    let backends = RuntimeBackends::in_memory();
    let provider_store = backends.provider_store.clone();

    // Register a curated set of built-in capabilities (no opinionated bundle
    // — we want a tight, predictable surface for the coding-CLI) plus our
    // bash capability.
    //
    // Filesystem-anchored (all read via the platform filesystem factory, so
    // they target the real workspace transparently):
    //   * agent_instructions   — re-reads AGENTS.md every turn
    //   * session_file_system  — read/write/edit/list/grep/delete/stat tools
    //   * skills               — discovers SKILL.md under /.agents/skills/
    //
    // Non-filesystem, but useful for a coding agent:
    //   * infinity_context     — keeps long sessions usable; adds query_history
    //   * stateless_todo_list  — write_todos tool for multi-step tasks
    //   * loop_detection       — safety net against repeated identical tool calls
    //   * prompt_caching       — Anthropic prompt caching; free token savings
    //   * duckduckgo           — free web search (`duckduckgo_search`); no API key
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(AgentInstructionsCapability);
    capabilities.register(FileSystemCapability);
    capabilities.register(SkillsCapability);
    capabilities.register(InfinityContextCapability);
    capabilities.register(StatelessTodoListCapability);
    capabilities.register(LoopDetectionCapability);
    capabilities.register(PromptCachingCapability::new());
    capabilities.register(DuckDuckGoCapability);
    capabilities.register(WebFetchCapability::from_env());
    capabilities.register(CodingBashCapability {
        workspace: workspace.clone(),
        gate: gate.clone(),
    });

    let mut driver_registry = DriverRegistry::new();
    everruns_anthropic::register_driver(&mut driver_registry);
    everruns_openai::register_driver(&mut driver_registry);
    let default_model = match &provider {
        ProviderChoice::Anthropic { .. } | ProviderChoice::OpenAi { .. } => {
            provider.model_with_provider()?
        }
        ProviderChoice::Sim => ModelWithProvider {
            model: "llmsim-coding-cli".into(),
            provider_type: LlmProviderType::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
        },
    };

    let platform = PlatformDefinition::builder()
        .capability_registry(capabilities)
        .driver_registry(driver_registry)
        .session_file_system_factory(Arc::new(CodingCliSessionFileSystemFactory {
            root: canonical_root.clone(),
            gate: gate.clone(),
        }))
        .build();

    // SingleSessionBuilder bundles the harness/agent/session shape with
    // defaults that runtime owns — keeps this example small and absorbs
    // future core-struct fields automatically.
    let session_title = format!("coding-cli @ {}", canonical_root.display());
    let harness_capabilities: Vec<AgentCapabilityConfig> = vec![
        // Configured to also pick up CLAUDE.md and .agents.md alongside the
        // default AGENTS.md. All three are re-read every turn (live-reload).
        AgentCapabilityConfig::with_config(
            AGENT_INSTRUCTIONS_CAPABILITY_ID,
            serde_json::json!({ "files": ["AGENTS.md", "CLAUDE.md", ".agents.md"] }),
        ),
        AgentCapabilityConfig::new("session_file_system"),
        AgentCapabilityConfig::new(SKILLS_CAPABILITY_ID),
        AgentCapabilityConfig::new(INFINITY_CONTEXT_CAPABILITY_ID),
        AgentCapabilityConfig::new("stateless_todo_list"),
        AgentCapabilityConfig::new("loop_detection"),
        AgentCapabilityConfig::new(PROMPT_CACHING_CAPABILITY_ID),
        AgentCapabilityConfig::new("duckduckgo"),
        // enable_file_download=true: saved responses land on disk through the
        // platform filesystem stack, so the blocklist and approval gate apply.
        AgentCapabilityConfig::with_config(
            "web_fetch",
            serde_json::json!({ "enable_file_download": true }),
        ),
        AgentCapabilityConfig::new("coding_cli_bash"),
    ];

    let mut builder = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .default_model(default_model)
        .backends(backends)
        .single_session(move |s| {
            let mut s = s
                .harness("coding-cli", HARNESS_PROMPT)
                .harness_display_name("Coding CLI")
                .harness_description("Embedded terminal coding agent.")
                .agent("coding-agent", AGENT_PROMPT)
                .agent_display_name("Coding Agent")
                .agent_description("Reads, edits, and runs commands inside a project workspace.")
                .agent_max_iterations(20)
                .session_title(session_title.clone())
                .tag("example")
                .tag("coding");
            for cap in harness_capabilities {
                s = s.harness_capability(cap);
            }
            s
        });
    builder = builder.llm_sim(
        LlmSimConfig::fixed(
            "I'm running in offline mode (llmsim — no API key set). \
             Set ANTHROPIC_API_KEY or OPENAI_API_KEY for real responses.",
        )
        .with_model("llmsim-coding-cli"),
    );
    let runtime = builder.build().await?;
    let session_id = runtime
        .default_session_id()
        .expect("single_session sets the default session id");

    let context = runtime.load_context(session_id).await?;
    let tool_names = context
        .runtime_agent
        .tools
        .iter()
        .map(|t| t.name().to_string())
        .collect();

    // Probe for AGENTS.md / CLAUDE.md / .agents.md on disk for the startup
    // banner only — the agent itself sees content via AgentInstructionsCapability,
    // which re-reads /AGENTS.md every turn through the session filesystem.
    let instruction_files = ["AGENTS.md", "CLAUDE.md", ".agents.md"]
        .iter()
        .filter(|f| canonical_root.join(f).exists())
        .map(|s| s.to_string())
        .collect();

    Ok(RuntimeBundle {
        runtime: Arc::new(runtime),
        session_id,
        workspace_root: canonical_root,
        instruction_files,
        tool_names,
        provider: RwLock::new(provider),
        provider_store,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_spec_can_switch_to_openai() {
        let provider = ProviderChoice::Sim;
        let next = provider.resolve_model_spec("openai/gpt-5.5").unwrap();

        assert_eq!(next.label(), "openai/gpt-5.5");
    }

    #[test]
    fn model_spec_can_switch_to_anthropic() {
        let provider = ProviderChoice::OpenAi {
            model: "gpt-5.5".to_string(),
        };
        let next = provider
            .resolve_model_spec("anthropic/claude-sonnet-4-5")
            .unwrap();

        assert_eq!(next.label(), "anthropic/claude-sonnet-4-5");
    }

    #[test]
    fn model_spec_uses_current_provider_without_prefix() {
        let provider = ProviderChoice::OpenAi {
            model: "gpt-5.5".to_string(),
        };
        let next = provider.resolve_model_spec("gpt-5.4").unwrap();

        assert_eq!(next.label(), "openai/gpt-5.4");
    }

    #[test]
    fn model_spec_accepts_llmsim_provider_name() {
        let provider = ProviderChoice::OpenAi {
            model: "gpt-5.5".to_string(),
        };
        let next = provider
            .resolve_model_spec("llmsim/llmsim-coding-cli")
            .unwrap();

        assert_eq!(next.label(), "llmsim/llmsim-coding-cli");
    }
}
