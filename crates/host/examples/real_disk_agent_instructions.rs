// Demonstrates that `AgentInstructionsCapability` reads `AGENTS.md` from
// whichever `SessionFileSystem` the embedder plugs in — here, a
// `RealDiskFileStore` rooted at a temp directory.
//
// What this proves:
//   1. Drop AGENTS.md on disk → it appears in the system prompt on the next
//      turn (no rebuild required).
//   2. Edit AGENTS.md on disk → the change appears on the next turn (the
//      capability re-reads every `load_context`).
//
// Run with:
//   cargo run -p everruns-host --example real_disk_agent_instructions

use everruns_test_support::LlmSimRuntimeExt;
use std::sync::Arc;

use chrono::Utc;
use everruns_core::capabilities::AgentInstructionsCapability;
use everruns_core::driver_registry::DriverRegistry;
use everruns_core::{
    AgentCapabilityConfig, AgentDefinition, CapabilityRegistry, DriverId, HarnessDefinition,
    PlatformDefinition, ResolvedModel, Session, SessionStatus,
};
use everruns_host::{InProcessRuntimeBuilder, RealDiskSessionFileSystemFactory};
use everruns_test_support::llmsim_driver::LlmSimConfig;
use tempfile::TempDir;

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

    // 3. Register AgentInstructionsCapability and configure the platform
    //    filesystem factory so only the workspace surface is real.
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(AgentInstructionsCapability);
    let platform = PlatformDefinition::builder()
        .capability_registry(capabilities)
        .driver_registry(DriverRegistry::new())
        .session_file_system_factory(Arc::new(RealDiskSessionFileSystemFactory::new(
            workspace.path(),
        )))
        .build();

    let harness_id = "harness_00000000000000000000000000000091".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000091".parse().unwrap();
    let session_id = "session_00000000000000000000000000000091".parse().unwrap();

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .llm_sim(LlmSimConfig::fixed("ack"))
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
        })
        .harness(everruns_host::SeededHarness {
            id: harness_id,
            definition: HarnessDefinition {
                capabilities: vec![AgentCapabilityConfig::new("agent_instructions")],
                ..HarnessDefinition::new("coding", "You are a coding assistant.")
            },
        })
        .agent(AgentDefinition {
            display_name: Some("Coding Agent".into()),
            max_iterations: Some(8),
            ..AgentDefinition::new(agent_id, "coding-agent", "Use tools when needed.")
        })
        .session(Session {
            source: Default::default(),
            activity: Default::default(),
            id: session_id,
            workspace_id: everruns_core::WorkspaceId::from_uuid((session_id).uuid()),
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
            goal: None,
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
            parallel_tool_calls: None,
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
            forked_from_session_id: None,
            forked_from_sequence: None,
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
