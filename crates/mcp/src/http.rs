//! HTTP (Streamable-HTTP) MCP transport with multi-era protocol support.
//!
//! Lifted from `worker/src/mcp_executor.rs::call_mcp_tool` and
//! `server/.../service.rs::fetch_mcp_tools` so the DNS-pinned SSRF contract
//! (TM-TOOL-018) and SSE/JSON handling are shared, not duplicated
//! (specs/runtime-mcp.md D1/D5). The free functions let callers that only hold
//! a `&dyn EgressService` (e.g. the control plane) reuse the exact same path.
//!
//! The transport speaks all three MCP eras through one code path
//! (specs/mcp-servers.md "Multi-era protocol support"): it emits `_meta` and
//! routable headers on every request (additive for older servers), and for
//! `Auto`/stateful servers it runs the `initialize` handshake and echoes the
//! `Mcp-Session-Id`. See [`crate::protocol`] for the pure pieces.

use crate::auth::McpCredential;
use crate::protocol::{self, Negotiated};
use crate::result::extract_json_from_response;
use crate::transport::{McpConnection, McpEndpoint, McpTransport};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use everruns_core::{
    DirectEgressService, EgressRequest, EgressRequestKind, EgressService, McpProtocolMode,
    McpToolCallResponse, McpToolCallResult, McpToolDefinition, McpToolsListResponse,
    normalize_mcp_error_code, validate_url_dns_pinned,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Timeout for tool discovery (`tools/list`). Kept shorter than execution so a
/// slow server doesn't stall turn-context assembly.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for tool execution (`tools/call`).
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a per-transport negotiation verdict stays valid before re-probing.
const NEGOTIATION_TTL: Duration = Duration::from_secs(300);

/// Raw MCP HTTP response: status, headers, and the (possibly SSE-framed) body
/// text — without the 2xx check, so negotiation can inspect failures.
pub struct RawMcpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

/// Send one MCP JSON-RPC request and return the raw response (no status check).
/// Performs DNS-pinned SSRF validation (TM-TOOL-018). `extra_headers` carry
/// per-request protocol headers (routable headers, session id) on top of the
/// server's configured `headers`.
async fn send_raw(
    egress: &dyn EgressService,
    url: &str,
    headers: &HashMap<String, String>,
    extra_headers: &[(String, String)],
    credential: Option<&McpCredential>,
    body: Vec<u8>,
    timeout: Duration,
) -> Result<RawMcpResponse> {
    let (validated_url, resolved_addrs) = validate_url_dns_pinned(url).await.map_err(|e| {
        tracing::warn!(url = %url, error = %e, "Blocked MCP request: URL failed SSRF validation");
        anyhow!("MCP server URL blocked: {}", e)
    })?;
    let pin_host = validated_url.host_str().unwrap_or("").to_string();

    let mut request = EgressRequest::new("POST", url, EgressRequestKind::Mcp)
        .pinned_addrs(pin_host, resolved_addrs)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .timeout_ms(timeout.as_millis() as u64)
        .body(body);

    for (name, value) in headers {
        request = request.header(name, value);
    }
    for (name, value) in extra_headers {
        request = request.header(name, value);
    }
    if let Some(credential) = credential {
        if let Some(auth) = &credential.authorization {
            request = request.header("Authorization", auth.clone());
        }
        for (name, value) in &credential.headers {
            request = request.header(name, value.clone());
        }
    }

    let response = egress
        .send(request)
        .await
        .map_err(|e| anyhow!("Failed to call MCP server: {}", e))?;

    let body = String::from_utf8(response.body)
        .map_err(|e| anyhow!("Failed to read MCP response body: {}", e))?;
    Ok(RawMcpResponse {
        status: response.status,
        headers: response.headers,
        body,
    })
}

/// Send one MCP JSON-RPC request over HTTP and return the (possibly SSE-framed)
/// response body, erroring on non-2xx. Stateless one-shot — kept for callers
/// that drive their own request shape. Performs DNS-pinned SSRF validation.
pub async fn http_send_rpc(
    egress: &dyn EgressService,
    url: &str,
    headers: &HashMap<String, String>,
    credential: Option<&McpCredential>,
    body: Vec<u8>,
    timeout: Duration,
) -> Result<String> {
    let response = send_raw(egress, url, headers, &[], credential, body, timeout).await?;
    if !(200..300).contains(&response.status) {
        return Err(anyhow!(
            "MCP server returned error: {} - {}",
            response.status,
            response.body
        ));
    }
    Ok(response.body)
}

/// Run the `initialize` handshake for a stateful (legacy/current) server and
/// return the negotiated protocol (server-reported version + captured session
/// id). Also fires a best-effort `notifications/initialized`.
async fn do_handshake(
    egress: &dyn EgressService,
    url: &str,
    headers: &HashMap<String, String>,
    credential: Option<&McpCredential>,
    preferred_version: &str,
    timeout: Duration,
) -> Result<Negotiated> {
    let body = serde_json::to_vec(&protocol::initialize_body(0, preferred_version))?;
    let extra = protocol::routable_headers(preferred_version, "initialize", None);
    let response = send_raw(egress, url, headers, &extra, credential, body, timeout).await?;
    if !(200..300).contains(&response.status) {
        return Err(anyhow!(
            "MCP initialize handshake failed: {} - {}",
            response.status,
            response.body
        ));
    }
    // The initialize result may be plain JSON or SSE-framed, like any MCP
    // response — extract the JSON payload before reading the negotiated version.
    let init_json = extract_json_from_response(&response.body).unwrap_or(&response.body);
    let version = protocol::protocol_version_from_initialize(init_json)
        .unwrap_or_else(|| preferred_version.to_string());
    let session_id = protocol::session_id_from_headers(&response.headers);

    // Best-effort `notifications/initialized` — some servers require it; ignore
    // failures since the handshake itself already succeeded.
    if let Ok(note) = serde_json::to_vec(&protocol::initialized_notification()) {
        let mut note_extra =
            protocol::routable_headers(&version, "notifications/initialized", None);
        if let Some(session_id) = &session_id {
            note_extra.push((protocol::HEADER_SESSION_ID.to_string(), session_id.clone()));
        }
        let _ = send_raw(egress, url, headers, &note_extra, credential, note, timeout).await;
    }

    Ok(Negotiated {
        version,
        stateful: true,
        session_id,
    })
}

/// Send an already-built operation body under a negotiated protocol, attaching
/// routable headers and the session id when stateful.
#[allow(clippy::too_many_arguments)]
async fn send_op(
    egress: &dyn EgressService,
    url: &str,
    headers: &HashMap<String, String>,
    credential: Option<&McpCredential>,
    negotiated: &Negotiated,
    method: &str,
    tool_name: Option<&str>,
    body: Vec<u8>,
    timeout: Duration,
) -> Result<RawMcpResponse> {
    let mut extra = protocol::routable_headers(&negotiated.version, method, tool_name);
    if let Some(session_id) = &negotiated.session_id {
        extra.push((protocol::HEADER_SESSION_ID.to_string(), session_id.clone()));
    }
    send_raw(egress, url, headers, &extra, credential, body, timeout).await
}

/// Drive one operation through protocol negotiation, returning the response
/// body text and the negotiated protocol used (so a caller can cache it).
///
/// `Auto` starts stateless (RC) and, on a failure that signals the server needs
/// a session, transparently runs the handshake and retries — this is what makes
/// legacy, current, and RC servers all work without operator action.
#[allow(clippy::too_many_arguments)]
async fn negotiate_and_send(
    egress: &dyn EgressService,
    url: &str,
    headers: &HashMap<String, String>,
    credential: Option<&McpCredential>,
    mode: McpProtocolMode,
    method: &str,
    tool_name: Option<&str>,
    op_body: Vec<u8>,
    cached: Option<Negotiated>,
    timeout: Duration,
) -> Result<(String, Negotiated)> {
    let mut negotiated = cached.unwrap_or_else(|| Negotiated::initial_for_mode(mode));

    // A stateful negotiation with no session id yet must handshake first. This
    // covers pinned Legacy/Stable and a cached stateful verdict that lost its
    // session.
    if negotiated.stateful && negotiated.session_id.is_none() {
        negotiated = do_handshake(
            egress,
            url,
            headers,
            credential,
            &negotiated.version,
            timeout,
        )
        .await?;
    }

    let response = send_op(
        egress,
        url,
        headers,
        credential,
        &negotiated,
        method,
        tool_name,
        op_body.clone(),
        timeout,
    )
    .await?;

    // Auto fallback: a stateless attempt the server rejected because it wants a
    // session — handshake, then retry once.
    let response = if mode == McpProtocolMode::Auto
        && !negotiated.stateful
        && !(200..300).contains(&response.status)
        && protocol::looks_like_handshake_required(response.status, &response.body)
    {
        tracing::debug!(
            url = %url,
            "MCP stateless attempt rejected; falling back to stateful handshake"
        );
        negotiated = do_handshake(
            egress,
            url,
            headers,
            credential,
            protocol::DEFAULT_STATEFUL_VERSION,
            timeout,
        )
        .await?;
        send_op(
            egress,
            url,
            headers,
            credential,
            &negotiated,
            method,
            tool_name,
            op_body,
            timeout,
        )
        .await?
    } else {
        response
    };

    if !(200..300).contains(&response.status) {
        return Err(anyhow!(
            "MCP server returned error: {} - {}",
            response.status,
            response.body
        ));
    }

    Ok((response.body, negotiated))
}

/// Parse a `tools/list` response body into tool definitions.
fn parse_tools_list(text: &str) -> Result<Vec<McpToolDefinition>> {
    let json_str = extract_json_from_response(text)
        .ok_or_else(|| anyhow!("SSE response missing data line"))?;
    let response: McpToolsListResponse = serde_json::from_str(json_str)?;
    if let Some(error) = response.error {
        return Err(anyhow!(
            "MCP server error: {} ({})",
            error.message,
            normalize_mcp_error_code(error.code)
        ));
    }
    Ok(response
        .result
        .ok_or_else(|| anyhow!("MCP server returned empty result"))?
        .tools)
}

/// Parse a `tools/call` response body into a tool-call result.
fn parse_tool_call(text: &str) -> Result<McpToolCallResult> {
    let json_str = extract_json_from_response(text)
        .ok_or_else(|| anyhow!("SSE response missing data line"))?;
    let response: McpToolCallResponse = serde_json::from_str(json_str)?;
    if let Some(error) = response.error {
        return Err(anyhow!(
            "MCP tool error: {} (code: {})",
            error.message,
            normalize_mcp_error_code(error.code)
        ));
    }
    response
        .result
        .ok_or_else(|| anyhow!("MCP server returned empty result"))
}

/// Discover a server's tools via `tools/list`. Negotiates the protocol era
/// (`Auto`); one-shot with no persistent cache (the control-plane caller layers
/// its own tool cache on top).
pub async fn http_list_tools(
    egress: &dyn EgressService,
    url: &str,
    headers: &HashMap<String, String>,
    credential: Option<&McpCredential>,
) -> Result<Vec<McpToolDefinition>> {
    let body = serde_json::to_vec(&protocol::tools_list_body(1))?;
    let (text, _negotiated) = negotiate_and_send(
        egress,
        url,
        headers,
        credential,
        McpProtocolMode::Auto,
        "tools/list",
        None,
        body,
        None,
        DISCOVERY_TIMEOUT,
    )
    .await?;
    parse_tools_list(&text)
}

/// Execute a tool via `tools/call`. Negotiates the protocol era (`Auto`).
pub async fn http_call_tool(
    egress: &dyn EgressService,
    url: &str,
    headers: &HashMap<String, String>,
    tool_name: &str,
    arguments: Value,
    credential: Option<&McpCredential>,
) -> Result<McpToolCallResult> {
    let body = serde_json::to_vec(&protocol::tools_call_body(1, tool_name, &arguments))?;
    let (text, _negotiated) = negotiate_and_send(
        egress,
        url,
        headers,
        credential,
        McpProtocolMode::Auto,
        "tools/call",
        Some(tool_name),
        body,
        None,
        CALL_TIMEOUT,
    )
    .await?;
    parse_tool_call(&text)
}

/// MCP transport over the platform [`EgressService`] boundary.
///
/// Holds a small negotiation cache scoped by URL plus logical connection and
/// auth context so a `tools/list` followed by a `tools/call` (the common turn
/// pattern) reuses its own negotiated era and session id without crossing
/// credentials or pinned protocol modes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct NegotiationCacheKey {
    url: String,
    server_name: String,
    protocol_mode: String,
    headers_hash: u64,
    credential_hash: u64,
}

impl NegotiationCacheKey {
    fn new(
        connection: &McpConnection,
        url: &str,
        headers: &HashMap<String, String>,
        credential: Option<&McpCredential>,
    ) -> Self {
        Self {
            url: url.to_string(),
            server_name: connection.name.clone(),
            protocol_mode: connection.protocol_mode.to_string(),
            headers_hash: hash_headers(headers),
            credential_hash: hash_credential(credential),
        }
    }
}

fn hash_headers(headers: &HashMap<String, String>) -> u64 {
    let mut hasher = DefaultHasher::new();
    let sorted: BTreeMap<_, _> = headers.iter().collect();
    sorted.hash(&mut hasher);
    hasher.finish()
}

fn hash_credential(credential: Option<&McpCredential>) -> u64 {
    let mut hasher = DefaultHasher::new();
    credential
        .and_then(|credential| credential.authorization.as_deref())
        .hash(&mut hasher);
    if let Some(credential) = credential {
        let sorted: BTreeMap<_, _> = credential.headers.iter().collect();
        sorted.hash(&mut hasher);
    }
    hasher.finish()
}

pub struct HttpTransport {
    egress: Arc<dyn EgressService>,
    negotiations: Mutex<HashMap<NegotiationCacheKey, (Negotiated, Instant)>>,
}

impl HttpTransport {
    pub fn new(egress: Arc<dyn EgressService>) -> Self {
        Self {
            egress,
            negotiations: Mutex::new(HashMap::new()),
        }
    }

    /// Build a transport backed by a [`DirectEgressService`] (real outbound
    /// HTTP). Used by hosts without a custom egress boundary, e.g. the CLI.
    pub fn direct() -> Self {
        Self::new(Arc::new(DirectEgressService::default()))
    }

    fn http_parts(connection: &McpConnection) -> Result<(&str, &HashMap<String, String>)> {
        match &connection.endpoint {
            McpEndpoint::Http { url, headers } => Ok((url.as_str(), headers)),
            #[cfg(feature = "stdio")]
            _ => Err(anyhow!(
                "HttpTransport received a non-HTTP endpoint for server '{}'",
                connection.name
            )),
        }
    }

    /// Fresh cached negotiation for one logical connection/auth scope, or
    /// `None` if absent/expired.
    fn cached_negotiation(&self, key: &NegotiationCacheKey) -> Option<Negotiated> {
        let cache = self.negotiations.lock().ok()?;
        let (negotiated, at) = cache.get(key)?;
        (at.elapsed() < NEGOTIATION_TTL).then(|| negotiated.clone())
    }

    fn store_negotiation(&self, key: NegotiationCacheKey, negotiated: Negotiated) {
        if let Ok(mut cache) = self.negotiations.lock() {
            cache.insert(key, (negotiated, Instant::now()));
        }
    }
}

#[async_trait]
impl McpTransport for HttpTransport {
    async fn list_tools(
        &self,
        connection: &McpConnection,
        credential: Option<&McpCredential>,
    ) -> Result<Vec<McpToolDefinition>> {
        let (url, headers) = Self::http_parts(connection)?;
        let cache_key = NegotiationCacheKey::new(connection, url, headers, credential);
        let cached = self.cached_negotiation(&cache_key);
        let body = serde_json::to_vec(&protocol::tools_list_body(1))?;
        let (text, negotiated) = negotiate_and_send(
            self.egress.as_ref(),
            url,
            headers,
            credential,
            connection.protocol_mode,
            "tools/list",
            None,
            body,
            cached,
            DISCOVERY_TIMEOUT,
        )
        .await?;
        self.store_negotiation(cache_key, negotiated);
        parse_tools_list(&text)
    }

    async fn call_tool(
        &self,
        connection: &McpConnection,
        tool_name: &str,
        arguments: Value,
        credential: Option<&McpCredential>,
    ) -> Result<McpToolCallResult> {
        let (url, headers) = Self::http_parts(connection)?;
        let cache_key = NegotiationCacheKey::new(connection, url, headers, credential);
        let cached = self.cached_negotiation(&cache_key);
        let body = serde_json::to_vec(&protocol::tools_call_body(1, tool_name, &arguments))?;
        let (text, negotiated) = negotiate_and_send(
            self.egress.as_ref(),
            url,
            headers,
            credential,
            connection.protocol_mode,
            "tools/call",
            Some(tool_name),
            body,
            cached,
            CALL_TIMEOUT,
        )
        .await?;
        self.store_negotiation(cache_key, negotiated);
        parse_tool_call(&text)
    }
}
