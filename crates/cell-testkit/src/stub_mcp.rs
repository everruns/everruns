//! A stub MCP server the test node answers in process (data plane v2, step
//! 11.7): load runs give some agents an MCP server without a server to run.
//!
//! Decisions:
//! - The stub sits behind the agents' egress ([`StubMcpEgress`]), not on a
//!   port: the runtime's own transport refuses private addresses (SSRF
//!   protection), so an in-cluster server would be refused, and an
//!   exception for it would be one in production code. Everything else
//!   still goes out through the Framework's direct transport.
//! - Its one tool, `lookup`, answers after `CELL_STUB_MCP_MS` (20 ms by
//!   default), about a nearby server's own time, so an MCP call costs what
//!   the cell adds (discovery, credentials, the client) plus that.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use everruns_core::{
    EgressRequest, EgressResponse, EgressResult, EgressScope, EgressService, EgressStreamResponse,
    SandboxProxyCredential,
};
use serde_json::{Value, json};

/// The stub server's address, as an agent's `mcpServers` names it.
///
/// A public address literal, as the core MCP tests use: the MCP transport
/// checks a server's address (DNS pinning) before its request reaches the
/// egress, so a name that does not resolve, or resolves to a private
/// address, never gets here. Nothing ever connects to it.
pub const URL: &str = "https://93.184.216.34/stub-mcp";

/// Requests the stub answered, for tests.
pub static CALLS: AtomicU64 = AtomicU64::new(0);
/// `tools/call` requests the stub answered.
pub static TOOL_CALLS: AtomicU64 = AtomicU64::new(0);

/// Answers [`URL`] itself and sends everything else on to `next`.
pub struct StubMcpEgress {
    next: Arc<dyn EgressService>,
    delay: Duration,
}

impl StubMcpEgress {
    pub fn new(next: Arc<dyn EgressService>) -> Self {
        Self {
            next,
            delay: crate::env_ms("CELL_STUB_MCP_MS", 20),
        }
    }

    /// In front of the Framework's direct, policy-aware transport.
    pub fn over_runtime() -> Self {
        Self::new(everruns::batteries::runtime_egress_service())
    }

    async fn answer(&self, request: &EgressRequest) -> EgressResponse {
        CALLS.fetch_add(1, Ordering::Relaxed);
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let Some(id) = body.get("id").cloned() else {
            // A notification (`notifications/initialized`): accepted.
            return EgressResponse {
                status: 202,
                headers: Default::default(),
                body: Vec::new(),
            };
        };
        let result = match body["method"].as_str().unwrap_or_default() {
            "initialize" => json!({
                "protocolVersion": body["params"]["protocolVersion"]
                    .as_str()
                    .unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "stub", "version": "1.0.0" },
            }),
            "tools/list" => json!({
                "tools": [{
                    "name": "lookup",
                    "description": "Look a term up.",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "query": { "type": "string" } },
                        "required": ["query"],
                    },
                }]
            }),
            "tools/call" => {
                TOOL_CALLS.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(self.delay).await;
                let query = body["params"]["arguments"]["query"]
                    .as_str()
                    .unwrap_or_default();
                json!({
                    "content": [{ "type": "text", "text": format!("stub found {query}") }],
                    "isError": false,
                })
            }
            "ping" => json!({}),
            method => {
                return json_response(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("no method {method}") },
                }));
            }
        };
        json_response(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }
}

fn json_response(body: Value) -> EgressResponse {
    EgressResponse {
        status: 200,
        headers: [("content-type".to_string(), "application/json".to_string())]
            .into_iter()
            .collect(),
        body: body.to_string().into_bytes(),
    }
}

#[async_trait]
impl EgressService for StubMcpEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        if request.url == URL {
            return Ok(self.answer(&request).await);
        }
        self.next.send(request).await
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        if request.url == URL {
            let response = self.answer(&request).await;
            return Ok(EgressStreamResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(futures::stream::iter([Ok(response.body)])),
            });
        }
        self.next.send_stream(request).await
    }

    fn sandbox_proxy_credential(
        &self,
        scope: Option<&EgressScope>,
        valid_for: Duration,
    ) -> EgressResult<Option<SandboxProxyCredential>> {
        self.next.sandbox_proxy_credential(scope, valid_for)
    }
}
