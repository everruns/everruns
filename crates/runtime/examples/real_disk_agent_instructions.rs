// Demonstrates that `AgentInstructionsCapability` reads `AGENTS.md` from
// whichever `SessionFileStore` the embedder plugs in — here, a
// `RealDiskFileStore` rooted at a temp directory.
//
// What this proves:
//   1. Drop AGENTS.md on disk → it appears in the system prompt on the next
//      turn (no rebuild required).
//   2. Edit AGENTS.md on disk → the change appears on the next turn (the
//      capability re-reads every `load_context`).
//
// Run with:
//   cargo run -p everruns-runtime --example real_disk_agent_instructions

use std::sync::Arc;

use chrono::Utc;
use everruns_core::capabilities::AgentInstructionsCapability;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::memory::{
    InMemoryAgentStore, InMemoryEventEmitter, InMemoryHarnessStore, InMemoryLlmProviderStore,
    InMemoryMemoryStore, InMemoryMessageRetriever,
};
use everruns_core::{
    Agent, AgentCapabilityConfig, AgentStatus, CapabilityRegistry, Harness, HarnessStatus,
    LlmProviderType, ModelWithProvider, PlatformDefinition, Session, SessionStatus,
};
use everruns_runtime::{
    InMemorySessionStorageStore, InMemorySessionStore, InProcessRuntimeBuilder, RealDiskFileStore,
    RuntimeBackends,
};
use tempfile::TempDir;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Stand up a real workspace directory on disk.
    let workspace = TempDir::new()?;
    println!("workspace root: {}", workspace.path().display());

    // 2. Drop an initial AGENTS.md straight onto disk. No FileStore call.
    std::fs::write(
        workspace.path().join("AGENTS.md"),
        "## Project rules\nPrefer snake_case identifiers.",
    )?;

    // 3. Build a RuntimeBackends with the real-disk file_store. Everything
    //    else stays in memory — only the workspace surface is real.
    let event_emitter = InMemoryEventEmitter::new();
    let backends = RuntimeBackends {
        harness_store: Arc::new(InMemoryHarnessStore::new()),
        agent_store: Arc::new(InMemoryAgentStore::new()),
        session_store: Arc::new(InMemorySessionStore::new()),
        message_store: Arc::new(InMemoryMessageRetriever::new()),
        provider_store: Arc::new(InMemoryLlmProviderStore::new()),
        event_emitter: Arc::new(event_emitter.clone()),
        event_collector: Some(Arc::new(event_emitter)),
        file_store: Arc::new(RealDiskFileStore::new(workspace.path())?),
        storage_store: Arc::new(InMemorySessionStorageStore::new()),
        memory_store: Arc::new(InMemoryMemoryStore::new()),
    };

    // 4. Register AgentInstructionsCapability and a no-op math capability so
    //    the harness has something to assemble.
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(AgentInstructionsCapability);
    let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());

    let harness_id = "harness_00000000000000000000000000000091".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000091".parse().unwrap();
    let session_id = "session_00000000000000000000000000000091".parse().unwrap();

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .backends(backends)
        .llm_sim(LlmSimConfig::fixed("ack"))
        .default_model(ModelWithProvider {
            model: "llmsim-model".into(),
            provider_type: LlmProviderType::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
        })
        .harness(Harness {
            id: harness_id,
            name: "coding".into(),
            display_name: Some("Coding".into()),
            description: Some("Harness that respects AGENTS.md".into()),
            system_prompt: "You are a coding assistant.".into(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("agent_instructions")],
            initial_files: vec![],
            network_access: None,
            mcp_servers: Default::default(),
            is_built_in: false,
            status: HarnessStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
        })
        .agent(Agent {
            public_id: agent_id,
            internal_id: Uuid::nil(),
            name: "coding-agent".into(),
            display_name: Some("Coding Agent".into()),
            description: None,
            system_prompt: "Use tools when needed.".into(),
            default_model_id: None,
            default_version_id: None,
            forked_from_agent_id: None,
            forked_from_version_id: None,
            root_agent_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            max_iterations: Some(8),
            tools: vec![],
            mcp_servers: Default::default(),
            status: AgentStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        })
        .session(Session {
            id: session_id,
            organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id,
            agent_id: Some(agent_id),
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            title: Some("Coding Session".into()),
            locale: None,
            preview: None,
            output_preview: None,
            tags: vec![],
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
        })
        .build()
        .await?;

    // 5. Inspect the assembled context. The AgentInstructionsCapability
    //    reads AGENTS.md from RealDiskFileStore and folds it into the
    //    system prompt.
    let ctx = runtime.load_context(session_id).await?;
    println!("\n--- system prompt (initial) ---");
    println!("{}", ctx.runtime_agent.system_prompt);
    assert!(
        ctx.runtime_agent.system_prompt.contains("snake_case"),
        "AGENTS.md content should be in the system prompt"
    );

    // 6. Mutate the file on disk and reload — the change appears on the
    //    next turn without rebuilding anything.
    std::fs::write(
        workspace.path().join("AGENTS.md"),
        "## Project rules\nPrefer camelCase identifiers.",
    )?;

    let ctx = runtime.load_context(session_id).await?;
    println!("\n--- system prompt (after editing AGENTS.md on disk) ---");
    println!("{}", ctx.runtime_agent.system_prompt);
    assert!(
        ctx.runtime_agent.system_prompt.contains("camelCase"),
        "edited AGENTS.md should appear on the next turn"
    );

    println!("\nok: AGENTS.md flows from real disk through AgentInstructionsCapability");
    Ok(())
}
