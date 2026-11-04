//! End-to-end discovery and bounded tool selection from a paginated MCP catalog.

use async_trait::async_trait;
use everruns_contracts::CapabilityRef;
use everruns_contracts::driver_registry::DriverRegistry;
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::provider::DriverId;
use everruns_contracts::runtime::mcp_server::{ScopedMcpServer, ScopedMcpServers};
use everruns_core::builtins::ToolSearchCapability;
use everruns_core::host::{AgentBuilder, HarnessBuilder, HostComposition, SessionBuilder};
use everruns_core::{
    CapabilityRegistry, EgressRequest, EgressResponse, EgressResult, EgressService,
    EgressStreamResponse,
};
use everruns_llmsim::LlmSimRuntimeExt;
use everruns_llmsim::{LlmSimConfig, SimToolCall, SimTurn};
use futures::stream;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

const FAKE_URL: &str = "http://8.8.8.8/mcp";
const TOOL_COUNT: usize = 2_000;
const PAGE_SIZE: usize = 100;

#[derive(Default)]
struct Traffic {
    list_cursors: Vec<Option<String>>,
    tool_calls: Vec<(String, Value)>,
}

struct CatalogEgress(Arc<Mutex<Traffic>>);

#[async_trait]
impl EgressService for CatalogEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let result = match body["method"].as_str().unwrap_or("") {
            "tools/list" => {
                let cursor = body["params"]["cursor"].as_str().map(str::to_owned);
                let page = cursor
                    .as_deref()
                    .map(|value| value.parse::<usize>().expect("page cursor"))
                    .unwrap_or(0);
                self.0.lock().unwrap().list_cursors.push(cursor);
                let start = page * PAGE_SIZE;
                let end = (start + PAGE_SIZE).min(TOOL_COUNT);
                let tools: Vec<_> = (start..end)
                    .map(|index| {
                        json!({
                            "name": format!("search_{index:04}"),
                            "description": format!("Search documents by stable identifier {index}."),
                            "inputSchema": {
                                "type": "object",
                                "properties": { "id": { "type": "string" } },
                                "required": ["id"],
                                "additionalProperties": false
                            }
                        })
                    })
                    .collect();
                let next_cursor = (end < TOOL_COUNT).then(|| (page + 1).to_string());
                json!({
                    "jsonrpc":"2.0",
                    "id":1,
                    "result": {
                        "tools": tools,
                        "nextCursor": next_cursor
                    }
                })
            }
            "tools/call" => {
                let params = &body["params"];
                let name = params["name"].as_str().unwrap_or("").to_owned();
                let arguments = params["arguments"].clone();
                self.0
                    .lock()
                    .unwrap()
                    .tool_calls
                    .push((name.clone(), arguments.clone()));
                json!({
                    "jsonrpc":"2.0",
                    "id":1,
                    "result": {
                        "content": [{"type":"text", "text":format!("called {name}")}],
                        "isError":false
                    }
                })
            }
            other => json!({
                "jsonrpc":"2.0",
                "id":1,
                "error":{"code":-32601,"message":format!("unexpected method: {other}")}
            }),
        };
        Ok(EgressResponse {
            status: 200,
            headers: Default::default(),
            body: serde_json::to_vec(&result).unwrap(),
        })
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        let response = self.send(_request).await?;
        Ok(EgressStreamResponse {
            status: response.status,
            headers: response.headers,
            body: Box::pin(stream::iter(vec![Ok(response.body)])),
        })
    }
}

fn large_deferred_server() -> ScopedMcpServers {
    let mut servers = ScopedMcpServers::default();
    servers.insert(
        "docs".to_string(),
        ScopedMcpServer {
            url: FAKE_URL.to_string(),
            description: Some("Internal product documentation and release notes".into()),
            deferred: true,
            ..Default::default()
        },
    );
    servers
}

#[tokio::test]
async fn search_reveals_one_tool_from_a_complete_large_mcp_catalog() {
    let traffic = Arc::new(Mutex::new(Traffic::default()));
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(ToolSearchCapability::with_threshold(1));
    let platform = HostComposition::builder()
        .capability_registry(capabilities)
        .driver_registry(DriverRegistry::new())
        .egress_service(Arc::new(CatalogEgress(traffic.clone())))
        .build();

    let seed = 2914;
    let harness_id = everruns_contracts::typed_id::HarnessId::from_seed(seed);
    let agent_id = everruns_contracts::typed_id::AgentId::from_seed(seed);
    let session_id = everruns_contracts::typed_id::SessionId::from_seed(seed);
    let target = "mcp_docs__search_1999";
    let runtime = everruns::batteries::runtime_builder()
        .host_composition(platform)
        .llm_sim_as_default(LlmSimConfig::scripted(vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "tool_search".into(),
                arguments: json!({"query":"internal product documentation"}),
                id: None,
            }]),
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "tool_search".into(),
                arguments: json!({"query":"search_1999"}),
                id: None,
            }]),
            SimTurn::ToolCalls(vec![SimToolCall {
                name: target.into(),
                arguments: json!({"id":"doc-1999"}),
                id: None,
            }]),
            SimTurn::Assistant("Found the document.".into()),
        ]))
        .default_model(ModelSpec::on((DriverId::LlmSim).as_str(), "llmsim-model"))
        .harness(
            HarnessBuilder::new("mcp catalog", "Use tools to search the docs.")
                .id(harness_id)
                .build(),
        )
        .agent(
            AgentBuilder::new("mcp catalog agent", "Search the requested catalog.")
                .id(agent_id)
                .max_iterations(8)
                .build(),
        )
        .session(
            SessionBuilder::new(harness_id)
                .id(session_id)
                .agent(agent_id)
                .mcp_servers(large_deferred_server())
                .capability(CapabilityRef::with_config(
                    "tool_search",
                    json!({
                        "bounded": true,
                        "threshold": 1,
                        "max_active_tools": 4,
                        "max_active_bytes": 16384,
                        "max_result_bytes": 8192
                    }),
                ))
                .build(),
        )
        .build()
        .await
        .expect("runtime builds");

    let initial = runtime
        .load_context(session_id)
        .await
        .expect("initial context loads");
    assert!(
        initial
            .runtime_agent
            .tools
            .iter()
            .any(|tool| tool.name() == "tool_search")
    );
    assert!(
        initial
            .runtime_agent
            .catalog_tools
            .iter()
            .any(|tool| tool.name() == "mcp_docs"),
        "the deferred server is authorized and searchable before it is revealed"
    );
    assert!(
        initial
            .runtime_agent
            .tools
            .iter()
            .all(|tool| tool.name() != "mcp_docs"),
        "bounded native projection keeps the deferred server hidden until search"
    );
    assert_eq!(traffic.lock().unwrap().list_cursors.len(), 0);

    let turn = runtime
        .run_text_turn(session_id, "Find document doc-1999")
        .await
        .expect("bounded search turn runs");
    assert!(turn.success, "turn failed: {:?}", turn.error);
    let after_search = runtime
        .load_context(session_id)
        .await
        .expect("context after search loads");
    let complete_catalog: HashSet<_> = after_search
        .runtime_agent
        .catalog_tools
        .iter()
        .map(|tool| tool.name().to_string())
        .chain(
            after_search
                .runtime_agent
                .tools
                .iter()
                .filter(|tool| tool.name().starts_with("mcp_docs__"))
                .map(|tool| tool.name().to_string()),
        )
        .collect();
    assert_eq!(
        complete_catalog.len(),
        TOOL_COUNT,
        "promoted operations remain available while the rest of the complete catalog stays searchable"
    );
    assert!(
        after_search
            .runtime_agent
            .tools
            .iter()
            .any(|tool| tool.name() == target),
        "the selected MCP operation becomes available"
    );
    assert!(
        after_search.runtime_agent.tools.len() <= 5,
        "the advertised working set stays within the configured operation plus search-tool bounds"
    );
    assert_eq!(
        after_search
            .runtime_agent
            .tools
            .iter()
            .find(|tool| tool.name() == target)
            .unwrap()
            .parameters()["properties"]["id"]["type"],
        "string"
    );
    let traffic = traffic.lock().unwrap();
    assert_eq!(traffic.list_cursors.len(), TOOL_COUNT / PAGE_SIZE);
    assert_eq!(
        traffic.list_cursors.first().cloned().flatten(),
        None,
        "the first tools/list page has no cursor"
    );
    assert_eq!(
        traffic.list_cursors.last().cloned().flatten().as_deref(),
        Some("19")
    );
    assert_eq!(
        traffic.tool_calls,
        vec![("search_1999".into(), json!({"id":"doc-1999"}))]
    );
}
