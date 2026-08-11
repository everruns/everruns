//! Lua code-mode agent: non-essential tool calls routed through the Lua sandbox.
//!
//! This example builds an agent whose action tools (`add`, `subtract`,
//! `multiply`, `divide` from `test_math`) are **hidden** from the model's direct
//! tool list by the `lua_code_mode` capability. The agent therefore cannot call
//! them as ordinary tool calls — instead it drives them from inside a single
//! `lua` script via the code-mode `tools.<name>(args)` table.
//!
//! The model is simulated (`LlmSim`) so the example is deterministic and needs
//! no API key — it doubles as a CI smoke test. The simulated model emits one
//! `lua` tool call that orchestrates two hidden math tools and writes the result
//! to the workspace, then a final answer.
//!
//! Run it:
//!
//! ```text
//! cargo run -p everruns-host --example lua_code_mode_agent --features lua
//! ```
//!
//! Without the `lua` feature the host does not link the Lua integration, so the
//! example is gated behind it.

#[cfg(not(feature = "lua"))]
fn main() {
    eprintln!(
        "This example requires the `lua` feature:\n  \
         cargo run -p everruns-host --example lua_code_mode_agent --features lua"
    );
}

#[cfg(feature = "lua")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use everruns_core::driver_registry::DriverRegistry;
    use everruns_core::{
        AgentId, CapabilityRegistry, DriverId, HarnessId, PlatformDefinition, ResolvedModel,
        SessionId,
    };
    use everruns_host::{AgentBuilder, HarnessBuilder, InProcessRuntimeBuilder, SessionBuilder};
    use everruns_integrations_lua::{LuaCapability, LuaCodeModeCapability};
    use everruns_test_support::llmsim_driver::{LlmSimConfig, SimToolCall, SimTurn};
    use everruns_test_support::{LlmSimRuntimeExt, TestMathCapability};

    // The math tools the agent will orchestrate through Lua. `tools.multiply` /
    // `tools.add` etc. become available inside the script.
    const SCRIPT: &str = r#"
        local product = tools.multiply({ a = 6, b = 7 })   -- 42
        local total = tools.add({ a = product.result, b = 8 }) -- 50
        fs.write("/workspace/out.txt", string.format("%d", total.result))
        return total.result
    "#;

    // Build the platform: only the math + lua capabilities are registered.
    // `FileSystemCapability` (`session_file_system`) is deliberately NOT
    // registered. `lua` depends on it, but an unregistered dependency is
    // silently skipped, so no file tools exist here and the only model-visible
    // tool ends up being `lua`. (The in-memory runtime still supplies the file
    // store that `fs.*` writes to.) In a full platform where
    // `session_file_system` IS registered, its tools would appear too: the
    // read-only ones routed into code mode, destructive ones like `delete_file`
    // staying directly callable.
    let mut caps = CapabilityRegistry::new();
    caps.register(LuaCapability);
    caps.register(LuaCodeModeCapability);
    caps.register(TestMathCapability);

    let mut drivers = DriverRegistry::new();
    everruns_test_support::llmsim_driver::register_driver(&mut drivers);
    let platform = PlatformDefinition::new(caps, drivers);

    // Simulated model: turn 1 calls `lua` with the orchestration script; turn 2
    // returns the final answer. No real API call.
    let sim = LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "lua".to_string(),
            arguments: serde_json::json!({ "script": SCRIPT }),
            id: None,
        }]),
        SimTurn::Assistant("The result is 50 (6 * 7 + 8).".to_string()),
    ]);

    let harness_id = HarnessId::new();
    let agent_id = AgentId::new();
    let session_id = SessionId::new();

    let harness = HarnessBuilder::new(
        "code-mode",
        "You are an automation agent. Use the `lua` tool to do work; other tools \
         are available inside Lua via tools.<name>(args).",
    )
    .id(harness_id)
    .capability("lua")
    .capability("lua_code_mode")
    .capability("test_math")
    .build();

    let agent = AgentBuilder::new("code-mode-agent", "Complete the task, then stop.")
        .id(agent_id)
        .max_iterations(6)
        .build();

    let session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .build();

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .llm_sim(sim)
        .default_model(ResolvedModel {
            model: "llmsim-model".to_string(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".to_string()),
            base_url: None,
            provider_metadata: None,
        })
        .harness(harness)
        .agent(agent)
        .session(session)
        .build()
        .await?;

    // 1. Inspect the model-facing tool list: the math tools are hidden.
    let ctx = runtime.load_context(session_id).await?;
    let visible: Vec<&str> = ctx.runtime_agent.tools.iter().map(|t| t.name()).collect();
    println!("Model-visible tools: {visible:?}");
    assert!(visible.contains(&"lua"), "lua must stay directly callable");
    for hidden in ["add", "subtract", "multiply", "divide"] {
        assert!(
            !visible.contains(&hidden),
            "`{hidden}` should be hidden from the model and only reachable via Lua",
        );
    }

    // The hidden tools are still discoverable: their catalog is grafted onto the
    // `lua` tool description so a real model knows what `tools.*` to call.
    let lua_desc = ctx
        .runtime_agent
        .tools
        .iter()
        .find(|t| t.name() == "lua")
        .map(|t| t.description().to_string())
        .unwrap_or_default();
    println!("\n--- lua tool description (tail) ---");
    if let Some(idx) = lua_desc.find("These tools are available ONLY inside this script") {
        println!("{}", &lua_desc[idx..]);
    }
    assert!(
        lua_desc.contains("multiply(a: number, b: number)"),
        "lua description should advertise the hidden tools and their typed args",
    );

    // 2. Run the turn. The agent orchestrates the hidden tools inside Lua.
    let turn = runtime
        .run_text_turn(session_id, "Compute 6 * 7 + 8.")
        .await?;
    assert!(turn.success, "turn should succeed");

    // 3. The model never called a math tool directly — only `lua`.
    let messages = runtime.messages(session_id).await?;
    let mut direct_tool_names = Vec::new();
    for msg in &messages {
        for call in msg.tool_calls() {
            direct_tool_names.push(call.name.clone());
        }
    }
    println!("Tools the model called directly: {direct_tool_names:?}");
    assert!(
        direct_tool_names.iter().all(|n| n == "lua"),
        "the model must only call `lua` directly, got {direct_tool_names:?}",
    );

    // 4. The Lua script produced the answer in the workspace.
    let out = runtime
        .read_file(session_id, "/workspace/out.txt")
        .await?
        .and_then(|f| f.content)
        .unwrap_or_default();
    println!("/workspace/out.txt = {out:?}");
    assert!(out.contains("50"), "expected 50 in out.txt, got {out:?}");

    println!("\n✅ Lua code-mode agent: math tools hidden from the model, executed via Lua.");
    Ok(())
}
