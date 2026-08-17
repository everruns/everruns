//! HTTP transport integration tests.
//!
//! The happy paths drive a fake [`EgressService`] so they run offline and
//! still exercise the real transport: DNS-pinned SSRF validation (public
//! IP-literal URLs pass without DNS), auth-header forwarding, SSE/JSON
//! parsing, result mapping, and executor routing. Loopback URLs are used to
//! assert the SSRF block (wiremock binds loopback, which is correctly
//! rejected, so a real server can't be used for the happy path).

use async_trait::async_trait;
use everruns_core::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
    McpServerAuthMode,
};
use everruns_mcp::{
    McpClient, McpConnection, McpExecutor, McpSecretBinding, NoAuthProvider, StaticAuthProvider,
    StaticConnectionResolver,
};
use everruns_provider::tool_types::ToolCall;
use serde_json::json;
use std::sync::{Arc, Mutex};

/// Fake egress that records the last request and returns a canned body.
struct FakeEgress {
    body: Vec<u8>,
    status: u16,
    last_authorization: Arc<Mutex<Option<String>>>,
    last_body: Arc<Mutex<Vec<u8>>>,
}

impl FakeEgress {
    fn new(body: impl Into<Vec<u8>>) -> (Arc<Self>, Arc<Mutex<Option<String>>>) {
        let last_authorization = Arc::new(Mutex::new(None));
        let egress = Arc::new(Self {
            body: body.into(),
            status: 200,
            last_authorization: last_authorization.clone(),
            last_body: Arc::new(Mutex::new(Vec::new())),
        });
        (egress, last_authorization)
    }

    fn with_status(body: impl Into<Vec<u8>>, status: u16) -> Arc<Self> {
        Arc::new(Self {
            body: body.into(),
            status,
            last_authorization: Arc::new(Mutex::new(None)),
            last_body: Arc::new(Mutex::new(Vec::new())),
        })
    }
}

fn client_with_fake_egress() -> McpClient {
    McpClient::new(
        FakeEgress::with_status(Vec::new(), 500),
        Arc::new(NoAuthProvider),
    )
}

#[async_trait]
impl EgressService for FakeEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        *self.last_authorization.lock().unwrap() = request.headers.get("Authorization").cloned();
        *self.last_body.lock().unwrap() = request.body;
        Ok(EgressResponse {
            status: self.status,
            headers: Default::default(),
            body: self.body.clone(),
        })
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        unimplemented!("not used by MCP transport")
    }
}

fn executor_with_bound_secret(egress: Arc<dyn EgressService>, secret: &str) -> McpExecutor {
    let client = Arc::new(McpClient::new(
        egress,
        Arc::new(everruns_mcp::NoAuthProvider),
    ));
    let mut connection = McpConnection::http("visti", FAKE_URL);
    connection.secret_bindings.insert(
        "visti_send".to_string(),
        vec![McpSecretBinding {
            parameter_name: "channel_key".to_string(),
            value: Some(secret.to_string()),
            setup_url: "/agents/agent_test?tab=credentials".to_string(),
            label: "Visti channel key".to_string(),
        }],
    );
    McpExecutor::new(
        client,
        Arc::new(StaticConnectionResolver::new().with(connection)),
    )
}

fn bound_tool_call() -> ToolCall {
    ToolCall {
        id: "call_bound".into(),
        name: "mcp_visti__visti_send".into(),
        arguments: json!({ "message": "controlled test" }),
    }
}

#[tokio::test]
async fn executor_injects_bound_credential_only_at_egress_boundary() {
    let sentinel = "test-secret-never-model-visible";
    let (egress, _) = FakeEgress::new(
        serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "content": [{ "type": "text", "text": "sent" }], "isError": false }
        }))
        .unwrap(),
    );
    let captured_body = egress.last_body.clone();
    let client = Arc::new(McpClient::new(
        egress,
        Arc::new(everruns_mcp::NoAuthProvider),
    ));
    let mut connection = McpConnection::http("visti", FAKE_URL);
    connection.secret_bindings.insert(
        "visti_send".to_string(),
        vec![McpSecretBinding {
            parameter_name: "channel_key".to_string(),
            value: Some(sentinel.to_string()),
            setup_url: "/agents/agent_test?tab=credentials".to_string(),
            label: "Visti channel key".to_string(),
        }],
    );
    let executor = McpExecutor::new(
        client,
        Arc::new(StaticConnectionResolver::new().with(connection)),
    );
    let tool_call = ToolCall {
        id: "call_bound".into(),
        name: "mcp_visti__visti_send".into(),
        arguments: json!({ "message": "controlled test" }),
    };

    let result = executor.execute_mcp_tool(&tool_call).await.unwrap();

    assert!(!tool_call.arguments.to_string().contains(sentinel));
    assert!(!serde_json::to_string(&result).unwrap().contains(sentinel));
    let outbound: serde_json::Value =
        serde_json::from_slice(&captured_body.lock().unwrap()).unwrap();
    assert_eq!(outbound["params"]["arguments"]["channel_key"], sentinel);
}

#[tokio::test]
async fn executor_redacts_bound_credential_reflected_in_success_result() {
    let sentinel = "reflected-success-secret";
    let (egress, _) = FakeEgress::new(
        serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "content": [{ "type": "text", "text": format!("echo: {sentinel}") }],
                "isError": false
            }
        }))
        .unwrap(),
    );
    let executor = executor_with_bound_secret(egress, sentinel);

    let result = executor.execute_mcp_tool(&bound_tool_call()).await.unwrap();
    let serialized = serde_json::to_string(&result).unwrap();

    assert!(!serialized.contains(sentinel));
    assert!(serialized.contains("REDACTED MCP CREDENTIAL"));
}

#[tokio::test]
async fn executor_redacts_bound_credential_reflected_in_json_rpc_error() {
    let sentinel = "reflected-json-rpc-secret";
    let (egress, _) = FakeEgress::new(
        serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32000, "message": format!("rejected {sentinel}") }
        }))
        .unwrap(),
    );
    let executor = executor_with_bound_secret(egress, sentinel);

    let error = executor
        .execute_mcp_tool(&bound_tool_call())
        .await
        .unwrap_err()
        .to_string();

    assert!(!error.contains(sentinel));
    assert!(error.contains("REDACTED MCP CREDENTIAL"));
}

#[tokio::test]
async fn executor_redacts_bound_credential_reflected_in_http_error_body() {
    let sentinel = "reflected-http-secret";
    let egress = FakeEgress::with_status(format!("request contained {sentinel}"), 500);
    let executor = executor_with_bound_secret(egress, sentinel);

    let error = executor
        .execute_mcp_tool(&bound_tool_call())
        .await
        .unwrap_err()
        .to_string();

    assert!(!error.contains(sentinel));
    assert!(error.contains("REDACTED MCP CREDENTIAL"));
}

#[tokio::test]
async fn missing_bound_credential_returns_structured_safe_setup_result() {
    let mut connection = McpConnection::http("visti", FAKE_URL);
    connection.secret_bindings.insert(
        "visti_send".to_string(),
        vec![McpSecretBinding {
            parameter_name: "channel_key".to_string(),
            value: None,
            setup_url: "/agents/agent_test?tab=credentials".to_string(),
            label: "Visti channel key".to_string(),
        }],
    );
    let executor = McpExecutor::new(
        Arc::new(client_with_fake_egress()),
        Arc::new(StaticConnectionResolver::new().with(connection)),
    );
    let result = executor
        .execute_mcp_tool(&ToolCall {
            id: "call_missing".into(),
            name: "mcp_visti__visti_send".into(),
            arguments: json!({ "message": "test" }),
        })
        .await
        .unwrap();

    assert_eq!(
        result.result.as_ref().unwrap()["code"],
        "credential_required"
    );
    assert_eq!(
        result.result.as_ref().unwrap()["setup_url"],
        "/agents/agent_test?tab=credentials"
    );
    assert!(result.error.is_none());
}

#[tokio::test]
async fn model_cannot_override_bound_credential() {
    let mut connection = McpConnection::http("visti", FAKE_URL);
    connection.secret_bindings.insert(
        "visti_send".to_string(),
        vec![McpSecretBinding {
            parameter_name: "channel_key".to_string(),
            value: Some("server-value".to_string()),
            setup_url: "/agents/agent_test?tab=credentials".to_string(),
            label: "Visti channel key".to_string(),
        }],
    );
    let executor = McpExecutor::new(
        Arc::new(client_with_fake_egress()),
        Arc::new(StaticConnectionResolver::new().with(connection)),
    );
    let result = executor
        .execute_mcp_tool(&ToolCall {
            id: "call_override".into(),
            name: "mcp_visti__visti_send".into(),
            arguments: json!({ "message": "test", "channel_key": "model-value" }),
        })
        .await
        .unwrap();

    assert_eq!(
        result.result.as_ref().unwrap()["code"],
        "credential_override_rejected"
    );
}

#[test]
fn credential_binding_debug_output_is_redacted() {
    let binding = McpSecretBinding {
        parameter_name: "channel_key".to_string(),
        value: Some("debug-sentinel-secret".to_string()),
        setup_url: "/agents/agent_test?tab=credentials".to_string(),
        label: "Visti channel key".to_string(),
    };

    let rendered = format!("{binding:?}");
    assert!(rendered.contains("configured: true"));
    assert!(!rendered.contains("debug-sentinel-secret"));
}

// A public, non-blocked IP literal: passes SSRF validation without DNS and is
// never actually connected to because egress is faked.
const FAKE_URL: &str = "http://8.8.8.8/mcp";

#[tokio::test]
async fn discovers_tools_over_http_plain_json() {
    let (egress, _) = FakeEgress::new(
        serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "tools": [
                { "name": "search", "description": "Search docs", "inputSchema": { "type": "object" } }
            ]}
        }))
        .unwrap(),
    );

    let client = McpClient::new(egress, Arc::new(everruns_mcp::NoAuthProvider));
    let tools = client
        .discover(&McpConnection::http("docs", FAKE_URL))
        .await
        .unwrap();

    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "search");
}

#[tokio::test]
async fn calls_tool_over_sse_with_resolved_auth() {
    let (egress, last_auth) = FakeEgress::new(
        "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}],\"isError\":false}}\n",
    );

    let auth = Arc::new(StaticAuthProvider::new().with_bearer("docs", "t0ken"));
    let client = McpClient::new(egress, auth);
    let mut connection = McpConnection::http("docs", FAKE_URL);
    connection.auth_mode = McpServerAuthMode::ApiKey;

    let result = client.call(&connection, "echo", json!({})).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(
        last_auth.lock().unwrap().as_deref(),
        Some("Bearer t0ken"),
        "resolved credential must reach the transport"
    );
}

#[tokio::test]
async fn executor_routes_and_maps_mcp_tool_call() {
    let (egress, _) = FakeEgress::new(
        serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "content": [{ "type": "text", "text": "routed" }], "isError": false }
        }))
        .unwrap(),
    );

    let client = Arc::new(McpClient::new(
        egress,
        Arc::new(everruns_mcp::NoAuthProvider),
    ));
    let resolver =
        Arc::new(StaticConnectionResolver::new().with(McpConnection::http("docs", FAKE_URL)));
    let executor = McpExecutor::new(client, resolver);

    let tool_call = ToolCall {
        id: "call_1".into(),
        name: "mcp_docs__echo".into(),
        arguments: json!({ "message": "hi" }),
    };
    let result = executor.execute_mcp_tool(&tool_call).await.unwrap();

    assert_eq!(result.tool_call_id, "call_1");
    assert_eq!(result.result.unwrap(), json!({ "result": "routed" }));
}

#[tokio::test]
async fn ssrf_blocks_localhost_discovery() {
    // URL validation must reject this before the injected egress is called.
    let client = client_with_fake_egress();
    let connection = McpConnection::http("evil", "http://localhost:9999/mcp");
    let error = client.discover(&connection).await.unwrap_err();
    assert!(
        error.to_string().contains("blocked"),
        "expected SSRF block, got: {error}"
    );
}

#[tokio::test]
async fn unresolved_server_prefix_errors() {
    let executor = McpExecutor::new(
        Arc::new(client_with_fake_egress()),
        Arc::new(StaticConnectionResolver::new()),
    );
    let tool_call = ToolCall {
        id: "call_1".into(),
        name: "mcp_missing__tool".into(),
        arguments: json!({}),
    };
    let error = executor.execute_mcp_tool(&tool_call).await.unwrap_err();
    assert!(error.to_string().contains("MCP server not found"));
}
