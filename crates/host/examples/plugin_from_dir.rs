// Load a plugin from a local directory and inspect the assembled context.
//
// This example demonstrates `InProcessRuntimeBuilder::with_plugin_dir`:
//   - compiles the testdata/plugins/microsoft-docs fixture
//   - builds a runtime with a single session whose agent enables the plugin
//   - calls `load_context` to inspect the result without executing a real turn
//
// Run with:
//   cargo run -p everruns-host --example plugin_from_dir

use everruns_host::InProcessRuntimeBuilder;
use everruns_test_support::LlmSimRuntimeExt;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load the bundled fixture plugin. In a real embedder this would point at
    // a checked-out plugin directory on disk.
    let plugin_dir = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../testdata/plugins/microsoft-docs"
    ));

    // Load and compile the plugin. Compilation errors surface here.
    let builder = InProcessRuntimeBuilder::new()
        .llm_sim(everruns_test_support::llmsim_driver::LlmSimConfig::fixed(
            "ok",
        ))
        .with_plugin_dir(plugin_dir)?;

    // Retrieve the hydrated capability config so we can attach it to the agent.
    let plugin_cap = builder
        .plugin_capability("microsoft-docs")
        .expect("plugin must be available after with_plugin_dir");

    println!("Loaded plugin: {}", plugin_cap.capability_id());

    // Build the runtime with a single session. The agent explicitly enables the
    // plugin by including the hydrated CapabilityRef.
    let runtime = builder
        .single_session(|s| {
            s.harness("example-harness", "You are an example harness.")
                .agent("example-agent", "Use docs when needed.")
                // Pass the full hydrated config so capability resolution can
                // deserialise the compiled DeclarativeCapabilityDefinition.
                .agent_capability(plugin_cap)
        })
        .build()
        .await?;

    // Non-fatal compile warnings (e.g. unsupported manifest fields) are
    // available on the built runtime.
    let warnings = runtime.plugin_warnings();
    if warnings.is_empty() {
        println!("No plugin compile warnings.");
    } else {
        for w in warnings {
            println!("Plugin warning: {w}");
        }
    }

    // Inspect the assembled context without running a turn.
    let session_id = runtime
        .default_session_id()
        .expect("single_session sets the default session id");
    let ctx = runtime.load_context(session_id).await?;

    // The system prompt should contain the docs-researcher agent section.
    let has_agent = ctx.runtime_agent.system_prompt.contains("docs-researcher");
    println!("System prompt contains docs-researcher agent: {has_agent}");

    // The plugin capability config should appear in the resolved set.
    let plugin_config = ctx
        .resolved_capability_configs
        .iter()
        .find(|c| c.capability_id() == "plugin:microsoft-docs");
    println!(
        "plugin:microsoft-docs in resolved configs: {}",
        plugin_config.is_some()
    );

    // The microsoft-learn MCP server should be declared in the definition.
    if let Some(cap) = plugin_config
        && let Ok(def) = serde_json::from_value::<everruns_core::DeclarativeCapabilityDefinition>(
            cap.config_value().clone(),
        )
    {
        if let Some(servers) = &def.mcp_servers
            && let Some(server) = servers.get("microsoft-learn")
        {
            println!("microsoft-learn MCP server URL: {}", server.url);
        }
        println!(
            "Plugin skills: {}",
            def.skills
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(())
}
