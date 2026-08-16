// Demonstrates that the built-in `file_system` capability's tools
// (`write_file`, `read_file`, `list_directory`) operate against whichever
// `SessionFileSystem` the embedder plugs in — here, a `RealDiskFileStore`
// rooted at a temp directory. The agent's tool calls land on real disk.
//
// What this proves:
//   1. A `write_file` tool call routed through `FileSystemCapability` ends
//      up as actual bytes on the host filesystem (verified with std::fs).
//   2. A `read_file` tool call routed through the same capability reads
//      those bytes back.
//   3. No bespoke per-embedder filesystem code: the in-tree capability is
//      the only thing wired up.
//
// Run with:
//   cargo run -p everruns-host --example real_disk_file_system_tools

use everruns_host::HostComposition;
use everruns_llmsim::LlmSimRuntimeExt;
use std::sync::Arc;

use everruns_core::{
    AgentDefinition, CapabilityRegistry, ExecutionSession, HarnessDefinition, SessionExecutionState,
};
use everruns_host::{InProcessRuntimeBuilder, RealDiskSessionFileSystemFactory};
use everruns_integrations_filesystem::FileSystemCapability;
use everruns_llmsim::LlmSimConfig;
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::provider::DriverId;
use everruns_provider::tool_types::ToolCall;
use tempfile::TempDir;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Real workspace directory on disk.
    let workspace = TempDir::new()?;
    println!("workspace root: {}", workspace.path().display());

    // 2. Pre-seed a source file directly on disk so the agent can read it.
    std::fs::write(workspace.path().join("input.txt"), "hello from real disk")?;

    // 3. Register only the built-in FileSystemCapability — no custom tools.
    //    HostComposition selects the real-disk session filesystem.
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(FileSystemCapability);
    let platform = HostComposition::builder()
        .capability_registry(capabilities)
        .driver_registry(DriverRegistry::new())
        .session_file_system_factory(Arc::new(RealDiskSessionFileSystemFactory::new(
            workspace.path(),
        )))
        .build();

    // 4. Drive the agent with a fixed tool-call script via llmsim: read the
    //    seeded file, then write a new file. The runtime forwards every
    //    tool call through `ToolContext.file_store` to the real-disk store.
    let llmsim = LlmSimConfig::fixed("done").with_tool_call_sequence(vec![
        vec![ToolCall {
            id: "call_read_1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({ "path": "/workspace/input.txt" }),
        }],
        vec![ToolCall {
            id: "call_write_1".into(),
            name: "write_file".into(),
            arguments: serde_json::json!({
                "path": "/workspace/output.txt",
                "content": "hello back",
            }),
        }],
        vec![],
    ]);

    let harness_id = "harness_00000000000000000000000000000092".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000092".parse().unwrap();
    let session_id = "session_00000000000000000000000000000092".parse().unwrap();

    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(platform)
        .llm_sim_as_default(llmsim)
        .default_model(ModelSpec::on((DriverId::LlmSim).as_str(), "llmsim-model"))
        .harness(everruns_host::SeededHarness {
            id: harness_id,
            definition: HarnessDefinition {
                capabilities: vec![everruns_capability::CapabilityRef::new(
                    "session_file_system",
                )],
                ..HarnessDefinition::new("files", "Use the file_system tools.")
            },
        })
        .agent(AgentDefinition {
            display_name: Some("Files Agent".into()),
            max_iterations: Some(8),
            ..AgentDefinition::new(agent_id, "files-agent", "Use tools when needed.")
        })
        .session(ExecutionSession {
            id: session_id,
            workspace_id: everruns_provider::typed_id::WorkspaceId::from_uuid((session_id).uuid()),
            organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id,
            agent_id: Some(agent_id),
            title: Some("Files ExecutionSession".into()),
            goal: None,
            locale: None,
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
            status: SessionExecutionState::Started,
            usage: None,
            parent_session_id: None,
            forked_from_session_id: None,
            blueprint_id: None,
            blueprint_config: None,
        })
        .build()
        .await?;

    let result = runtime
        .run_text_turn(session_id, "Read input.txt then write output.txt.")
        .await?;
    println!(
        "turn done: success={} iterations={} response={:?}",
        result.success, result.iterations, result.response
    );

    // 6. Verify the write actually hit real disk. No FileStore call here —
    //    we go straight to std::fs to prove the bytes are on the host.
    let on_disk = std::fs::read_to_string(workspace.path().join("output.txt"))?;
    println!("output.txt on real disk: {on_disk:?}");
    assert_eq!(on_disk, "hello back");

    println!("\nok: file_system capability tool calls landed on real disk");
    Ok(())
}
