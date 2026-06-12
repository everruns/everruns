//! End-to-end MCP wiring through the real in-process runtime
//! (specs/runtime-mcp.md D4): a session's scoped MCP server is discovered at
//! `tools/list`, its tool appears to the (scripted) LLM, and the resulting
//! `mcp_*` tool call is routed back out as `tools/call`.
//!
//! A fake `EgressService` returns canned MCP responses so the test is offline.
//! A public IP-literal URL passes the runtime's DNS-pinned SSRF validation
//! without resolving (and the fake egress never actually connects).

use async_trait::async_trait;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::{
    CapabilityRegistry, EgressRequest, EgressResponse, EgressResult, EgressService,
    EgressStreamResponse, LlmProviderType, ModelWithProvider, PlatformDefinition, ScopedMcpServer,
    ScopedMcpServers, ToolCall,
};
use everruns_runtime::{AgentBuilder, HarnessBuilder, InProcessRuntimeBuilder, SessionBuilder};
use serde_json::{Value, json};
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

fn platform_with_egress(traffic: Arc<Mutex<McpTraffic>>) -> PlatformDefinition {
    PlatformDefinition::builder()
        .capability_registry(CapabilityRegistry::with_builtins())
        .driver_registry(DriverRegistry::new())
        .egress_service(Arc::new(FakeMcpEgress { traffic }))
        .build()
}

#[tokio::test]
async fn runtime_discovers_and_executes_scoped_mcp_tool() {
    let traffic = Arc::new(Mutex::new(McpTraffic::default()));
    let seed = 808u128;
    let harness_id = everruns_core::HarnessId::from_seed(seed);
    let agent_id = everruns_core::AgentId::from_seed(seed);
    let session_id = everruns_core::SessionId::from_seed(seed);

    let mcp_tool_call = ToolCall {
        id: "call_1".to_string(),
        name: "mcp_docs__echo".to_string(),
        arguments: json!({ "message": "hi" }),
    };

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform_with_egress(traffic.clone()))
        .llm_sim(
            LlmSimConfig::fixed("Calling the docs MCP tool.")
                .with_tool_call_sequence(vec![vec![mcp_tool_call], vec![]]),
        )
        .default_model(ModelWithProvider {
            model: "llmsim-model".into(),
            provider_type: LlmProviderType::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
        })
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
