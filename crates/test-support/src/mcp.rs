//! In-memory MCP resource and OAuth authorization server for integration tests.
//!
//! The harness implements [`EgressService`] instead of opening a socket. Tests
//! exercise the production egress boundary without DNS, credentials, fixed
//! ports, shared global state, or ordering constraints.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use everruns_core::{
    EgressError, EgressRequest, EgressResponse, EgressResult, EgressService,
    EgressStreamResponse,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MCP_HOST: &str = "8.8.8.8";

/// MCP protocol era emulated by the resource server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockMcpProtocolEra {
    /// Stateful `2025-03-26`.
    V2025March,
    /// Stateful `2025-06-18`.
    V2025June,
    /// Stateless `2026-07-28`.
    V2026July,
}

impl MockMcpProtocolEra {
    /// Wire version for this era.
    pub fn version(self) -> &'static str {
        match self {
            Self::V2025March => "2025-03-26",
            Self::V2025June => "2025-06-18",
            Self::V2026July => "2026-07-28",
        }
    }

    fn is_stateful(self) -> bool {
        !matches!(self, Self::V2026July)
    }
}

/// Configurable response to a `tools/call`.
#[derive(Debug, Clone)]
pub enum MockCallResponse {
    /// Return a completed MCP tool result.
    Complete(Value),
    /// Require one MRTR retry before returning `complete`.
    InputRequired {
        /// Opaque state echoed by the client.
        request_state: String,
        /// Result returned after the retry.
        result: Value,
    },
    /// Request URL-mode elicitation until the client responds with `accept`.
    UrlElicitation {
        /// Opaque state echoed by the client.
        request_state: String,
        /// HTTPS URL shown to the human.
        url: String,
        /// Human-readable reason for the interaction.
        message: String,
        /// Result returned after acceptance.
        result: Value,
    },
}

impl Default for MockCallResponse {
    fn default() -> Self {
        Self::Complete(json!({
            "content": [{"type": "text", "text": "ok"}],
            "isError": false
        }))
    }
}

/// One MCP request observed at the egress boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedMcpRequest {
    /// JSON-RPC method.
    pub method: String,
    /// Complete request headers.
    pub headers: BTreeMap<String, String>,
    /// Parsed JSON-RPC body.
    pub body: Value,
}

impl RecordedMcpRequest {
    /// Case-insensitive header lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// One OAuth request observed at the egress boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedOAuthRequest {
    /// HTTP method.
    pub method: String,
    /// Absolute request URL.
    pub url: String,
    /// Raw request body.
    pub body: Vec<u8>,
}

/// Authorization failures produced by the mock policy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MockOAuthError {
    /// The configured server rejects application actors.
    #[error("actor parameter rejected")]
    ActorRejected,
    /// The configured server rejects this scope.
    #[error("scope rejected: {0}")]
    ScopeRejected(String),
}

#[derive(Debug, Clone)]
struct AuthorizationCode {
    challenge: String,
    actor: Option<String>,
    scope: Option<String>,
}

#[derive(Debug)]
struct State {
    era: MockMcpProtocolEra,
    session_id: String,
    tools: Vec<Value>,
    cache_hints: Option<(i64, String)>,
    call_responses: HashMap<String, MockCallResponse>,
    mcp_requests: Vec<RecordedMcpRequest>,
    oauth_requests: Vec<RecordedOAuthRequest>,
    authorization_codes: HashMap<String, AuthorizationCode>,
    active_refresh_tokens: HashSet<String>,
    token_counter: u64,
    reject_actor: bool,
    rejected_scopes: HashSet<String>,
}

/// Shared mock MCP resource and OAuth authorization server.
///
/// Clone the value to hand the same isolated state to a client and assertions.
/// Each instance has unique endpoint paths and a unique MCP session ID.
#[derive(Clone)]
pub struct MockMcpOAuthServer {
    id: String,
    oauth_origin: String,
    state: Arc<Mutex<State>>,
}

impl Default for MockMcpOAuthServer {
    fn default() -> Self {
        Self::new(MockMcpProtocolEra::V2026July)
    }
}

impl MockMcpOAuthServer {
    /// Create an isolated harness for one MCP protocol era.
    pub fn new(era: MockMcpProtocolEra) -> Self {
        let uuid = Uuid::new_v4();
        let id = uuid.simple().to_string();
        let port = 10_000 + (uuid.as_u128() % 50_000) as u16;
        Self {
            oauth_origin: format!("http://127.0.0.1:{port}"),
            id,
            state: Arc::new(Mutex::new(State {
                era,
                session_id: format!("session-{uuid}"),
                tools: vec![json!({
                    "name": "echo",
                    "description": "Echo a value",
                    "inputSchema": {"type": "object"}
                })],
                cache_hints: None,
                call_responses: HashMap::new(),
                mcp_requests: Vec::new(),
                oauth_requests: Vec::new(),
                authorization_codes: HashMap::new(),
                active_refresh_tokens: HashSet::new(),
                token_counter: 0,
                reject_actor: false,
                rejected_scopes: HashSet::new(),
            })),
        }
    }

    /// Public IP-literal resource URL used by the MCP client.
    ///
    /// No connection reaches this address: this harness itself is the injected
    /// egress service. The IP literal lets production SSRF validation run
    /// without DNS, matching the existing MCP transport tests.
    pub fn mcp_url(&self) -> String {
        format!("https://{MCP_HOST}/mock/{}/mcp", self.id)
    }

    /// Loopback OAuth issuer URL. No socket is opened.
    pub fn oauth_issuer(&self) -> &str {
        &self.oauth_origin
    }

    /// OAuth token endpoint.
    pub fn token_endpoint(&self) -> String {
        format!("{}/token", self.oauth_origin)
    }

    /// OAuth revocation endpoint.
    pub fn revocation_endpoint(&self) -> String {
        format!("{}/revoke", self.oauth_origin)
    }

    /// Replace the `tools/list` payload.
    pub fn set_tools(&self, tools: Vec<Value>) {
        self.lock().tools = tools;
    }

    /// Add `ttlMs` and `cacheScope` to `tools/list`.
    pub fn set_cache_hints(&self, ttl_ms: i64, cache_scope: impl Into<String>) {
        self.lock().cache_hints = Some((ttl_ms, cache_scope.into()));
    }

    /// Configure one tool's call behavior.
    pub fn set_call_response(&self, tool: impl Into<String>, response: MockCallResponse) {
        self.lock().call_responses.insert(tool.into(), response);
    }

    /// Reject OAuth authorization requests carrying any actor parameter.
    pub fn reject_actor(&self) {
        self.lock().reject_actor = true;
    }

    /// Reject one OAuth scope.
    pub fn reject_scope(&self, scope: impl Into<String>) {
        self.lock().rejected_scopes.insert(scope.into());
    }

    /// Mint a one-time authorization code bound to a PKCE S256 challenge.
    pub fn authorize(
        &self,
        code_challenge: impl Into<String>,
        actor: Option<&str>,
        scope: Option<&str>,
    ) -> Result<String, MockOAuthError> {
        let mut state = self.lock();
        if state.reject_actor && actor.is_some() {
            return Err(MockOAuthError::ActorRejected);
        }
        if let Some(scope) = scope
            && state.rejected_scopes.contains(scope)
        {
            return Err(MockOAuthError::ScopeRejected(scope.to_string()));
        }
        let code = format!("code-{}", Uuid::new_v4());
        state.authorization_codes.insert(
            code.clone(),
            AuthorizationCode {
                challenge: code_challenge.into(),
                actor: actor.map(str::to_string),
                scope: scope.map(str::to_string),
            },
        );
        Ok(code)
    }

    /// MCP requests observed so far.
    pub fn mcp_requests(&self) -> Vec<RecordedMcpRequest> {
        self.lock().mcp_requests.clone()
    }

    /// OAuth requests observed so far.
    pub fn oauth_requests(&self) -> Vec<RecordedOAuthRequest> {
        self.lock().oauth_requests.clone()
    }

    /// Authorization headers on `tools/list` and `tools/call`, in call order.
    pub fn authorization_headers(&self) -> Vec<Option<String>> {
        self.lock()
            .mcp_requests
            .iter()
            .filter(|request| matches!(request.method.as_str(), "tools/list" | "tools/call"))
            .map(|request| request.header("authorization").map(str::to_string))
            .collect()
    }

    /// Assert that the latest MCP operation used the invoking user's token.
    pub fn assert_called_as_user(&self, access_token: &str) {
        self.assert_latest_authorization(Some(access_token), "user");
    }

    /// Assert that the latest MCP operation used the agent identity's token.
    pub fn assert_called_as_identity(&self, access_token: &str) {
        self.assert_latest_authorization(Some(access_token), "identity");
    }

    /// Assert that the latest MCP operation carried no authorization header.
    pub fn assert_no_authorization_header(&self) {
        self.assert_latest_authorization(None, "anonymous");
    }

    fn assert_latest_authorization(&self, token: Option<&str>, principal: &str) {
        let headers = self.authorization_headers();
        let actual = headers
            .last()
            .unwrap_or_else(|| panic!("no MCP operation was recorded"));
        let expected = token.map(|token| format!("Bearer {token}"));
        assert_eq!(
            actual.as_deref(),
            expected.as_deref(),
            "latest MCP call did not use the expected {principal} credential"
        );
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn json_response(status: u16, body: Value) -> EgressResponse {
        EgressResponse {
            status,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&body).unwrap_or_default(),
        }
    }

    fn invalid_grant() -> EgressResponse {
        Self::json_response(400, json!({"error": "invalid_grant"}))
    }

    fn handle_mcp(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let body: Value = serde_json::from_slice(&request.body)
            .map_err(|error| EgressError::invalid(format!("invalid MCP JSON: {error}")))?;
        let method = body["method"].as_str().unwrap_or_default().to_string();
        let id = body.get("id").cloned().unwrap_or(Value::Null);
        let mut state = self.lock();
        state.mcp_requests.push(RecordedMcpRequest {
            method: method.clone(),
            headers: request.headers.clone(),
            body: body.clone(),
        });

        if method == "initialize" {
            let mut response = Self::json_response(
                200,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": state.era.version(),
                        "capabilities": {}
                    }
                }),
            );
            response
                .headers
                .insert("Mcp-Session-Id".to_string(), state.session_id.clone());
            return Ok(response);
        }
        if method == "notifications/initialized" {
            return Ok(EgressResponse {
                status: 202,
                headers: BTreeMap::new(),
                body: Vec::new(),
            });
        }

        if state.era.is_stateful() && header(&request.headers, "Mcp-Session-Id").is_none() {
            return Ok(EgressResponse {
                status: 400,
                headers: BTreeMap::new(),
                body: b"Bad Request: Mcp-Session-Id header is required".to_vec(),
            });
        }

        match method.as_str() {
            "tools/list" => {
                let mut result = json!({"tools": state.tools});
                if let Some((ttl_ms, scope)) = &state.cache_hints {
                    result["resultType"] = json!("complete");
                    result["ttlMs"] = json!(ttl_ms);
                    result["cacheScope"] = json!(scope);
                }
                Ok(Self::json_response(
                    200,
                    json!({"jsonrpc": "2.0", "id": id, "result": result}),
                ))
            }
            "tools/call" => {
                let tool = body["params"]["name"].as_str().unwrap_or_default();
                let response = state
                    .call_responses
                    .get(tool)
                    .cloned()
                    .unwrap_or_default();
                let retried = body["params"].get("requestState").is_some();
                let accepted = body["params"]["inputResponses"]
                    .as_object()
                    .is_some_and(|responses| {
                        responses
                            .values()
                            .any(|response| response["action"] == "accept")
                    });
                let result = match response {
                    MockCallResponse::Complete(result) => result,
                    MockCallResponse::InputRequired { result, .. } if retried => result,
                    MockCallResponse::InputRequired { request_state, .. } => json!({
                        "resultType": "input_required",
                        "requestState": request_state
                    }),
                    MockCallResponse::UrlElicitation { result, .. } if accepted => result,
                    MockCallResponse::UrlElicitation {
                        request_state,
                        url,
                        message,
                        ..
                    } => json!({
                        "resultType": "input_required",
                        "requestState": request_state,
                        "inputRequests": {
                            "connect": {
                                "method": "elicitation/create",
                                "params": {
                                    "mode": "url",
                                    "url": url,
                                    "message": message
                                }
                            }
                        }
                    }),
                };
                Ok(Self::json_response(
                    200,
                    json!({"jsonrpc": "2.0", "id": id, "result": result}),
                ))
            }
            _ => Ok(Self::json_response(
                200,
                json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            )),
        }
    }

    fn handle_oauth(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let mut state = self.lock();
        state.oauth_requests.push(RecordedOAuthRequest {
            method: request.method.clone(),
            url: request.url.clone(),
            body: request.body.clone(),
        });

        let suffix = request.url.strip_prefix(&self.oauth_origin).unwrap_or_default();
        match (request.method.as_str(), suffix) {
            ("GET", path) if path.starts_with("/.well-known/oauth-protected-resource") => {
                Ok(Self::json_response(
                    200,
                    json!({
                        "resource": self.mcp_url(),
                        "authorization_servers": [self.oauth_origin]
                    }),
                ))
            }
            ("GET", path) if path.starts_with("/.well-known/oauth-authorization-server") => {
                Ok(Self::json_response(
                    200,
                    json!({
                        "issuer": self.oauth_origin,
                        "authorization_endpoint": format!("{}/authorize", self.oauth_origin),
                        "token_endpoint": self.token_endpoint(),
                        "registration_endpoint": format!("{}/register", self.oauth_origin),
                        "revocation_endpoint": self.revocation_endpoint(),
                        "code_challenge_methods_supported": ["S256"]
                    }),
                ))
            }
            ("POST", "/register") => Ok(Self::json_response(
                201,
                json!({"client_id": format!("client-{}", self.id)}),
            )),
            ("POST", "/token") => {
                let form: BTreeMap<String, String> = serde_urlencoded::from_bytes(&request.body)
                    .map_err(|error| {
                        EgressError::invalid(format!("invalid OAuth token form: {error}"))
                    })?;
                match form.get("grant_type").map(String::as_str) {
                    Some("authorization_code") => {
                        let Some(code) = form.get("code") else {
                            return Ok(Self::invalid_grant());
                        };
                        let Some(grant) = state.authorization_codes.remove(code) else {
                            return Ok(Self::invalid_grant());
                        };
                        let verifier = form.get("code_verifier").map(String::as_str).unwrap_or("");
                        if pkce_challenge(verifier) != grant.challenge {
                            return Ok(Self::invalid_grant());
                        }
                        if state.reject_actor && grant.actor.is_some() {
                            return Ok(Self::json_response(
                                400,
                                json!({"error": "invalid_request", "error_description": "actor rejected"}),
                            ));
                        }
                        if grant
                            .scope
                            .as_ref()
                            .is_some_and(|scope| state.rejected_scopes.contains(scope))
                        {
                            return Ok(Self::json_response(
                                400,
                                json!({"error": "invalid_scope"}),
                            ));
                        }
                        Ok(issue_tokens(&mut state, grant.scope))
                    }
                    Some("refresh_token") => {
                        let Some(refresh_token) = form.get("refresh_token") else {
                            return Ok(Self::invalid_grant());
                        };
                        if !state.active_refresh_tokens.remove(refresh_token) {
                            return Ok(Self::invalid_grant());
                        }
                        Ok(issue_tokens(&mut state, None))
                    }
                    _ => Ok(Self::json_response(
                        400,
                        json!({"error": "unsupported_grant_type"}),
                    )),
                }
            }
            ("POST", "/revoke") => {
                let form: BTreeMap<String, String> = serde_urlencoded::from_bytes(&request.body)
                    .map_err(|error| {
                        EgressError::invalid(format!("invalid OAuth revoke form: {error}"))
                    })?;
                if let Some(token) = form.get("token") {
                    state.active_refresh_tokens.remove(token);
                }
                Ok(EgressResponse {
                    status: 200,
                    headers: BTreeMap::new(),
                    body: Vec::new(),
                })
            }
            _ => Ok(Self::json_response(404, json!({"error": "not_found"}))),
        }
    }
}

#[async_trait]
impl EgressService for MockMcpOAuthServer {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        if request.url == self.mcp_url() {
            self.handle_mcp(request)
        } else if request.url.starts_with(&format!(
            "https://{MCP_HOST}/.well-known/oauth-protected-resource/"
        )) {
            let mut state = self.lock();
            state.oauth_requests.push(RecordedOAuthRequest {
                method: request.method,
                url: request.url,
                body: request.body,
            });
            drop(state);
            Ok(Self::json_response(
                200,
                json!({
                    "resource": self.mcp_url(),
                    "authorization_servers": [self.oauth_origin]
                }),
            ))
        } else if request.url.starts_with(&self.oauth_origin) {
            self.handle_oauth(request)
        } else {
            Err(EgressError::Transport(format!(
                "mock has no route for {} {}",
                request.method, request.url
            )))
        }
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        Err(EgressError::Transport(
            "mock MCP/OAuth harness does not stream".to_string(),
        ))
    }

    fn name(&self) -> &'static str {
        "MockMcpOAuthServer"
    }
}

fn header<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn pkce_challenge(verifier: &str) -> String {
    use base64::Engine as _;

    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()).as_slice())
}

fn issue_tokens(state: &mut State, scope: Option<String>) -> EgressResponse {
    state.token_counter += 1;
    let access_token = format!("access-{}", state.token_counter);
    let refresh_token = format!("refresh-{}", state.token_counter);
    state.active_refresh_tokens.insert(refresh_token.clone());
    MockMcpOAuthServer::json_response(
        200,
        json!({
            "access_token": access_token,
            "refresh_token": refresh_token,
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": scope
        }),
    )
}
