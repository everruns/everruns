//! Integration smoke test for the `lua_code_mode` capability over the public
//! in-process runtime.
//!
//! Verifies the end-to-end contract: tools eligible for code mode are removed
//! from the model-facing tool list, yet remain executable from inside a `lua`
//! script via `tools.<name>(args)`. The model is simulated, so the test is
//! deterministic and runs without credentials.
//!
//! Requires the `lua` feature (compiles the mlua engine):
//!   cargo test -p everruns-host --features lua --test lua_code_mode_test

#![cfg(feature = "lua")]

use everruns_core::driver_registry::DriverRegistry;
use everruns_core::{
    AgentId, CapabilityRegistry, DriverId, HarnessId, PlatformDefinition, ResolvedModel, SessionId,
};
use everruns_host::{AgentBuilder, HarnessBuilder, InProcessRuntimeBuilder, SessionBuilder};
use everruns_integrations_lua::{LuaCapability, LuaCodeModeCapability};
use everruns_test_support::LlmSimRuntimeExt;
use everruns_test_support::TestMathCapability;
use everruns_test_support::llmsim_driver::{LlmSimConfig, SimToolCall, SimTurn};

const ORCHESTRATION_SCRIPT: &str = r#"
    local product = tools.multiply({ a = 6, b = 7 })       -- 42
    local total = tools.add({ a = product.result, b = 8 }) -- 50
    fs.write("/workspace/out.txt", string.format("%d", total.result))
    return total.result
"#;

fn platform() -> PlatformDefinition {
    let mut caps = CapabilityRegistry::new();
    caps.register(LuaCapability);
    caps.register(LuaCodeModeCapability);
    caps.register(TestMathCapability);

    let mut drivers = DriverRegistry::new();
    everruns_test_support::llmsim_driver::register_driver(&mut drivers);

    PlatformDefinition::new(caps, drivers)
}

#[tokio::test]
async fn hides_math_tools_but_runs_them_via_lua() {
    let harness_id = HarnessId::new();
    let agent_id = AgentId::new();
    let session_id = SessionId::new();

    let sim = LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "lua".to_string(),
            arguments: serde_json::json!({ "script": ORCHESTRATION_SCRIPT }),
            id: None,
        }]),
        SimTurn::Assistant("Done: 50.".to_string()),
    ]);

    let harness = HarnessBuilder::new("code-mode", "Use the lua tool to act.")
        .id(harness_id)
        .capability("lua")
        .capability("lua_code_mode")
        .capability("test_math")
        .build();
    let agent = AgentBuilder::new("agent", "Finish the task then stop.")
        .id(agent_id)
        .max_iterations(6)
        .build();
    let session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .build();

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform())
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
        .await
        .expect("build runtime");

    // The math tools are hidden from the model; `lua` stays directly callable.
    let ctx = runtime
        .load_context(session_id)
        .await
        .expect("load context");
    let visible: Vec<&str> = ctx.runtime_agent.tools.iter().map(|t| t.name()).collect();
    assert!(
        visible.contains(&"lua"),
        "lua should be visible: {visible:?}"
    );
    for hidden in ["add", "subtract", "multiply", "divide"] {
        assert!(
            !visible.contains(&hidden),
            "`{hidden}` should be hidden from the model: {visible:?}",
        );
    }

    // The hidden tools are advertised in the lua tool description so a real
    // model can discover them (their standalone schemas are gone).
    let lua_desc = ctx
        .runtime_agent
        .tools
        .iter()
        .find(|t| t.name() == "lua")
        .map(|t| t.description().to_string())
        .unwrap_or_default();
    assert!(
        lua_desc.contains("multiply(a: number, b: number)") && lua_desc.contains("tools.<name>"),
        "lua description should catalog the hidden tools with typed args: {lua_desc}",
    );

    // Run the turn; the agent orchestrates the hidden tools inside Lua.
    let turn = runtime
        .run_text_turn(session_id, "Compute 6 * 7 + 8.")
        .await
        .expect("run turn");
    assert!(turn.success, "turn should succeed");

    // The model only ever called `lua` directly.
    let messages = runtime.messages(session_id).await.expect("messages");
    let direct: Vec<String> = messages
        .iter()
        .flat_map(|m| m.tool_calls().into_iter().map(|c| c.name.clone()))
        .collect();
    assert!(!direct.is_empty(), "expected at least one tool call");
    assert!(
        direct.iter().all(|n| n == "lua"),
        "model must only call lua directly, got {direct:?}",
    );

    // Lua executed the hidden tools and wrote the result.
    let out = runtime
        .read_file(session_id, "/workspace/out.txt")
        .await
        .expect("read out.txt")
        .and_then(|f| f.content)
        .unwrap_or_default();
    assert!(out.contains("50"), "expected 50 in out.txt, got {out:?}");
}
