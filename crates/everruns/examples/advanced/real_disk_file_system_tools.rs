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
// The agent is a real model (OpenAI `gpt-6.1-sol`) that decides to call the
// file tools from a plain-language request. Set `OPENAI_API_KEY`.
//
// Run with:
//   cargo run -p everruns --features openai --example real_disk_file_system_tools

use everruns_core::host::HostComposition;
use std::sync::Arc;

use everruns_contracts::driver_registry::DriverRegistry;
use everruns_contracts::model_spec::ModelSpec;
use everruns_core::host::RealDiskSessionFileSystemFactory;
use everruns_core::{
    AgentDefinition, CapabilityRegistry, ExecutionSession, HarnessDefinition, SessionExecutionState,
};
use everruns_integrations::filesystem::FileSystemCapability;
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

    // 4. A real model drives the agent. The runtime forwards every tool call
    //    it makes through `ToolContext.file_store` to the real-disk store.
    let harness_id = "harness_00000000000000000000000000000092".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000092".parse().unwrap();
    let session_id = "session_00000000000000000000000000000092".parse().unwrap();

    let runtime = everruns::batteries::runtime_builder()
        .host_composition(platform)
        .provider(everruns::OpenAI::from_env()?.into())
        .default_model(ModelSpec::on("openai", "gpt-6.1-sol"))
        .harness(everruns_core::host::SeededHarness {
            id: harness_id,
            definition: HarnessDefinition {
                capabilities: vec![everruns_contracts::CapabilityRef::new(
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
            workspace_id: everruns_contracts::typed_id::WorkspaceId::from_uuid((session_id).uuid()),
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
        .run_text_turn(
            session_id,
            "Read /workspace/input.txt and tell me what it says. Then create \
             /workspace/output.txt whose entire content is exactly: hello back",
        )
        .await?;
    println!(
        "turn done: success={} iterations={} response={:?}",
        result.success, result.iterations, result.response
    );

    // 5. The model reached the file through the capability's tools.
    let messages = runtime.messages(session_id).await?;
    let tool_calls: Vec<String> = messages
        .iter()
        .flat_map(|m| m.tool_calls())
        .map(|c| c.name.clone())
        .collect();
    println!("tool calls: {tool_calls:?}");
    assert!(
        tool_calls.iter().any(|n| n == "read_file"),
        "expected a read_file call, got {tool_calls:?}"
    );
    assert!(
        result.response.contains("hello from real disk"),
        "the answer should quote input.txt"
    );

    // 6. Verify the write actually hit real disk. No FileStore call here —
    //    we go straight to std::fs to prove the bytes are on the host.
    let on_disk = std::fs::read_to_string(workspace.path().join("output.txt"))?;
    println!("output.txt on real disk: {on_disk:?}");
    assert_eq!(on_disk.trim(), "hello back");

    println!("\nok: file_system capability tool calls landed on real disk");
    Ok(())
}
