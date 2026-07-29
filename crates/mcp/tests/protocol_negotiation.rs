//! Multi-era protocol negotiation integration tests (specs/mcp-servers.md
//! "Multi-era protocol support").
//!
//! A scripted [`EgressService`] stands in for three kinds of MCP server:
//!
//! - a *stateless* 2026-07-28 server that answers operations directly,
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
use everruns_mcp::{McpClient, McpConnection, NoAuthProvider, StaticAuthProvider};
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
    authorization: Option<String>,
    session_id: Option<String>,
}

/// Whether the scripted server behaves statelessly (2026-07-28) or demands a handshake.
#[derive(Clone, Copy, PartialEq)]
enum ServerKind {
    StatelessRc,
    Stateful {
        version: &'static str,
    },
    RejectsRc {
        version: &'static str,
        stable_failure: bool,
    },
    /// Stateless server that answers the first `tools/call` with an
    /// `input_required` result (MRTR) and the retry with a real result.
    Mrtr {
        /// Whether the `input_required` names concrete `inputRequests`. A
        /// compliant server would not, since this client declares no
        /// elicitation/sampling/roots capability.
        with_input_requests: bool,
    },
}

/// A scripted MCP server over the egress boundary.
struct ScriptedEgress {
    kind: ServerKind,
    session_id: &'static str,
    /// `(ttlMs, cacheScope)` to attach to `tools/list` results, mimicking a
    /// 2026-07-28 server that sends caching hints. `None` mimics an older
    /// server that sends none.
    cache_hints: Option<(i64, &'static str)>,
    log: Arc<Mutex<Vec<Recorded>>>,
}

impl ScriptedEgress {
    fn new(kind: ServerKind) -> (Arc<Self>, Arc<Mutex<Vec<Recorded>>>) {
        Self::build(kind, None)
    }

    /// A server that decorates `tools/list` with caching hints.
    fn cacheable(
        kind: ServerKind,
        ttl_ms: i64,
        scope: &'static str,
    ) -> (Arc<Self>, Arc<Mutex<Vec<Recorded>>>) {
        Self::build(kind, Some((ttl_ms, scope)))
    }

    fn build(
        kind: ServerKind,
        cache_hints: Option<(i64, &'static str)>,
    ) -> (Arc<Self>, Arc<Mutex<Vec<Recorded>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let egress = Arc::new(Self {
            kind,
            session_id: "sess-xyz",
            cache_hints,
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
        let session_id = Self::header(&request.headers, "Mcp-Session-Id").cloned();
        let has_session = session_id.is_some();
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
            authorization: Self::header(&request.headers, "Authorization").cloned(),
            session_id,
        });

        let mut tools = json!({
            "jsonrpc": "2.0", "id": 1,
            "result": { "tools": [
                { "name": "search", "description": "Search", "inputSchema": { "type": "object" } }
            ]}
        });
        if let Some((ttl_ms, scope)) = self.cache_hints {
            let result = tools["result"].as_object_mut().unwrap();
            result.insert("resultType".to_string(), json!("complete"));
            result.insert("ttlMs".to_string(), json!(ttl_ms));
            result.insert("cacheScope".to_string(), json!(scope));
        }
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
            ServerKind::Mrtr {
                with_input_requests,
            } => match method.as_str() {
                "tools/list" => Ok(Self::ok(tools)),
                "tools/call" => {
                    let prior_calls = self
                        .log
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|r| r.method == "tools/call")
                        .count();
                    // The push above already counted this request.
                    if prior_calls > 1 {
                        return Ok(Self::ok(call));
                    }
                    let mut result = json!({
                        "resultType": "input_required",
                        "requestState": "opaque-state",
                    });
                    if with_input_requests {
                        result["inputRequests"] = json!({
                            "github_login": { "method": "elicitation/create" }
                        });
                    }
                    Ok(Self::ok(
                        json!({ "jsonrpc": "2.0", "id": 1, "result": result }),
                    ))
                }
                _ => Ok(Self::ok(json!({ "jsonrpc": "2.0", "id": 1, "result": {} }))),
            },
            kind => {
                let (version, stable_failure, stateless_rejection) = match kind {
                    ServerKind::Stateful { version } => (
                        version,
                        false,
                        b"Bad Request: Mcp-Session-Id header is required".to_vec(),
                    ),
                    ServerKind::RejectsRc {
                        version,
                        stable_failure,
                    } => (
                        version,
                        stable_failure,
                        serde_json::to_vec(&json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "error": {
                                "code": -32600,
                                "message": "Bad Request: Unsupported protocol version: 2026-07-28 (supported versions include 2025-06-18)"
                            }
                        }))
                        .unwrap(),
                    ),
                    ServerKind::StatelessRc | ServerKind::Mrtr { .. } => unreachable!(),
                };
                match method.as_str() {
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
                        body: stateless_rejection,
                    }),
                    "tools/list" | "tools/call" if stable_failure => Ok(EgressResponse {
                        status: 400,
                        headers: BTreeMap::new(),
                        body: b"stable request rejected".to_vec(),
                    }),
                    "tools/list" => Ok(Self::ok(tools)),
                    "tools/call" => Ok(Self::ok(call)),
                    _ => Ok(Self::ok(json!({ "jsonrpc": "2.0", "id": 1, "result": {} }))),
                }
            }
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
async fn auto_falls_back_when_authenticated_server_rejects_rc_protocol_version() {
    let (egress, log) = ScriptedEgress::new(ServerKind::RejectsRc {
        version: "2025-06-18",
        stable_failure: false,
    });
    let auth = StaticAuthProvider::new().with_bearer("linear", "linear-token");
    let tools = McpClient::new(egress, Arc::new(auth))
        .discover(&McpConnection::http("linear", URL))
        .await
        .unwrap();
    assert_eq!(tools.len(), 1);

    let log = log.lock().unwrap();
    let methods: Vec<&str> = log.iter().map(|request| request.method.as_str()).collect();
    assert_eq!(
        methods,
        vec![
            "tools/list",
            "initialize",
            "notifications/initialized",
            "tools/list"
        ],
        "the rejected 2026-07-28 probe must cause exactly one stateful retry"
    );
    assert!(
        log.iter()
            .all(|request| request.authorization.as_deref() == Some("Bearer linear-token")),
        "authentication must be preserved across the fallback"
    );
    assert_eq!(log[0].protocol_header.as_deref(), Some("2026-07-28"));
    assert_eq!(log[3].protocol_header.as_deref(), Some("2025-06-18"));
    assert!(
        log[3].has_session,
        "the stable retry must carry the session id"
    );
}

#[tokio::test]
async fn auto_reports_rc_probe_and_stable_fallback_failures() {
    let (egress, log) = ScriptedEgress::new(ServerKind::RejectsRc {
        version: "2025-06-18",
        stable_failure: true,
    });
    let error = client(egress)
        .discover(&McpConnection::http("linear", URL))
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("MCP 2026-07-28 probe failed"));
    assert!(error.contains("Unsupported protocol version: 2026-07-28"));
    assert!(error.contains("stateful fallback failed: 400 - stable request rejected"));
    let log = log.lock().unwrap();
    assert_eq!(
        log.iter()
            .filter(|request| request.method == "initialize")
            .count(),
        1,
        "a failed fallback must not restart negotiation"
    );
    assert_eq!(
        log.iter()
            .filter(|request| request.method == "tools/list")
            .count(),
        2,
        "the operation must be retried exactly once"
    );
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
async fn negotiation_cache_is_scoped_by_auth_and_protocol_mode() {
    let (egress, log) = ScriptedEgress::new(ServerKind::Stateful {
        version: "2025-06-18",
    });
    let auth = StaticAuthProvider::new()
        .with_bearer("alice", "alice-token")
        .with_bearer("bob", "bob-token");
    let c = McpClient::new(egress, Arc::new(auth));

    c.discover(&McpConnection::http("alice", URL))
        .await
        .unwrap();
    c.discover(&McpConnection::http("bob", URL)).await.unwrap();

    let log = log.lock().unwrap();
    let initialize_count = log.iter().filter(|r| r.method == "initialize").count();
    assert_eq!(
        initialize_count, 2,
        "same-URL connections with different credentials must not share sessions"
    );

    let bob_initialize = log
        .iter()
        .find(|r| {
            r.method == "initialize" && r.authorization.as_deref() == Some("Bearer bob-token")
        })
        .expect("bob must negotiate his own session");
    assert!(
        bob_initialize.session_id.is_none(),
        "bob's initialize must not inherit alice's session id"
    );
}

#[tokio::test]
async fn pinned_legacy_handshakes_first_without_a_stateless_probe() {
    let (egress, log) = ScriptedEgress::new(ServerKind::Stateful {
        version: "2025-03-26",
    });
    let connection =
        McpConnection::http("docs", URL).with_protocol_mode(McpProtocolMode::V2025March);
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
async fn pinned_stable_handshakes_first_without_a_stateless_probe() {
    let (egress, log) = ScriptedEgress::new(ServerKind::Stateful {
        version: "2025-06-18",
    });
    let connection =
        McpConnection::http("docs", URL).with_protocol_mode(McpProtocolMode::V2025June);
    client(egress).discover(&connection).await.unwrap();

    let log = log.lock().unwrap();
    assert_eq!(log[0].method, "initialize");
    assert_eq!(log[0].protocol_header.as_deref(), Some("2025-06-18"));
    assert_eq!(
        log.iter()
            .filter(|request| request.method == "initialize")
            .count(),
        1
    );
}

#[tokio::test]
async fn pinned_rc_never_handshakes() {
    let (egress, log) = ScriptedEgress::new(ServerKind::StatelessRc);
    let connection =
        McpConnection::http("docs", URL).with_protocol_mode(McpProtocolMode::V2026July);
    client(egress).discover(&connection).await.unwrap();

    let log = log.lock().unwrap();
    assert!(
        log.iter().all(|r| r.method != "initialize"),
        "a pinned 2026-07-28 mode must never run the handshake"
    );
    assert_eq!(log[0].protocol_header.as_deref(), Some("2026-07-28"));
}

// ---------------------------------------------------------------------------
// Cacheable list results (2026-07-28 `ttlMs` / `cacheScope`)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn positive_ttl_serves_the_second_discover_from_cache() {
    let (egress, log) = ScriptedEgress::cacheable(ServerKind::StatelessRc, 300_000, "public");
    let client = client(egress);
    let connection = McpConnection::http("docs", URL);

    let first = client.discover(&connection).await.unwrap();
    let second = client.discover(&connection).await.unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1, "cached result must match the fetched one");

    let log = log.lock().unwrap();
    assert_eq!(
        log.iter()
            .filter(|request| request.method == "tools/list")
            .count(),
        1,
        "a fresh cached list must not be re-fetched"
    );
}

#[tokio::test]
async fn absent_hints_are_never_cached() {
    // 2025-era servers send no `ttlMs`; the spec says treat that as immediately
    // stale rather than inventing a client-side TTL.
    let (egress, log) = ScriptedEgress::new(ServerKind::StatelessRc);
    let client = client(egress);
    let connection = McpConnection::http("docs", URL);

    client.discover(&connection).await.unwrap();
    client.discover(&connection).await.unwrap();

    let log = log.lock().unwrap();
    assert_eq!(
        log.iter()
            .filter(|request| request.method == "tools/list")
            .count(),
        2,
        "without hints every discover must re-fetch"
    );
}

#[tokio::test]
async fn zero_and_negative_ttl_are_treated_as_immediately_stale() {
    for ttl_ms in [0, -1] {
        let (egress, log) = ScriptedEgress::cacheable(ServerKind::StatelessRc, ttl_ms, "public");
        let client = client(egress);
        let connection = McpConnection::http("docs", URL);

        client.discover(&connection).await.unwrap();
        client.discover(&connection).await.unwrap();

        let log = log.lock().unwrap();
        assert_eq!(
            log.iter()
                .filter(|request| request.method == "tools/list")
                .count(),
            2,
            "ttlMs {ttl_ms} must not produce a fresh cache entry"
        );
    }
}

// ---------------------------------------------------------------------------
// Multi round-trip requests (MRTR)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn input_required_without_requests_is_retried_with_request_state() {
    let (egress, log) = ScriptedEgress::new(ServerKind::Mrtr {
        with_input_requests: false,
    });
    let result = client(egress)
        .call(&McpConnection::http("docs", URL), "search", json!({}))
        .await
        .expect("MRTR retry should complete the call");
    assert!(!result.is_error);

    let log = log.lock().unwrap();
    assert_eq!(
        log.iter()
            .filter(|request| request.method == "tools/call")
            .count(),
        2,
        "input_required must trigger exactly one retry"
    );
}

#[tokio::test]
async fn input_required_naming_unsupported_inputs_fails_loudly() {
    // The client declares no elicitation capability, so a server asking for one
    // is out of contract. Returning an empty result would look like success.
    let (egress, log) = ScriptedEgress::new(ServerKind::Mrtr {
        with_input_requests: true,
    });
    let error = client(egress)
        .call(&McpConnection::http("docs", URL), "search", json!({}))
        .await
        .expect_err("unsupported input requests must surface as an error");
    let error = error.to_string();
    assert!(error.contains("github_login"), "unexpected error: {error}");

    let log = log.lock().unwrap();
    assert_eq!(
        log.iter()
            .filter(|request| request.method == "tools/call")
            .count(),
        1,
        "there is nothing to retry with, so no retry should be sent"
    );
}
