//! Multi-era protocol negotiation integration tests (specs/mcp-servers.md
//! "Multi-era protocol support").
//!
//! A scripted [`EgressService`] stands in for three kinds of MCP server:
//!
//! - a *stateless* RC server that answers operations directly,
//! - a *stateful* legacy/current server that rejects session-less requests and
//!   requires the `initialize` handshake + `Mcp-Session-Id`,
//!
//! and asserts that `Auto` adapts to each, that pinned modes skip/force the
//! handshake, and that `_meta` + routable headers ride on every request.

use async_trait::async_trait;
use everruns_core::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
    McpProtocolMode,
};
use everruns_mcp::{McpClient, McpConnection, NoAuthProvider};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// One observed request, distilled to what the assertions care about.
#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    has_session: bool,
    has_meta: bool,
    protocol_header: Option<String>,
    method_header: Option<String>,
}

/// Whether the scripted server behaves statelessly (RC) or demands a handshake.
#[derive(Clone, Copy, PartialEq)]
enum ServerKind {
    StatelessRc,
    Stateful { version: &'static str },
}

/// A scripted MCP server over the egress boundary.
struct ScriptedEgress {
    kind: ServerKind,
    session_id: &'static str,
    log: Arc<Mutex<Vec<Recorded>>>,
}

impl ScriptedEgress {
    fn new(kind: ServerKind) -> (Arc<Self>, Arc<Mutex<Vec<Recorded>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let egress = Arc::new(Self {
            kind,
            session_id: "sess-xyz",
            log: log.clone(),
        });
        (egress, log)
    }

    fn header<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a String> {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    }

    fn ok(body: Value) -> EgressResponse {
        EgressResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&body).unwrap(),
        }
    }
}

#[async_trait]
impl EgressService for ScriptedEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let parsed: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let method = parsed
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let has_session = Self::header(&request.headers, "Mcp-Session-Id").is_some();
        let has_meta = parsed
            .get("params")
            .and_then(|p| p.get("_meta"))
            .map(|m| m.is_object())
            .unwrap_or(false);

        self.log.lock().unwrap().push(Recorded {
            method: method.clone(),
            has_session,
            has_meta,
            protocol_header: Self::header(&request.headers, "MCP-Protocol-Version").cloned(),
            method_header: Self::header(&request.headers, "Mcp-Method").cloned(),
        });

        let tools = json!({
            "jsonrpc": "2.0", "id": 1,
            "result": { "tools": [
                { "name": "search", "description": "Search", "inputSchema": { "type": "object" } }
            ]}
        });
        let call = json!({
            "jsonrpc": "2.0", "id": 1,
            "result": { "content": [{ "type": "text", "text": "ok" }], "isError": false }
        });

        match self.kind {
            ServerKind::StatelessRc => match method.as_str() {
                "tools/list" => Ok(Self::ok(tools)),
                "tools/call" => Ok(Self::ok(call)),
                _ => Ok(Self::ok(json!({ "jsonrpc": "2.0", "id": 1, "result": {} }))),
            },
            ServerKind::Stateful { version } => match method.as_str() {
                "initialize" => {
                    let mut headers = BTreeMap::new();
                    headers.insert("Mcp-Session-Id".to_string(), self.session_id.to_string());
                    Ok(EgressResponse {
                        status: 200,
                        headers,
                        body: serde_json::to_vec(&json!({
                            "jsonrpc": "2.0", "id": 0,
                            "result": { "protocolVersion": version, "capabilities": {} }
                        }))
                        .unwrap(),
                    })
                }
                "notifications/initialized" => Ok(EgressResponse {
                    status: 202,
                    headers: BTreeMap::new(),
                    body: Vec::new(),
                }),
                "tools/list" | "tools/call" if !has_session => Ok(EgressResponse {
                    status: 400,
                    headers: BTreeMap::new(),
                    body: b"Bad Request: Mcp-Session-Id header is required".to_vec(),
                }),
                "tools/list" => Ok(Self::ok(tools)),
                "tools/call" => Ok(Self::ok(call)),
                _ => Ok(Self::ok(json!({ "jsonrpc": "2.0", "id": 1, "result": {} }))),
            },
        }
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        unimplemented!("not used by MCP transport")
    }
}

// Public IP literal: passes SSRF validation without DNS, never actually dialed.
const URL: &str = "http://8.8.8.8/mcp";

fn client(egress: Arc<ScriptedEgress>) -> McpClient {
    McpClient::new(egress, Arc::new(NoAuthProvider))
}

#[tokio::test]
async fn auto_uses_stateless_path_for_rc_server() {
    let (egress, log) = ScriptedEgress::new(ServerKind::StatelessRc);
    let tools = client(egress)
        .discover(&McpConnection::http("docs", URL))
        .await
        .unwrap();
    assert_eq!(tools.len(), 1);

    let log = log.lock().unwrap();
    assert_eq!(log.len(), 1, "stateless server needs exactly one request");
    let req = &log[0];
    assert_eq!(req.method, "tools/list");
    assert!(!req.has_session, "no session header in stateless era");
    assert!(req.has_meta, "_meta must ride on the request body");
    assert_eq!(req.protocol_header.as_deref(), Some("2026-07-28"));
    assert_eq!(req.method_header.as_deref(), Some("tools/list"));
}

#[tokio::test]
async fn auto_falls_back_to_handshake_for_stateful_server() {
    let (egress, log) = ScriptedEgress::new(ServerKind::Stateful {
        version: "2025-03-26",
    });
    let tools = client(egress)
        .discover(&McpConnection::http("docs", URL))
        .await
        .unwrap();
    assert_eq!(tools.len(), 1);

    let log = log.lock().unwrap();
    let methods: Vec<&str> = log.iter().map(|r| r.method.as_str()).collect();
    // Stateless attempt rejected -> initialize -> initialized -> retried list.
    assert_eq!(
        methods,
        vec![
            "tools/list",
            "initialize",
            "notifications/initialized",
            "tools/list"
        ]
    );
    // The first attempt had no session; the retry echoes the captured session.
    assert!(!log[0].has_session);
    assert!(log[3].has_session, "retried list must carry the session id");
}

#[tokio::test]
async fn auto_caches_negotiation_so_call_after_list_reuses_session() {
    let (egress, log) = ScriptedEgress::new(ServerKind::Stateful {
        version: "2025-06-18",
    });
    let connection = McpConnection::http("docs", URL);
    let c = client(egress);
    c.discover(&connection).await.unwrap();
    let result = c.call(&connection, "search", json!({})).await.unwrap();
    assert!(!result.is_error);

    let log = log.lock().unwrap();
    let initialize_count = log.iter().filter(|r| r.method == "initialize").count();
    assert_eq!(
        initialize_count, 1,
        "negotiation must be cached: only one handshake across list + call"
    );
    // The cached session must reach the tools/call.
    let call = log.iter().rev().find(|r| r.method == "tools/call").unwrap();
    assert!(call.has_session, "cached session id must reach tools/call");
}

#[tokio::test]
async fn pinned_legacy_handshakes_first_without_a_stateless_probe() {
    let (egress, log) = ScriptedEgress::new(ServerKind::Stateful {
        version: "2025-03-26",
    });
    let connection = McpConnection::http("docs", URL).with_protocol_mode(McpProtocolMode::Legacy);
    client(egress).discover(&connection).await.unwrap();

    let log = log.lock().unwrap();
    assert_eq!(
        log[0].method, "initialize",
        "pinned Legacy must handshake first, not probe statelessly"
    );
    // No wasted session-less tools/list before the handshake.
    assert!(
        !log.iter()
            .take_while(|r| r.method != "initialize")
            .any(|r| r.method == "tools/list"),
        "no stateless probe should precede the pinned handshake"
    );
}

#[tokio::test]
async fn pinned_rc_never_handshakes() {
    let (egress, log) = ScriptedEgress::new(ServerKind::StatelessRc);
    let connection = McpConnection::http("docs", URL).with_protocol_mode(McpProtocolMode::Rc);
    client(egress).discover(&connection).await.unwrap();

    let log = log.lock().unwrap();
    assert!(
        log.iter().all(|r| r.method != "initialize"),
        "pinned RC must never run the handshake"
    );
    assert_eq!(log[0].protocol_header.as_deref(), Some("2026-07-28"));
}
