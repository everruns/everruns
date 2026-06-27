//! Multi-era MCP protocol support (specs/mcp-servers.md "Multi-era protocol
//! support").
//!
//! Everruns' MCP client speaks three eras with one code path:
//!
//! - Legacy `2025-03-26` / current `2025-06-18` are *stateful*: the client runs
//!   the `initialize` handshake, echoes any `Mcp-Session-Id` it receives, and
//!   sends `notifications/initialized`.
//! - RC `2026-07-28` is *stateless*: no handshake, protocol version + client
//!   info ride in `_meta` on every request, and routable headers let edge
//!   infrastructure route without parsing the body.
//!
//! This module holds the pure pieces (version selection, `_meta` and header
//! builders, request bodies, and the negotiation outcome). The egress-bound
//! orchestration that uses them lives in [`crate::http`].

use everruns_core::{
    MCP_PROTOCOL_VERSION_LEGACY, MCP_PROTOCOL_VERSION_RC, MCP_PROTOCOL_VERSION_STABLE,
    McpProtocolMode,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// Client name advertised to MCP servers.
pub const CLIENT_NAME: &str = "everruns";
/// Client version advertised to MCP servers (this crate's version).
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Canonical `_meta` key carrying client info, per the 2026 spec.
const CLIENT_INFO_META_KEY: &str = "io.modelcontextprotocol/clientInfo";

/// Routable headers (SEP-2243) — let gateways route/throttle without parsing
/// the JSON body. Additive and ignored by pre-RC servers.
pub const HEADER_PROTOCOL_VERSION: &str = "MCP-Protocol-Version";
pub const HEADER_METHOD: &str = "Mcp-Method";
pub const HEADER_NAME: &str = "Mcp-Name";
/// Session header for stateful (legacy/current) servers.
pub const HEADER_SESSION_ID: &str = "Mcp-Session-Id";

/// Default stateful era to request when `Auto` falls back to a handshake. The
/// server's `initialize` response decides the actual negotiated version; this
/// is only the client's *preferred* version in the handshake.
pub const DEFAULT_STATEFUL_VERSION: &str = MCP_PROTOCOL_VERSION_STABLE;

/// Client identity object (`{ "name", "version" }`).
pub fn client_info() -> Value {
    json!({ "name": CLIENT_NAME, "version": CLIENT_VERSION })
}

/// The `_meta` object carried on every request body in the stateless era.
/// Additive for stateful servers, which ignore unknown `params._meta`.
pub fn request_meta() -> Value {
    let mut meta = Map::new();
    meta.insert(CLIENT_INFO_META_KEY.to_string(), client_info());
    Value::Object(meta)
}

/// Routable headers for an operation. `name` is the tool name for
/// `tools/call`, `None` for `tools/list` / `initialize`.
pub fn routable_headers(version: &str, method: &str, name: Option<&str>) -> Vec<(String, String)> {
    let mut headers = vec![
        (HEADER_PROTOCOL_VERSION.to_string(), version.to_string()),
        (HEADER_METHOD.to_string(), method.to_string()),
    ];
    if let Some(name) = name {
        headers.push((HEADER_NAME.to_string(), name.to_string()));
    }
    headers
}

/// `tools/list` request body, carrying `_meta`.
pub fn tools_list_body(id: i64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
        "params": { "_meta": request_meta() }
    })
}

/// `tools/call` request body, carrying `_meta`.
pub fn tools_call_body(id: i64, name: &str, arguments: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
            "_meta": request_meta(),
        }
    })
}

/// `initialize` handshake body for the given (preferred) protocol version.
pub fn initialize_body(id: i64, version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": client_info(),
        }
    })
}

/// `notifications/initialized` body (a notification — no `id`).
pub fn initialized_notification() -> Value {
    json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
}

/// Extract `protocolVersion` from an `initialize` result body, if present.
pub fn protocol_version_from_initialize(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    value
        .get("result")?
        .get("protocolVersion")?
        .as_str()
        .map(|s| s.to_string())
}

/// Case-insensitive lookup of the `Mcp-Session-Id` response header.
pub fn session_id_from_headers(headers: &BTreeMap<String, String>) -> Option<String> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(HEADER_SESSION_ID))
        .map(|(_, value)| value.clone())
        .filter(|value| !value.is_empty())
}

/// Heuristic: does a *failed stateless attempt* indicate the server requires
/// the stateful `initialize` handshake? Used only by `Auto` to decide whether
/// to fall back. Deliberately conservative — a false negative just surfaces the
/// original error; a false positive costs one wasted handshake.
pub fn looks_like_handshake_required(status: u16, body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    // HTTP-level signals from servers that reject session-less requests.
    if matches!(status, 400 | 404 | 405 | 409 | 426) {
        if lower.contains("session")
            || lower.contains("initialize")
            || lower.contains("mcp-session-id")
        {
            return true;
        }
    }
    // JSON-RPC-level signals (may arrive on a 200).
    lower.contains("server not initialized")
        || lower.contains("session required")
        || lower.contains("missing session")
        || lower.contains("not initialized")
}

/// The outcome of protocol negotiation for one server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Negotiated {
    /// Negotiated protocol version string.
    pub version: String,
    /// Whether the server requires the stateful handshake + session id.
    pub stateful: bool,
    /// Session id captured from the `initialize` response, if any.
    pub session_id: Option<String>,
}

impl Negotiated {
    /// Stateless negotiation (RC): no handshake, no session id.
    pub fn stateless(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            stateful: false,
            session_id: None,
        }
    }

    /// The optimistic starting point for a given mode before any probe.
    /// `Auto` starts stateless and may fall back on the first failure.
    pub fn initial_for_mode(mode: McpProtocolMode) -> Self {
        match mode {
            McpProtocolMode::Auto | McpProtocolMode::Rc => {
                Negotiated::stateless(MCP_PROTOCOL_VERSION_RC)
            }
            McpProtocolMode::Stable => Self {
                version: MCP_PROTOCOL_VERSION_STABLE.to_string(),
                stateful: true,
                session_id: None,
            },
            McpProtocolMode::Legacy => Self {
                version: MCP_PROTOCOL_VERSION_LEGACY.to_string(),
                stateful: true,
                session_id: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_meta_carries_client_info_under_canonical_key() {
        let meta = request_meta();
        let info = meta.get(CLIENT_INFO_META_KEY).expect("client info present");
        assert_eq!(info.get("name").and_then(|v| v.as_str()), Some(CLIENT_NAME));
        assert_eq!(
            info.get("version").and_then(|v| v.as_str()),
            Some(CLIENT_VERSION)
        );
    }

    #[test]
    fn routable_headers_include_name_only_for_calls() {
        let list = routable_headers(MCP_PROTOCOL_VERSION_RC, "tools/list", None);
        assert!(list.iter().all(|(k, _)| k != HEADER_NAME));
        assert!(
            list.iter()
                .any(|(k, v)| k == HEADER_PROTOCOL_VERSION && v == MCP_PROTOCOL_VERSION_RC)
        );

        let call = routable_headers(MCP_PROTOCOL_VERSION_RC, "tools/call", Some("search"));
        assert!(call.iter().any(|(k, v)| k == HEADER_NAME && v == "search"));
        assert!(
            call.iter()
                .any(|(k, v)| k == HEADER_METHOD && v == "tools/call")
        );
    }

    #[test]
    fn bodies_carry_meta() {
        let list = tools_list_body(1);
        assert!(list["params"]["_meta"].is_object());
        let call = tools_call_body(2, "search", &json!({"q": "x"}));
        assert_eq!(call["params"]["name"], "search");
        assert!(call["params"]["_meta"].is_object());
        assert_eq!(call["params"]["arguments"]["q"], "x");
    }

    #[test]
    fn parses_protocol_version_and_session_id() {
        let body = r#"{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2025-03-26","capabilities":{}}}"#;
        assert_eq!(
            protocol_version_from_initialize(body).as_deref(),
            Some("2025-03-26")
        );

        let mut headers = BTreeMap::new();
        headers.insert("mcp-session-id".to_string(), "abc123".to_string());
        assert_eq!(session_id_from_headers(&headers).as_deref(), Some("abc123"));
        headers.clear();
        assert_eq!(session_id_from_headers(&headers), None);
    }

    #[test]
    fn handshake_detection_triggers_on_session_signals_only() {
        assert!(looks_like_handshake_required(
            400,
            "Bad Request: Mcp-Session-Id header is required"
        ));
        assert!(looks_like_handshake_required(
            200,
            r#"{"error":{"code":-32600,"message":"Server not initialized"}}"#
        ));
        assert!(!looks_like_handshake_required(500, "internal server error"));
        assert!(!looks_like_handshake_required(400, "invalid arguments"));
    }

    #[test]
    fn initial_negotiation_matches_mode() {
        assert!(!Negotiated::initial_for_mode(McpProtocolMode::Auto).stateful);
        assert!(!Negotiated::initial_for_mode(McpProtocolMode::Rc).stateful);
        assert!(Negotiated::initial_for_mode(McpProtocolMode::Stable).stateful);
        assert!(Negotiated::initial_for_mode(McpProtocolMode::Legacy).stateful);
        assert_eq!(
            Negotiated::initial_for_mode(McpProtocolMode::Legacy).version,
            MCP_PROTOCOL_VERSION_LEGACY
        );
    }
}
