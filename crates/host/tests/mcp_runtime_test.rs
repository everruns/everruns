//! End-to-end MCP wiring through the real in-process runtime
//! (knowledge/integrations/runtime-mcp.md D4): a session's scoped MCP server is discovered at
//! `tools/list`, its tool appears to the (scripted) LLM, and the resulting
//! `mcp_*` tool call is routed back out as `tools/call`.
//!
//! A fake `EgressService` returns canned MCP responses so the test is offline.
//! A public IP-literal URL passes the runtime's DNS-pinned SSRF validation
//! without resolving (and the fake egress never actually connects).

use async_trait::async_trait;
use everruns_core::command::{CommandDescriptor, CommandSource};
use everruns_core::{
    Capability, CapabilityRegistry, CapabilityStatus, EgressRequest, EgressResponse, EgressResult,
    EgressService, EgressStreamResponse, ScopedMcpServer, ScopedMcpServers, Tool,
    ToolExecutionResult, tool_context::ToolContext,
};
use everruns_host::HostComposition;
use everruns_host::{AgentBuilder, HarnessBuilder, InProcessRuntimeBuilder, SessionBuilder};
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::provider::DriverId;
use everruns_provider::tool_types::{ToolCall, ToolResult};
use everruns_test_support::LlmSimRuntimeExt;
use everruns_test_support::llmsim_driver::{LlmSimConfig, SimToolCall, SimTurn};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A public, non-blocked IP literal: passes SSRF validation without DNS and is
/// never actually connected to because egress is faked.
const FAKE_URL: &str = "http://8.8.8.8/mcp";

#[derive(Default)]
struct McpTraffic {
    tools_list_calls: usize,
    tools_call_names: Vec<String>,
}

/// Fake egress that answers MCP JSON-RPC requests from canned data and records
/// what it was asked to do.
struct FakeMcpEgress {
    traffic: Arc<Mutex<McpTraffic>>,
}

#[async_trait]
impl EgressService for FakeMcpEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let method = body.get("method").and_then(Value::as_str).unwrap_or("");

        let result = match method {
            "tools/list" => {
                self.traffic.lock().unwrap().tools_list_calls += 1;
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": { "tools": [{
                        "name": "echo",
                        "description": "Echo a message",
                        "inputSchema": { "type": "object", "properties": { "message": { "type": "string" } } }
                    }]}
                })
            }
            "tools/call" => {
                let name = body
                    .get("params")
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.traffic.lock().unwrap().tools_call_names.push(name);
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": { "content": [{ "type": "text", "text": "mcp-says-hi" }], "isError": false }
                })
            }
            other => {
                json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32601, "message": format!("unexpected method: {other}") } })
            }
        };

        Ok(EgressResponse {
            status: 200,
            headers: Default::default(),
            body: serde_json::to_vec(&result).unwrap(),
        })
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        unimplemented!("not used by MCP transport")
    }
}

fn scoped_servers() -> ScopedMcpServers {
    let mut servers = ScopedMcpServers::default();
    servers.insert(
        "docs".to_string(),
        ScopedMcpServer {
            url: FAKE_URL.to_string(),
            ..Default::default()
        },
    );
    servers
}

fn platform_with_egress(traffic: Arc<Mutex<McpTraffic>>) -> HostComposition {
    HostComposition::builder()
        .capability_registry(CapabilityRegistry::new())
        .driver_registry(DriverRegistry::new())
        .egress_service(Arc::new(FakeMcpEgress { traffic }))
        .build()
}

#[derive(Default)]
struct HookCounts {
    pre: AtomicUsize,
    post: AtomicUsize,
}

struct LiveTool;

#[async_trait]
impl Tool for LiveTool {
    fn name(&self) -> &str {
        "live_echo"
    }

    fn description(&self) -> &str {
        "Echo a value from a live capability."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "value": { "type": "string" } },
            "required": ["value"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({ "echo": arguments["value"] }))
    }
}

struct CountingPreHook(Arc<HookCounts>);

#[async_trait]
impl everruns_core::tool_hooks::PreToolUseHook for CountingPreHook {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _tool_def: &everruns_provider::tool_types::ToolDefinition,
        _context: &ToolContext,
    ) -> everruns_core::tool_hooks::PreToolUseDecision {
        self.0.pre.fetch_add(1, Ordering::SeqCst);
        everruns_core::tool_hooks::PreToolUseDecision::Continue(tool_call)
    }
}

struct CountingPostHook(Arc<HookCounts>);

#[async_trait]
impl everruns_core::tool_hooks::PostToolExecHook for CountingPostHook {
    async fn after_exec(
        &self,
        _tool_call: &ToolCall,
        _tool_def: &everruns_provider::tool_types::ToolDefinition,
        _result: &mut ToolResult,
        _context: &ToolContext,
    ) {
        self.0.post.fetch_add(1, Ordering::SeqCst);
    }
}

struct LiveCapability {
    hooks: Arc<HookCounts>,
}

impl Capability for LiveCapability {
    fn id(&self) -> &str {
        "live_test"
    }

    fn name(&self) -> &str {
        "Live Test"
    }

    fn description(&self) -> &str {
        "Exercises every live capability surface."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some("LIVE_CAPABILITY_PROMPT")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(LiveTool)]
    }

    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn everruns_core::tool_hooks::PreToolUseHook>> {
        vec![Arc::new(CountingPreHook(self.hooks.clone()))]
    }

    fn post_tool_exec_hooks(&self) -> Vec<Arc<dyn everruns_core::tool_hooks::PostToolExecHook>> {
        vec![Arc::new(CountingPostHook(self.hooks.clone()))]
    }

    fn commands(&self) -> Vec<CommandDescriptor> {
        vec![CommandDescriptor {
            name: "live".to_string(),
            description: "Live command".to_string(),
            args: vec![],
            source: CommandSource::System,
        }]
    }

    fn mcp_servers(&self) -> ScopedMcpServers {
        scoped_servers()
    }

    fn validate_config(&self, config: &Value) -> std::result::Result<(), String> {
        if config.get("enabled").and_then(Value::as_bool) == Some(true) {
            Ok(())
        } else {
            Err("enabled must be true".to_string())
        }
    }
}

#[tokio::test]
async fn runtime_discovers_and_executes_scoped_mcp_tool() {
    let traffic = Arc::new(Mutex::new(McpTraffic::default()));
    let seed = 808u128;
    let harness_id = everruns_provider::typed_id::HarnessId::from_seed(seed);
    let agent_id = everruns_provider::typed_id::AgentId::from_seed(seed);
    let session_id = everruns_provider::typed_id::SessionId::from_seed(seed);

    let mcp_tool_call = ToolCall {
        id: "call_1".to_string(),
        name: "mcp_docs__echo".to_string(),
        arguments: json!({ "message": "hi" }),
    };

    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(platform_with_egress(traffic.clone()))
        .llm_sim(
            LlmSimConfig::fixed("Calling the docs MCP tool.")
                .with_tool_call_sequence(vec![vec![mcp_tool_call], vec![]]),
        )
        .default_model(ModelSpec::on((DriverId::LlmSim).as_str(), "llmsim-model"))
        .harness(
            HarnessBuilder::new("mcp", "You are an MCP test assistant.")
                .id(harness_id)
                .build(),
        )
        .agent(
            AgentBuilder::new("mcp-agent", "Use the MCP tools provided.")
                .id(agent_id)
                .max_iterations(8)
                .build(),
        )
        .session(
            SessionBuilder::new(harness_id)
                .id(session_id)
                .agent(agent_id)
                .mcp_servers(scoped_servers())
                .build(),
        )
        .build()
        .await
        .expect("runtime builds");

    let context = runtime
        .load_context(session_id)
        .await
        .expect("context inspection discovers MCP tools");
    assert!(
        context
            .runtime_agent
            .tools
            .iter()
            .any(|tool| tool.name() == "mcp_docs__echo"),
        "load_context must use the same MCP discovery path as execution"
    );

    let result = runtime
        .run_text_turn(session_id, "Use the docs MCP echo tool.")
        .await
        .expect("turn runs");
    assert!(result.success, "turn should succeed");

    let traffic = traffic.lock().unwrap();
    assert!(
        traffic.tools_list_calls >= 1,
        "scoped MCP server should be discovered via tools/list"
    );
    assert_eq!(
        traffic.tools_call_names,
        vec!["echo".to_string()],
        "the mcp_docs__echo call should be routed out as a tools/call for 'echo'"
    );
}

#[tokio::test]
async fn live_capability_activation_and_deactivation_refresh_every_surface() {
    let traffic = Arc::new(Mutex::new(McpTraffic::default()));
    let hooks = Arc::new(HookCounts::default());
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(LiveCapability {
        hooks: hooks.clone(),
    });
    let platform = HostComposition::builder()
        .capability_registry(capabilities)
        .driver_registry(DriverRegistry::new())
        .egress_service(Arc::new(FakeMcpEgress {
            traffic: traffic.clone(),
        }))
        .build();

    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(platform)
        .llm_sim(LlmSimConfig::scripted(vec![
            SimTurn::Assistant("before activation".to_string()),
            SimTurn::ToolCalls(vec![
                SimToolCall {
                    name: "live_echo".to_string(),
                    arguments: json!({ "value": "hello" }),
                    id: None,
                },
                SimToolCall {
                    name: "mcp_docs__echo".to_string(),
                    arguments: json!({ "message": "hi" }),
                    id: None,
                },
            ]),
            SimTurn::Assistant("after live tools".to_string()),
            SimTurn::Assistant("after deactivation".to_string()),
        ]))
        .single_session(|session| {
            session
                .harness("live", "Base prompt")
                .agent("live-agent", "Base agent prompt")
                .agent_max_iterations(8)
        })
        .build()
        .await
        .expect("runtime builds");
    let session_id = runtime.default_session_id().expect("session id");

    runtime
        .run_text_turn(session_id, "Before activation")
        .await
        .expect("initial turn runs");
    let before = runtime.load_context(session_id).await.unwrap();
    assert!(
        !before
            .runtime_agent
            .system_prompt
            .contains("LIVE_CAPABILITY_PROMPT")
    );
    assert!(
        !before
            .runtime_agent
            .tools
            .iter()
            .any(|tool| tool.name() == "live_echo")
    );
    assert!(runtime.list_commands(session_id).await.unwrap().is_empty());
    assert_eq!(traffic.lock().unwrap().tools_list_calls, 0);

    let invalid = runtime
        .activate_capability(
            session_id,
            everruns_capability::CapabilityRef::new("live_test"),
        )
        .await
        .expect_err("invalid config must fail before mutation");
    assert!(invalid.to_string().contains("enabled must be true"));

    let delta = runtime
        .activate_capability(
            session_id,
            everruns_capability::CapabilityRef::with_config(
                "live_test",
                json!({ "enabled": true }),
            ),
        )
        .await
        .expect("activation succeeds");
    assert!(delta.changed && delta.active && delta.surfaces_dirty);
    let duplicate = runtime
        .activate_capability(
            session_id,
            everruns_capability::CapabilityRef::with_config(
                "live_test",
                json!({ "enabled": true }),
            ),
        )
        .await
        .expect("activation is idempotent");
    assert!(!duplicate.changed && duplicate.active && !duplicate.surfaces_dirty);

    let active = runtime.load_context(session_id).await.unwrap();
    assert!(
        active
            .runtime_agent
            .system_prompt
            .contains("LIVE_CAPABILITY_PROMPT")
    );
    assert!(
        active
            .runtime_agent
            .tools
            .iter()
            .any(|tool| tool.name() == "live_echo")
    );
    assert_eq!(
        runtime.list_commands(session_id).await.unwrap()[0].name,
        "live"
    );

    let live_turn = runtime
        .run_text_turn(session_id, "Use the live tools")
        .await
        .expect("live turn runs");
    assert!(live_turn.success);
    assert_eq!(traffic.lock().unwrap().tools_call_names, vec!["echo"]);
    assert_eq!(hooks.pre.load(Ordering::SeqCst), 2);
    assert_eq!(hooks.post.load(Ordering::SeqCst), 2);

    let delta = runtime
        .deactivate_capability(session_id, "live_test")
        .await
        .expect("deactivation succeeds");
    assert!(delta.changed && !delta.active && delta.surfaces_dirty);
    let duplicate = runtime
        .deactivate_capability(session_id, "live_test")
        .await
        .expect("deactivation is idempotent");
    assert!(!duplicate.changed && !duplicate.active && !duplicate.surfaces_dirty);

    let inactive = runtime.load_context(session_id).await.unwrap();
    assert!(
        !inactive
            .runtime_agent
            .system_prompt
            .contains("LIVE_CAPABILITY_PROMPT")
    );
    assert!(
        !inactive
            .runtime_agent
            .tools
            .iter()
            .any(|tool| tool.name() == "live_echo")
    );
    assert!(runtime.list_commands(session_id).await.unwrap().is_empty());
    runtime
        .run_text_turn(session_id, "After deactivation")
        .await
        .expect("post-deactivation turn runs");
    assert_eq!(hooks.pre.load(Ordering::SeqCst), 2);
    assert_eq!(hooks.post.load(Ordering::SeqCst), 2);
    assert_eq!(traffic.lock().unwrap().tools_call_names, vec!["echo"]);

    let unknown = runtime
        .activate_capability(session_id, "does_not_exist")
        .await
        .expect_err("unknown capability must fail");
    assert!(unknown.to_string().contains("unknown capability"));
}
