//! Multi-era MCP protocol support (knowledge/integrations/mcp-servers.md "Multi-era protocol
//! support").
//!
//! Everruns' MCP client speaks three eras with one code path:
//!
//! - `2025-03-26` / `2025-06-18` are *stateful*: the client runs the
//!   `initialize` handshake, echoes any `Mcp-Session-Id` it receives, and
//!   sends `notifications/initialized`.
//! - `2026-07-28` is *stateless*: no handshake, protocol version + client
//!   info ride in `_meta` on every request, and routable headers let edge
//!   infrastructure route without parsing the body.
//!
//! This module holds the pure pieces (version selection, `_meta` and header
//! builders, request bodies, and the negotiation outcome). The egress-bound
//! orchestration that uses them lives in [`crate::http`].

use everruns_core::{
    MCP_PROTOCOL_VERSION_2025_03, MCP_PROTOCOL_VERSION_2025_06, MCP_PROTOCOL_VERSION_2026_07,
    McpProtocolMode,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

/// Client name advertised to MCP servers.
pub const CLIENT_NAME: &str = "everruns";
/// Client version advertised to MCP servers (this crate's version).
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Canonical `_meta` keys the 2026 spec requires on every request. They replace
/// what the `initialize` handshake used to establish once per connection.
const CLIENT_INFO_META_KEY: &str = "io.modelcontextprotocol/clientInfo";
const PROTOCOL_VERSION_META_KEY: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES_META_KEY: &str = "io.modelcontextprotocol/clientCapabilities";

/// Routable headers (SEP-2243) — let gateways route/throttle without parsing
/// the JSON body. Additive and ignored by 2025-era servers.
pub const HEADER_PROTOCOL_VERSION: &str = "MCP-Protocol-Version";
pub const HEADER_METHOD: &str = "Mcp-Method";
pub const HEADER_NAME: &str = "Mcp-Name";
/// Session header for stateful (legacy/current) servers.
pub const HEADER_SESSION_ID: &str = "Mcp-Session-Id";

/// Default stateful era to request when `Auto` falls back to a handshake. The
/// server's `initialize` response decides the actual negotiated version; this
/// is only the client's *preferred* version in the handshake.
pub const DEFAULT_STATEFUL_VERSION: &str = MCP_PROTOCOL_VERSION_2025_06;

/// Client identity object (`{ "name", "version" }`).
pub fn client_info() -> Value {
    json!({ "name": CLIENT_NAME, "version": CLIENT_VERSION })
}

/// What this client can answer when a server asks for input mid-call.
///
/// Capability declaration is load-bearing, not cosmetic: under MRTR a server
/// **MUST NOT** ask for an input type the client did not declare, so an
/// accurate declaration is what keeps servers from blocking on prompts nobody
/// can answer. Everything here defaults to "cannot", and a host turns a flag on
/// only by supplying the machinery that answers it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClientCapabilities {
    /// The host can show a URL to a human, take their consent, and open it out
    /// of band ([`crate::elicitation::UrlElicitationHandler`]).
    pub url_elicitation: bool,
}

impl ClientCapabilities {
    /// Declares nothing — the default for hosts with no human in the loop.
    pub fn none() -> Self {
        Self::default()
    }
}

/// Capabilities this client advertises, in wire shape.
///
/// `sampling` and `roots` are always absent: both require a model or a
/// filesystem view to answer a request mid-call, which this transport has no
/// way to reach. `elicitation` is declared only in `url` mode and only when the
/// host supplied a consent handler; form mode stays undeclared because a form
/// answer would have to flow back through the model, which is exactly what URL
/// mode exists to avoid.
pub fn client_capabilities(capabilities: ClientCapabilities) -> Value {
    if capabilities.url_elicitation {
        json!({ "elicitation": { "url": {} } })
    } else {
        json!({})
    }
}

/// The `_meta` object carried on every request body in the stateless era.
///
/// Carries what the `initialize` handshake used to negotiate once: protocol
/// version, client identity, and client capabilities. Additive for stateful
/// servers, which ignore unknown `params._meta`.
pub fn request_meta(version: &str, capabilities: ClientCapabilities) -> Value {
    let mut meta = Map::new();
    meta.insert(PROTOCOL_VERSION_META_KEY.to_string(), json!(version));
    meta.insert(CLIENT_INFO_META_KEY.to_string(), client_info());
    meta.insert(
        CLIENT_CAPABILITIES_META_KEY.to_string(),
        client_capabilities(capabilities),
    );
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
pub fn tools_list_body(id: i64, version: &str, capabilities: ClientCapabilities) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
        "params": { "_meta": request_meta(version, capabilities) }
    })
}

/// `tools/call` request body, carrying `_meta`.
pub fn tools_call_body(
    id: i64,
    name: &str,
    arguments: &Value,
    version: &str,
    capabilities: ClientCapabilities,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
            "_meta": request_meta(version, capabilities),
        }
    })
}

/// `tools/call` retry body for a multi round-trip request.
///
/// Echoes `requestState` back verbatim — it is opaque server state that the
/// client must not inspect or alter — carries any `inputResponses` gathered for
/// the server's `inputRequests`, and uses a **different** JSON-RPC id from the
/// request that produced the `input_required` result, since MRTR treats the
/// retry as an independent request.
pub fn tools_call_retry_body(
    id: i64,
    name: &str,
    arguments: &Value,
    version: &str,
    capabilities: ClientCapabilities,
    request_state: Option<&str>,
    input_responses: &BTreeMap<String, Value>,
) -> Value {
    let mut body = tools_call_body(id, name, arguments, version, capabilities);
    let Some(params) = body["params"].as_object_mut() else {
        return body;
    };
    if let Some(request_state) = request_state {
        params.insert("requestState".to_string(), json!(request_state));
    }
    if !input_responses.is_empty() {
        params.insert(
            "inputResponses".to_string(),
            Value::Object(input_responses.clone().into_iter().collect()),
        );
    }
    body
}

/// `initialize` handshake body for the given (preferred) protocol version.
pub fn initialize_body(id: i64, version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": version,
            // 2025-era elicitation is a server-initiated request over a
            // server→client stream this transport does not open, so the
            // handshake declares nothing regardless of host capabilities. URL
            // mode elicitation is 2026-07-28-only for us, via `_meta`.
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

/// One entry of an MRTR `inputRequests` map: the server-assigned key plus the
/// request it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputRequest {
    /// Key the response must be returned under in `inputResponses`.
    pub key: String,
    /// JSON-RPC method the server is asking the client to serve
    /// (`elicitation/create`, `sampling/createMessage`, `roots/list`).
    pub method: String,
    /// The request's params, uninterpreted.
    pub params: Value,
}

impl InputRequest {
    /// The URL mode elicitation this request carries, if that is what it is.
    ///
    /// Returns `(message, url)`. A `mode` of `form` (or an absent `mode`, which
    /// means form) is not a URL elicitation and yields `None` — this client
    /// declares no form mode, so such a request is a server error, reported by
    /// the caller rather than silently answered.
    pub fn url_elicitation(&self) -> Option<(String, String)> {
        if self.method != "elicitation/create" {
            return None;
        }
        if self.params.get("mode").and_then(Value::as_str)? != "url" {
            return None;
        }
        let url = self.params.get("url").and_then(Value::as_str)?.to_string();
        let message = self
            .params
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("The server needs you to complete an interaction in your browser.")
            .to_string();
        Some((message, url))
    }
}

/// A `resultType: "input_required"` result — the multi round-trip request
/// (MRTR) pattern that replaced server-initiated requests in `2026-07-28`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputRequired {
    /// The inputs the server wants. Empty when the server only needs the round
    /// trip itself.
    pub requests: Vec<InputRequest>,
    /// Opaque server state to echo back on the retry, if any.
    pub request_state: Option<String>,
}

impl InputRequired {
    /// Server-assigned keys, in the order they were parsed.
    pub fn keys(&self) -> Vec<String> {
        self.requests.iter().map(|r| r.key.clone()).collect()
    }
}

/// Read an `input_required` result from a JSON-RPC result body, if that is what
/// it is. Returns `None` for ordinary (`complete`) results.
pub fn input_required_from_result(body: &str) -> Option<InputRequired> {
    let value: Value = serde_json::from_str(body).ok()?;
    let result = value.get("result")?;
    if result.get("resultType")?.as_str()? != "input_required" {
        return None;
    }
    let requests = result
        .get("inputRequests")
        .and_then(Value::as_object)
        .map(|requests| {
            requests
                .iter()
                .map(|(key, request)| InputRequest {
                    key: key.clone(),
                    method: request
                        .get("method")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    params: request.get("params").cloned().unwrap_or(Value::Null),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(InputRequired {
        requests,
        request_state: result
            .get("requestState")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Who may reuse a cached result, per the `2026-07-28` caching spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheScope {
    /// No caller-specific data — reusable across authorization contexts.
    Public,
    /// Reusable only within the authorization context that fetched it.
    Private,
}

/// Server-provided caching hints on a `2026-07-28` list/read result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheHints {
    /// How long the result may be considered fresh.
    pub ttl: Duration,
    /// Who may reuse it.
    pub scope: CacheScope,
}

/// Read `ttlMs` / `cacheScope` from a JSON-RPC result body.
///
/// Returns `None` when the result is not cacheable, which per spec covers every
/// case that is not an explicitly positive `ttlMs`: absent (2025-era servers),
/// zero, or negative (which clients must treat as zero). An unrecognized or
/// missing `cacheScope` is read as `Private` — the conservative direction, since
/// treating caller-specific data as public would leak it across authorization
/// contexts.
pub fn cache_hints_from_result(body: &str) -> Option<CacheHints> {
    let value: Value = serde_json::from_str(body).ok()?;
    let result = value.get("result")?;
    let ttl_ms = result.get("ttlMs")?.as_i64()?;
    if ttl_ms <= 0 {
        return None;
    }
    let scope = match result.get("cacheScope").and_then(Value::as_str) {
        Some("public") => CacheScope::Public,
        _ => CacheScope::Private,
    };
    Some(CacheHints {
        ttl: Duration::from_millis(ttl_ms as u64),
        scope,
    })
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
    if matches!(status, 400 | 404 | 405 | 409 | 426)
        && (lower.contains("session")
            || lower.contains("initialize")
            || lower.contains("mcp-session-id"))
    {
        return true;
    }
    // Some 2025-era servers reject the optimistic 2026 header before they can
    // report a session requirement. Keep this limited to explicit protocol
    // version language so unrelated validation failures do not trigger a retry.
    if status == 400
        && (lower.contains("protocol version")
            || lower.contains("protocol-version")
            || lower.contains("protocolversion"))
        && (lower.contains("unsupported")
            || lower.contains("not supported")
            || lower.contains("invalid"))
    {
        return true;
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
    /// Stateless negotiation (2026-07-28): no handshake, no session id.
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
            McpProtocolMode::Auto | McpProtocolMode::V2026July => {
                Negotiated::stateless(MCP_PROTOCOL_VERSION_2026_07)
            }
            McpProtocolMode::V2025June => Self {
                version: MCP_PROTOCOL_VERSION_2025_06.to_string(),
                stateful: true,
                session_id: None,
            },
            McpProtocolMode::V2025March => Self {
                version: MCP_PROTOCOL_VERSION_2025_03.to_string(),
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
        let meta = request_meta(MCP_PROTOCOL_VERSION_2026_07, ClientCapabilities::none());
        let info = meta.get(CLIENT_INFO_META_KEY).expect("client info present");
        assert_eq!(info.get("name").and_then(|v| v.as_str()), Some(CLIENT_NAME));
        assert_eq!(
            info.get("version").and_then(|v| v.as_str()),
            Some(CLIENT_VERSION)
        );
    }

    #[test]
    fn routable_headers_include_name_only_for_calls() {
        let list = routable_headers(MCP_PROTOCOL_VERSION_2026_07, "tools/list", None);
        assert!(list.iter().all(|(k, _)| k != HEADER_NAME));
        assert!(
            list.iter()
                .any(|(k, v)| k == HEADER_PROTOCOL_VERSION && v == MCP_PROTOCOL_VERSION_2026_07)
        );

        let call = routable_headers(MCP_PROTOCOL_VERSION_2026_07, "tools/call", Some("search"));
        assert!(call.iter().any(|(k, v)| k == HEADER_NAME && v == "search"));
        assert!(
            call.iter()
                .any(|(k, v)| k == HEADER_METHOD && v == "tools/call")
        );
    }

    #[test]
    fn bodies_carry_meta() {
        let list = tools_list_body(1, MCP_PROTOCOL_VERSION_2026_07, ClientCapabilities::none());
        assert!(list["params"]["_meta"].is_object());
        let call = tools_call_body(
            2,
            "search",
            &json!({"q": "x"}),
            MCP_PROTOCOL_VERSION_2026_07,
            ClientCapabilities::none(),
        );
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
    fn handshake_detection_requires_explicit_fallback_signals() {
        assert!(looks_like_handshake_required(
            400,
            "Bad Request: Mcp-Session-Id header is required"
        ));
        assert!(looks_like_handshake_required(
            200,
            r#"{"error":{"code":-32600,"message":"Server not initialized"}}"#
        ));
        assert!(looks_like_handshake_required(
            400,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Unsupported protocol version: 2026-07-28"}}"#
        ));
        assert!(looks_like_handshake_required(
            400,
            "Invalid MCP-Protocol-Version header"
        ));
        assert!(!looks_like_handshake_required(500, "internal server error"));
        assert!(!looks_like_handshake_required(400, "invalid arguments"));
        assert!(!looks_like_handshake_required(
            400,
            "unsupported tool argument"
        ));
    }

    #[test]
    fn initial_negotiation_matches_mode() {
        assert!(!Negotiated::initial_for_mode(McpProtocolMode::Auto).stateful);
        assert!(!Negotiated::initial_for_mode(McpProtocolMode::V2026July).stateful);
        assert!(Negotiated::initial_for_mode(McpProtocolMode::V2025June).stateful);
        assert!(Negotiated::initial_for_mode(McpProtocolMode::V2025March).stateful);
        assert_eq!(
            Negotiated::initial_for_mode(McpProtocolMode::V2025March).version,
            MCP_PROTOCOL_VERSION_2025_03
        );
    }

    #[test]
    fn meta_carries_version_capabilities_and_client_info() {
        let meta = request_meta(MCP_PROTOCOL_VERSION_2026_07, ClientCapabilities::none());
        assert_eq!(
            meta.get(PROTOCOL_VERSION_META_KEY).and_then(|v| v.as_str()),
            Some(MCP_PROTOCOL_VERSION_2026_07)
        );
        // An empty capabilities object is a positive declaration that this
        // client answers no server-initiated input requests.
        assert_eq!(meta.get(CLIENT_CAPABILITIES_META_KEY), Some(&json!({})));

        // A host that can reach a human declares URL mode elicitation — and
        // only URL mode: form mode would route the answer back through the
        // model, which is what URL mode exists to avoid.
        let with_elicitation = request_meta(
            MCP_PROTOCOL_VERSION_2026_07,
            ClientCapabilities {
                url_elicitation: true,
            },
        );
        assert_eq!(
            with_elicitation.get(CLIENT_CAPABILITIES_META_KEY),
            Some(&json!({ "elicitation": { "url": {} } }))
        );
    }

    #[test]
    fn url_elicitation_requests_are_recognized_and_others_are_not() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"input_required",
            "inputRequests":{
              "connect":{"method":"elicitation/create","params":{"mode":"url",
                "url":"https://mcp.example.com/connect?x=1","message":"Authorize Example."}},
              "name":{"method":"elicitation/create","params":{"mode":"form","message":"Name?"}},
              "guess":{"method":"sampling/createMessage","params":{}}
            },"requestState":"blob"}}"#;
        let parsed = input_required_from_result(body).expect("input_required");
        let by_key = |key: &str| {
            parsed
                .requests
                .iter()
                .find(|r| r.key == key)
                .expect("request present")
                .clone()
        };
        assert_eq!(
            by_key("connect").url_elicitation(),
            Some((
                "Authorize Example.".to_string(),
                "https://mcp.example.com/connect?x=1".to_string()
            ))
        );
        // Form mode and sampling are not URL elicitations: this client
        // declares neither, so the caller reports them instead of answering.
        assert_eq!(by_key("name").url_elicitation(), None);
        assert_eq!(by_key("guess").url_elicitation(), None);
    }

    #[test]
    fn detects_input_required_results() {
        let bare = r#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"input_required","requestState":"opaque"}}"#;
        let parsed = input_required_from_result(bare).expect("input_required");
        assert!(parsed.requests.is_empty());
        assert_eq!(parsed.request_state.as_deref(), Some("opaque"));

        let with_requests = r#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"input_required","inputRequests":{"github_login":{"method":"elicitation/create"}}}}"#;
        let parsed = input_required_from_result(with_requests).expect("input_required");
        assert_eq!(parsed.keys(), vec!["github_login".to_string()]);
        assert_eq!(parsed.request_state, None);

        let complete =
            r#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","content":[]}}"#;
        assert!(input_required_from_result(complete).is_none());
        // 2025-era results have no resultType at all.
        assert!(input_required_from_result(r#"{"result":{"content":[]}}"#).is_none());
    }

    #[test]
    fn retry_body_echoes_request_state_verbatim() {
        let mut responses = BTreeMap::new();
        responses.insert("connect".to_string(), json!({ "action": "accept" }));
        let body = tools_call_retry_body(
            2,
            "search",
            &json!({"q": "x"}),
            MCP_PROTOCOL_VERSION_2026_07,
            ClientCapabilities {
                url_elicitation: true,
            },
            Some("AEAD-blob=="),
            &responses,
        );
        assert_eq!(body["id"], 2);
        assert_eq!(body["params"]["requestState"], "AEAD-blob==");
        assert_eq!(body["params"]["arguments"]["q"], "x");
        assert_eq!(
            body["params"]["inputResponses"]["connect"]["action"],
            "accept"
        );

        // No state offered means none echoed back, and no responses gathered
        // means no `inputResponses` key at all.
        let bare = tools_call_retry_body(
            2,
            "search",
            &json!({}),
            MCP_PROTOCOL_VERSION_2026_07,
            ClientCapabilities::none(),
            None,
            &BTreeMap::new(),
        );
        assert!(bare["params"].get("requestState").is_none());
        assert!(bare["params"].get("inputResponses").is_none());
    }

    #[test]
    fn cache_hints_parse_from_result() {
        let public = r#"{"result":{"tools":[],"ttlMs":300000,"cacheScope":"public"}}"#;
        let hints = cache_hints_from_result(public).expect("hints");
        assert_eq!(hints.ttl, Duration::from_millis(300_000));
        assert_eq!(hints.scope, CacheScope::Public);

        // Missing scope is read as private — never over-share by default.
        let unscoped = r#"{"result":{"tools":[],"ttlMs":1000}}"#;
        assert_eq!(
            cache_hints_from_result(unscoped).unwrap().scope,
            CacheScope::Private
        );

        // Zero, negative, and absent all mean "immediately stale".
        assert!(cache_hints_from_result(r#"{"result":{"ttlMs":0}}"#).is_none());
        assert!(cache_hints_from_result(r#"{"result":{"ttlMs":-5}}"#).is_none());
        assert!(cache_hints_from_result(r#"{"result":{"tools":[]}}"#).is_none());
    }
}
