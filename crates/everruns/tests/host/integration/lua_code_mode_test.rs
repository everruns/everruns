//! Integration smoke test for the `lua_code_mode` capability over the public
//! in-process runtime.
//!
//! Verifies the end-to-end contract: tools eligible for code mode are removed
//! from the model-facing tool list, yet remain executable from inside a `lua`
//! script via `tools.<name>(args)`. The model is simulated, so the test is
//! deterministic and runs without credentials.
//!
//! Requires the `lua` feature (compiles the mlua engine):
//!   cargo test -p everruns --features lua --test host integration::lua_code_mode_test::

#![cfg(feature = "lua")]

use everruns_contracts::driver_registry::DriverRegistry;
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::provider::DriverId;
use everruns_contracts::runtime::Capability;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tool_hooks::{PreToolUseDecision, PreToolUseHook};
use everruns_contracts::tool_types::{ToolCall, ToolDefinition};
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::CapabilityRegistry;
use everruns_core::host::HostComposition;
use everruns_core::host::{AgentBuilder, HarnessBuilder, SessionBuilder};
use everruns_integrations::lua::{LuaCapability, LuaCodeModeCapability};
use everruns_llmsim::LlmSimRuntimeExt;
use everruns_llmsim::{LlmSimConfig, SimToolCall, SimTurn};
use everruns_test_support::TestMathCapability;
use std::sync::Arc;

const ORCHESTRATION_SCRIPT: &str = r#"
    local product = tools.multiply({ a = 6, b = 7 })       -- 42
    local total = tools.add({ a = product.result, b = 8 }) -- 50
    fs.write("/workspace/out.txt", string.format("%d", total.result))
    return total.result
"#;

fn platform() -> HostComposition {
    let mut caps = CapabilityRegistry::new();
    caps.register(LuaCapability);
    caps.register(LuaCodeModeCapability);
    caps.register(TestMathCapability);
    // Contributes its hook only to harnesses that select it.
    caps.register(DenyMultiplyCapability);

    let mut drivers = DriverRegistry::new();
    everruns_llmsim::register_driver(&mut drivers);

    HostComposition::new(caps, drivers)
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

    let runtime = everruns::batteries::runtime_builder()
        .host_composition(platform())
        .llm_sim_as_default(sim)
        .default_model(ModelSpec::on(
            (DriverId::LlmSim).as_str(),
            "llmsim-model".to_string(),
        ))
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

/// Blocks every `multiply` call, as a tool-specific user hook or guardrail
/// would.
struct DenyMultiplyHook;

#[async_trait::async_trait]
impl PreToolUseHook for DenyMultiplyHook {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        if tool_call.name == "multiply" {
            return PreToolUseDecision::Block {
                tool_call,
                reason: "multiply is denied".to_string(),
                user_message: None,
            };
        }
        PreToolUseDecision::Continue(tool_call)
    }
}

struct DenyMultiplyCapability;

impl Capability for DenyMultiplyCapability {
    fn id(&self) -> &str {
        "deny_multiply"
    }
    fn name(&self) -> &str {
        "Deny Multiply"
    }
    fn description(&self) -> &str {
        "Test capability: blocks the multiply tool."
    }
    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn PreToolUseHook>> {
        vec![Arc::new(DenyMultiplyHook)]
    }
}

/// THREAT[TM-LUA-009]: a call a pre-tool hook denies when made directly is
/// denied from a code-mode script too, and allowed calls still run (EVE-1210).
#[tokio::test]
async fn pre_tool_hook_denial_applies_to_code_mode_calls() {
    const SCRIPT: &str = r#"
        local ok, err = pcall(tools.multiply, { a = 6, b = 7 })
        local sum = tools.add({ a = 1, b = 2 })
        fs.write("/workspace/out.txt", string.format("%s|%s|%d", tostring(ok), tostring(err), sum.result))
        return sum.result
    "#;
    let harness_id = HarnessId::new();
    let agent_id = AgentId::new();
    let session_id = SessionId::new();

    let sim = LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "lua".to_string(),
            arguments: serde_json::json!({ "script": SCRIPT }),
            id: None,
        }]),
        SimTurn::Assistant("Done.".to_string()),
    ]);

    let harness = HarnessBuilder::new("code-mode", "Use the lua tool to act.")
        .id(harness_id)
        .capability("lua")
        .capability("lua_code_mode")
        .capability("test_math")
        .capability("deny_multiply")
        .build();
    let agent = AgentBuilder::new("agent", "Finish the task then stop.")
        .id(agent_id)
        .max_iterations(6)
        .build();
    let session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .build();

    let runtime = everruns::batteries::runtime_builder()
        .host_composition(platform())
        .llm_sim_as_default(sim)
        .default_model(ModelSpec::on(
            (DriverId::LlmSim).as_str(),
            "llmsim-model".to_string(),
        ))
        .harness(harness)
        .agent(agent)
        .session(session)
        .build()
        .await
        .expect("build runtime");

    let turn = runtime
        .run_text_turn(session_id, "Compute 6 * 7 and 1 + 2.")
        .await
        .expect("run turn");
    assert!(turn.success, "turn should succeed");

    let out = runtime
        .read_file(session_id, "/workspace/out.txt")
        .await
        .expect("read out.txt")
        .and_then(|f| f.content)
        .unwrap_or_default();
    assert!(
        out.starts_with("false|") && out.contains("multiply is denied"),
        "the denied multiply must not run from code mode: {out:?}"
    );
    assert!(
        out.ends_with("|3"),
        "the allowed add must still run: {out:?}"
    );
}
