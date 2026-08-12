// MCP Endpoint — exposes Everruns as an MCP server (Streamable HTTP transport)
//
// Design decisions:
// - JSON-RPC 2.0 over POST /mcp (Streamable HTTP, per MCP spec)
// - Tier 1 tools: agent_run, session_send_message, session_get_status
//   → Direct service calls, first-class support for the agent conversation loop
// - Tier 2 tools: discover, query, execute
//   → Backed by bashkit ScriptedTool with API operations exposed as builtins
//   → discover: reads inventory descriptors and returns schema-bearing JSON
//   → query: runs bash scripts with the read-only subset of API ops
//   → execute: runs bash scripts with the full API surface, including writes
// - Tier 0 tools: me, list_organizations
//   → Identity & org context tools for multi-org OAuth flows
//   → MCP clients can't set cookies, so org selection is via organization_id arguments
// - Auth: separate path from /api/* (TM-MCP-006). The McpAuthUser/McpResolvedOrg
//   extractors accept personal access tokens and resource-bound MCP OAuth tokens
//   (validate_mcp_token), plus anonymous in no-auth mode — never a regular
//   session/access JWT or cookie. Org context resolves the same as session auth.
// - No MCP session state — stateless request/response per JSON-RPC call.
//   This already satisfies the MCP 2026-07-28 stateless model (no
//   `Mcp-Session-Id`, no sticky sessions); any request can hit any instance.
// - Protocol versions: 2026-07-28, 2025-06-18, 2025-03-26 (fallback). 2026
//   dropped the `initialize` handshake — client info rides in per-request
//   `_meta` and routing rides in `Mcp-Method`/`Mcp-Name` headers — but we keep
//   `initialize` for older clients since it creates no server state either way.
// - Multi-org: org-scoped tools accept optional `organization_id` to override the default org

mod caching;
mod cards;
mod tasks;
mod tool_registry;

use crate::auth::AuthMethod;
use crate::auth::middleware::AuthUser;
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::budgets::BudgetService;
use crate::domains::common::{Command, CommandError, CommandErrorKind, Ctx};
use crate::domains::messages::MessageService;
use crate::domains::reporting::ReportingService;
use crate::domains::session_files::WorkspaceFileService;
use crate::domains::session_sandbox::SessionSandboxService;
use crate::domains::sessions::SessionService;
use crate::services::{CapabilityService, EventService};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::post,
};
use everruns_core::mcp_server::{McpErrorCode, McpExecuteError, classify_mcp_execute_error};
use everruns_core::{Caller, OrgRole};
use everruns_durable::WorkflowEventStore;
use everruns_host::HostComposition;
use everruns_platform::session_sqldb::SessionSqlDbStore;
use everruns_platform::validate_org_public_id;
use everruns_worker::AgentRunner;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use super::common::impl_auth_state;

pub(crate) mod catalog;
pub(crate) mod positional;

// ============================================================================
// JSON-RPC 2.0 types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }

    fn method_not_found(id: Option<Value>) -> Self {
        Self::error(id, -32601, "Method not found")
    }

    fn invalid_params(id: Option<Value>, msg: impl Into<String>) -> Self {
        Self::error(id, -32602, msg)
    }
}

// ============================================================================
// MCP Tool definitions
// ============================================================================

const MCP_SERVER_NAME: &str = "everruns";
const MCP_SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const MCP_PROTOCOL_VERSION_FALLBACK: &str = "2025-03-26";
const MCP_PROTOCOL_VERSION_2025_06: &str = "2025-06-18";
const MCP_PROTOCOL_VERSION_LATEST: &str = "2026-07-28";
// Newest first — negotiation and the "supported" error message both rely on
// this ordering reading high-to-low.
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    MCP_PROTOCOL_VERSION_LATEST,
    MCP_PROTOCOL_VERSION_2025_06,
    MCP_PROTOCOL_VERSION_FALLBACK,
];

/// The richer tool shape introduced in MCP 2025-06-18 — `title`,
/// `outputSchema`, `structuredContent`, and entity-card tools — applies to
/// that version and every later one (2026-07-28 is a superset). Only the
/// 2025-03-26 fallback omits it.
pub(crate) fn supports_rich_tool_shape(protocol_version: &str) -> bool {
    matches!(
        protocol_version,
        MCP_PROTOCOL_VERSION_2025_06 | MCP_PROTOCOL_VERSION_LATEST
    )
}

const ORG_ID_DESCRIPTION: &str = "Optional organization ID (format: org_{32-hex}). MCP has no current-organization switch; pass this on each org-scoped call to override the default organization. Use list_organizations to see available orgs.";

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct InitializeParams {
    #[serde(default)]
    protocol_version: Option<String>,
}

fn tool_definitions(protocol_version: &str) -> Vec<tool_registry::McpEndpointToolDefinition> {
    tool_registry::tool_definitions(protocol_version, ORG_ID_DESCRIPTION)
}

fn find_tool_definition(
    tool_name: &str,
    protocol_version: &str,
) -> Option<tool_registry::McpEndpointToolDefinition> {
    tool_registry::tool_definition(tool_name, protocol_version, ORG_ID_DESCRIPTION)
}

fn negotiate_protocol_version(requested: Option<&str>) -> &'static str {
    let Some(requested) = requested else {
        return MCP_PROTOCOL_VERSION_FALLBACK;
    };
    // Echo a version we support exactly.
    if let Some(version) = SUPPORTED_PROTOCOL_VERSIONS
        .iter()
        .copied()
        .find(|&v| v == requested)
    {
        return version;
    }
    // Unknown version: offer the newest version we support that is not newer
    // than what the client asked for. The date-string ordering is
    // lexicographic-safe (YYYY-MM-DD). A client ahead of us gets our latest
    // and decides whether to proceed.
    if requested > MCP_PROTOCOL_VERSION_LATEST {
        MCP_PROTOCOL_VERSION_LATEST
    } else if requested > MCP_PROTOCOL_VERSION_2025_06 {
        MCP_PROTOCOL_VERSION_2025_06
    } else {
        MCP_PROTOCOL_VERSION_FALLBACK
    }
}

fn protocol_version_from_headers(headers: &HeaderMap) -> Result<&'static str, String> {
    match headers.get("MCP-Protocol-Version") {
        None => Ok(MCP_PROTOCOL_VERSION_FALLBACK),
        Some(value) => {
            let version = value
                .to_str()
                .map_err(|_| "Invalid MCP-Protocol-Version header".to_string())?;
            SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .copied()
                .find(|&v| v == version)
                .ok_or_else(|| {
                    format!(
                        "Unsupported MCP-Protocol-Version: {version}. Supported: {}",
                        SUPPORTED_PROTOCOL_VERSIONS.join(", ")
                    )
                })
        }
    }
}

/// MCP 2026-07-28 removed the `initialize`/`initialized` handshake (SEP-2575):
/// protocol version, client info, and capabilities now ride on every request.
/// Client info arrives in `params._meta["io.modelcontextprotocol/clientInfo"]`.
/// The endpoint is stateless and stores nothing — we surface it for telemetry
/// only. Returns `(name, version)`, defaulting either field to `"unknown"`.
fn client_info_from_params(params: &Value) -> Option<(String, String)> {
    let info = params
        .get("_meta")
        .and_then(|meta| meta.get("io.modelcontextprotocol/clientInfo"))?;
    let name = info
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let version = info
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    Some((name, version))
}

/// MCP 2026-07-28 Streamable HTTP adds the `Mcp-Method` and `Mcp-Name` request
/// headers (SEP) so load balancers, gateways, and rate-limiters can route on
/// the operation without parsing the JSON-RPC body. They are optional and the
/// body stays authoritative, but when a client sends them they MUST agree with
/// the body — a disagreement is a request-smuggling signal, so we reject it.
fn single_routing_header<'a>(
    headers: &'a HeaderMap,
    name: &str,
) -> Result<Option<&'a str>, String> {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(format!("Duplicate {name} header"));
    }
    value
        .to_str()
        .map(Some)
        .map_err(|_| format!("Invalid {name} header"))
}

fn validate_routing_headers(headers: &HeaderMap, req: &JsonRpcRequest) -> Result<(), String> {
    if let Some(header_method) = single_routing_header(headers, "Mcp-Method")?
        && header_method != req.method
    {
        return Err(format!(
            "Mcp-Method header ({header_method}) does not match request method ({})",
            req.method
        ));
    }
    if let Some(header_name) = single_routing_header(headers, "Mcp-Name")? {
        // `Mcp-Name` identifies the specific tool/resource/prompt. We can only
        // cross-check it on `tools/call`, where the body carries `params.name`.
        if req.method == "tools/call"
            && let Some(body_name) = req.params.get("name").and_then(Value::as_str)
            && body_name != header_name
        {
            return Err(format!(
                "Mcp-Name header ({header_name}) does not match tool name ({body_name})"
            ));
        }
    }
    Ok(())
}

// ============================================================================
// App State
// ============================================================================

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: Arc<EventService>,
    pub session_file_service: Arc<WorkspaceFileService>,
    pub session_sandbox_service: Option<Arc<SessionSandboxService>>,
    pub capability_service: Arc<CapabilityService>,
    pub connector_registry: everruns_platform::connector::ConnectorRegistry,
    pub budget_service: Arc<BudgetService>,
    pub reporting_service: Arc<ReportingService>,
    pub runner: Arc<dyn AgentRunner>,
    pub auth: AuthState,
    pub org_rate_limiter: crate::auth::rate_limit::OrgRateLimiter,
    pub encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
    pub workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    pub fallback_base_harness_name: Option<String>,
    pub fallback_default_harness_name: Option<String>,
    pub chat_harness_name: Option<String>,
    pub chat_session_title: Option<String>,
    pub sqldb_store: Option<Arc<dyn SessionSqlDbStore>>,
    /// System utility LLM for sanctioned internal analysis commands.
    pub utility_llm_service: Arc<dyn everruns_core::UtilityLlmService>,
    /// Agent health check service (knowledge/evaluation/agent-checks.md, tier-3), so the
    /// health-check commands work over MCP, not just HTTP.
    pub health_check_service: Option<Arc<crate::domains::agents::AgentHealthCheckService>>,
    /// Absolute URL of `/.well-known/oauth-protected-resource/mcp`, used to
    /// populate the `WWW-Authenticate: Bearer resource_metadata="..."` header
    /// on 401 responses per RFC 9728 §5.1 and the MCP 2025-06-18 auth spec.
    /// Path-derived per RFC 9728 §3.1 for the `/mcp` resource.
    /// `None` disables the header (e.g. tests without an issuer configured).
    pub resource_metadata_url: Option<String>,
    /// Canonical MCP resource URL (`{root}/mcp`) that MCP OAuth access tokens are
    /// bound to (RFC 8707 audience). The `McpAuthUser` extractor passes this to
    /// `validate_mcp_token` so only tokens minted for this exact resource are
    /// accepted (TM-MCP-006). `None` still rejects regular access tokens via the
    /// token_type check but cannot match an audience.
    pub mcp_resource: Option<String>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn AgentRunner>,
        auth: AuthState,
        host_composition: &HostComposition,
        built_in_harnesses: &[everruns_platform::BuiltInHarnessDefinition],
        notifications_enabled: bool,
        event_delivery: crate::event_delivery::EventDelivery,
        encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
        workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
        capability_service: Arc<CapabilityService>,
        sqldb_store: Option<Arc<dyn SessionSqlDbStore>>,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::with_registry(
                db.clone(),
                host_composition.capability_registry().clone(),
            )),
            message_service: Arc::new(MessageService::new(
                db.clone(),
                runner.clone(),
                notifications_enabled,
                event_delivery.clone(),
            )),
            event_service: Arc::new(EventService::new(db.clone(), event_delivery)),
            session_file_service: Arc::new(WorkspaceFileService::new(db.clone())),
            session_sandbox_service: None,
            capability_service,
            // Hosted connector catalog (EVE-879): defaults to the OSS
            // inventory preset; server composition overrides via
            // `with_connector_registry`.
            connector_registry: crate::platform::oss_connector_registry(),
            budget_service: Arc::new(BudgetService::new(db.clone())),
            reporting_service: Arc::new(ReportingService::new(db.clone())),
            db,
            runner,
            auth,
            org_rate_limiter: crate::auth::rate_limit::OrgRateLimiter::default(),
            encryption,
            workflow_store,
            fallback_base_harness_name: everruns_platform::harness_for_role(
                built_in_harnesses,
                everruns_platform::BuiltInHarnessRole::Base,
            )
            .map(|h| h.name.clone()),
            fallback_default_harness_name: everruns_platform::harness_for_role(
                built_in_harnesses,
                everruns_platform::BuiltInHarnessRole::Default,
            )
            .map(|h| h.name.clone()),
            chat_harness_name: everruns_platform::harness_for_role(
                built_in_harnesses,
                everruns_platform::BuiltInHarnessRole::Chat,
            )
            .map(|h| h.name.clone()),
            chat_session_title: everruns_platform::harness_for_role(
                built_in_harnesses,
                everruns_platform::BuiltInHarnessRole::Chat,
            )
            .map(|h| h.display_name.clone()),
            sqldb_store,
            utility_llm_service: host_composition.utility_llm_service(),
            health_check_service: None,
            resource_metadata_url: None,
            mcp_resource: None,
        }
    }

    pub fn with_resource_metadata_url(mut self, url: impl Into<String>) -> Self {
        self.resource_metadata_url = Some(url.into());
        self
    }

    pub fn with_connector_registry(
        mut self,
        registry: everruns_platform::connector::ConnectorRegistry,
    ) -> Self {
        self.connector_registry = registry;
        self
    }

    pub fn with_org_rate_limiter(
        mut self,
        limiter: crate::auth::rate_limit::OrgRateLimiter,
    ) -> Self {
        self.org_rate_limiter = limiter;
        self
    }

    /// Set the canonical MCP resource URL (`{root}/mcp`) used to validate the
    /// audience of MCP OAuth access tokens (TM-MCP-006).
    pub fn with_mcp_resource(mut self, resource: impl Into<String>) -> Self {
        self.mcp_resource = Some(resource.into());
        self
    }

    pub fn with_virtual_registry(
        mut self,
        registry: Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>,
    ) -> Self {
        self.session_file_service =
            Arc::new(WorkspaceFileService::new(self.db.clone()).with_virtual_registry(registry));
        self
    }

    pub fn with_session_sandbox_service(mut self, service: Arc<SessionSandboxService>) -> Self {
        self.session_sandbox_service = Some(service);
        self
    }

    pub fn with_health_check_service(
        mut self,
        service: Arc<crate::domains::agents::AgentHealthCheckService>,
    ) -> Self {
        self.health_check_service = Some(service);
        self
    }
}

impl_auth_state!(AppState);

// ============================================================================
// MCP-scoped auth extractors
// ============================================================================
//
// THREAT[TM-MCP-006]: the `/mcp` endpoint authenticates on a separate path from
// `/api/*`. `McpAuthUser` accepts only anonymous (no-auth mode), personal access
// tokens, and resource-bound MCP OAuth tokens — never a regular session/access
// JWT or cookie. `McpResolvedOrg` then resolves org context from that user
// without going back through `validate_token`, so the audience split is intact.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

/// Authenticated caller for the `/mcp` endpoint (MCP-scoped validation).
pub struct McpAuthUser(pub AuthUser);

impl FromRequestParts<AppState> for McpAuthUser {
    type Rejection = crate::auth::middleware::AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = crate::auth::middleware::extract_mcp_auth_user(
            parts,
            &state.auth,
            state.mcp_resource.as_deref(),
        )
        .await?;
        Ok(McpAuthUser(user))
    }
}

/// Org context for the `/mcp` endpoint, resolved from the MCP-scoped user.
pub struct McpResolvedOrg(pub ResolvedOrg);

impl FromRequestParts<AppState> for McpResolvedOrg {
    type Rejection = crate::auth::middleware::AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let McpAuthUser(user) = McpAuthUser::from_request_parts(parts, state).await?;
        let is_anonymous = user.auth_method == AuthMethod::None;
        let mut org =
            crate::auth::middleware::resolve_org_for_user(user, parts, &state.auth).await?;
        // THREAT[TM-MCP-006]: no-auth MCP may use the default org for local
        // development, but it is not an authenticated user for user-scoped
        // commands. Keep that boundary separate from the browser/API no-auth
        // principal, whose stable ID is required for Platform Chat ownership.
        if is_anonymous {
            org.user_id = None;
        }
        Ok(McpResolvedOrg(org))
    }
}

// ============================================================================
// Routes
// ============================================================================

pub fn routes(state: AppState) -> Router {
    let metadata_url = state.resource_metadata_url.clone();
    let router = Router::new()
        .route("/mcp", post(handle_mcp))
        .with_state(state);
    match metadata_url
        .as_deref()
        .and_then(parse_www_authenticate_value)
    {
        Some(header_value) => router.layer(axum::middleware::from_fn(
            move |req: Request, next: Next| {
                let header_value = header_value.clone();
                async move { inject_www_authenticate(req, next, header_value).await }
            },
        )),
        None => router,
    }
}

fn build_www_authenticate_value(resource_metadata_url: &str) -> String {
    format!("Bearer realm=\"mcp\", resource_metadata=\"{resource_metadata_url}\"")
}

/// Build the static header value once at router construction time. Returns
/// `None` when the configured URL cannot produce a valid `HeaderValue`
/// (malformed config), which disables the layer rather than paying parse
/// cost per request.
fn parse_www_authenticate_value(resource_metadata_url: &str) -> Option<axum::http::HeaderValue> {
    build_www_authenticate_value(resource_metadata_url)
        .parse()
        .ok()
}

/// Adds `WWW-Authenticate: Bearer resource_metadata="..."` to 401 responses
/// from the MCP endpoint so clients can discover the authorization server
/// via RFC 9728 protected-resource metadata.
async fn inject_www_authenticate(
    req: Request,
    next: Next,
    header_value: axum::http::HeaderValue,
) -> Response {
    let mut response = next.run(req).await;
    if response.status() == StatusCode::UNAUTHORIZED
        && !response.headers().contains_key(header::WWW_AUTHENTICATE)
    {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, header_value);
    }
    response
}

// ============================================================================
// Main handler
// ============================================================================

async fn handle_mcp(
    McpAuthUser(auth_user): McpAuthUser,
    McpResolvedOrg(org): McpResolvedOrg,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    let protocol_version = if req.method == "initialize" {
        None
    } else {
        match protocol_version_from_headers(&headers) {
            Ok(version) => Some(version),
            Err(msg) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(JsonRpcResponse::error(req.id.clone(), -32600, msg)),
                )
                    .into_response();
            }
        }
    };

    // Per JSON-RPC 2.0, a request without an `id` is a notification and the
    // server MUST NOT reply. MCP lifecycle uses this for
    // `notifications/initialized`, `notifications/cancelled`, etc. We're
    // stateless, so there's nothing to do — acknowledge with 202 Accepted and
    // no body. Short-circuit before the jsonrpc version check because the
    // spec gives no addressable id to return an error against.
    if req.id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }

    if req.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::error(
            req.id,
            -32600,
            "Invalid Request: jsonrpc must be \"2.0\"",
        ))
        .into_response();
    }

    // MCP 2026-07-28 routing headers: optional, but must agree with the body.
    if let Err(msg) = validate_routing_headers(&headers, &req) {
        return (
            StatusCode::BAD_REQUEST,
            Json(JsonRpcResponse::error(req.id, -32600, msg)),
        )
            .into_response();
    }

    // MCP 2026-07-28 carries client info in `_meta` on every request instead of
    // a once-per-connection `initialize`. Surface it for telemetry; we store
    // nothing.
    if let Some((name, version)) = client_info_from_params(&req.params) {
        tracing::debug!(
            mcp.client.name = %name,
            mcp.client.version = %version,
            mcp.method = %req.method,
            "MCP request client info"
        );
    }

    // Result metadata (`resultType` and, where the result is cacheable, `ttlMs`
    // / `cacheScope`) is attached here rather than inside each handler: the
    // policy is per-method and per-era, and the handlers have many success
    // paths. See `caching` for the TTL and scope rationale.
    let negotiated_version = protocol_version.unwrap_or(MCP_PROTOCOL_VERSION_FALLBACK);
    let response = match req.method.as_str() {
        "initialize" => handle_initialize(req.id, req.params),
        "tools/list" => {
            let mut response = handle_tools_list(req.id, negotiated_version);
            if let Some(result) = response.result.as_mut() {
                caching::mark_cacheable(
                    result,
                    negotiated_version,
                    caching::TOOLS_LIST_TTL_MS,
                    // The catalog is static per protocol version — identical
                    // for every caller — so it is safe to share across
                    // authorization contexts.
                    caching::CacheScope::Public,
                );
            }
            response
        }
        "tools/call" => {
            let mut response = handle_tools_call(
                req.id.clone(),
                req.params,
                &auth_user,
                &org,
                &state,
                negotiated_version,
            )
            .await;
            // Tool results are never cacheable; they only carry `resultType`.
            if let Some(result) = response.result.as_mut() {
                caching::mark_complete(result, negotiated_version);
            }
            response
        }
        "resources/list" => {
            let mut response = handle_resources_list(req.id);
            if let Some(result) = response.result.as_mut() {
                caching::mark_cacheable(
                    result,
                    negotiated_version,
                    caching::RESOURCES_LIST_TTL_MS,
                    // Static catalog of resource URIs, same for every caller.
                    caching::CacheScope::Public,
                );
            }
            response
        }
        "resources/read" => {
            let mut response = handle_resources_read(req.id, req.params, &org, &state).await;
            if let Some(result) = response.result.as_mut() {
                caching::mark_cacheable(
                    result,
                    negotiated_version,
                    caching::RESOURCES_READ_TTL_MS,
                    // Payloads are org-scoped; a shared cache keyed on the URI
                    // alone would serve one org's data to another.
                    caching::CacheScope::Private,
                );
            }
            response
        }
        // MCP 2026-07-28 Tasks extension (SEP-2663). Task handles map to
        // sessions; these methods delegate to the same session logic the
        // agent_run/session_get_status/session_send_message tools use. Gated to
        // both the negotiated 2026-07-28 protocol and per-request client
        // capability opt-in; clients outside the extension contract get
        // method_not_found, matching "the method does not exist here".
        "tasks/get" | "tasks/cancel" | "tasks/update"
            if protocol_version
                .map(|version| tasks::tasks_enabled(version, &req.params))
                .unwrap_or(false) =>
        {
            handle_tasks_method(
                req.method.as_str(),
                req.id,
                req.params,
                &auth_user,
                &org,
                &state,
            )
            .await
        }
        "ping" => JsonRpcResponse::success(req.id, json!({})),
        _ => JsonRpcResponse::method_not_found(req.id),
    };

    Json(response).into_response()
}

// ============================================================================
// Protocol handlers
// ============================================================================

fn handle_initialize(id: Option<Value>, params: Value) -> JsonRpcResponse {
    let params: InitializeParams = serde_json::from_value(params).unwrap_or_default();
    let protocol_version = negotiate_protocol_version(params.protocol_version.as_deref());
    let mut capabilities = json!({
        "tools": {
            "listChanged": false
        },
        "resources": {}
    });
    // Advertise the Tasks extension (SEP-2663) only under the negotiated
    // 2026-07-28 protocol. 2025-* clients see the capabilities shape unchanged.
    if let Some(extensions) = tasks::initialize_extensions(protocol_version) {
        capabilities["extensions"] = extensions;
    }
    JsonRpcResponse::success(
        id,
        json!({
            "protocolVersion": protocol_version,
            "capabilities": capabilities,
            "serverInfo": {
                "name": MCP_SERVER_NAME,
                "version": MCP_SERVER_VERSION
            }
        }),
    )
}

fn handle_tools_list(id: Option<Value>, protocol_version: &str) -> JsonRpcResponse {
    JsonRpcResponse::success(id, json!({ "tools": tool_definitions(protocol_version) }))
}

// ============================================================================
// Resource handlers (MCP resources capability)
// ============================================================================

/// Static resource catalog — returned by resources/list.
fn handle_resources_list(id: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        json!({
            "resources": [
                {
                    "uri": "everruns://capabilities",
                    "name": "Capabilities",
                    "description": "Available capabilities (tools, sandboxes, integrations)",
                    "mimeType": "application/json"
                },
                {
                    "uri": "everruns://harnesses",
                    "name": "Harnesses",
                    "description": "Available harnesses (base environments for sessions)",
                    "mimeType": "application/json"
                },
                {
                    "uri": "everruns://models",
                    "name": "LLM Models",
                    "description": "Available LLM models and providers",
                    "mimeType": "application/json"
                },
                {
                    "uri": "everruns://agents",
                    "name": "Agents",
                    "description": "Agent summaries (id, name, description)",
                    "mimeType": "application/json"
                }
            ]
        }),
    )
}

/// Read a resource by URI — fetches fresh data on each call.
async fn handle_resources_read(
    id: Option<Value>,
    params: Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let uri = match params.get("uri").and_then(|v| v.as_str()) {
        Some(uri) => uri,
        None => return JsonRpcResponse::invalid_params(id, "Missing 'uri' in params"),
    };

    let result = match uri {
        "everruns://capabilities" => read_capabilities(org, state).await,
        "everruns://harnesses" => read_harnesses(org, state).await,
        "everruns://models" => read_models(org, state).await,
        "everruns://agents" => read_agents(org, state).await,
        _ => return JsonRpcResponse::invalid_params(id, format!("Unknown resource URI: {uri}")),
    };

    match result {
        Ok(text) => JsonRpcResponse::success(
            id,
            json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": text
                }]
            }),
        ),
        // -32603 = internal error (service failure, not client error)
        Err(msg) => JsonRpcResponse::error(id, -32603, &msg),
    }
}

fn mcp_ctx(org: &ResolvedOrg, state: &AppState) -> Ctx {
    let mut ctx = Ctx::new(
        Caller::from(org),
        state.db.clone(),
        state.capability_service.clone(),
        state.encryption.clone(),
        state.auth.permission_resolver.clone(),
    )
    .with_feature_flags(org.feature_flags.clone())
    .with_org_rate_limiter(state.org_rate_limiter.clone())
    .with_utility_llm_service(state.utility_llm_service.clone());
    if let Some(service) = &state.health_check_service {
        ctx = ctx.with_health_check_service(service.clone());
    }
    ctx
}

fn resource_error(resource: &str, e: CommandError) -> String {
    match e.kind {
        CommandErrorKind::Forbidden(msg)
        | CommandErrorKind::BadRequest(msg)
        | CommandErrorKind::NotFound(msg)
        | CommandErrorKind::Conflict(msg)
        | CommandErrorKind::RateLimited(msg)
        | CommandErrorKind::Unprocessable(msg) => msg,
        CommandErrorKind::Internal(err) => format!("Failed to list {resource}: {err}"),
    }
}

async fn read_capabilities(org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let ctx = mcp_ctx(org, state);
    let capabilities = crate::domains::capabilities::ListCapabilities {
        search: None,
        offset: Some(0),
        limit: Some(200),
    }
    .run(&ctx)
    .await
    .map_err(|e| resource_error("capabilities", e))?;

    let summary: Vec<Value> = capabilities
        .data
        .into_iter()
        .map(|c| {
            json!({
                "id": c.id.as_str(),
                "name": c.name,
                "description": c.description,
                "status": c.status,
            })
        })
        .collect();

    let mut value = Value::Array(summary);
    link_builder(state).decorate_value_links(&mut value);
    serde_json::to_string(&value).map_err(|e| format!("Serialization error: {e}"))
}

async fn read_harnesses(org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let ctx = mcp_ctx(org, state);
    let harnesses = crate::domains::harnesses::ListHarnesses {
        search: None,
        include_archived: false,
    }
    .run(&ctx)
    .await
    .map_err(|e| resource_error("harnesses", e))?;

    let summary: Vec<Value> = harnesses
        .into_iter()
        .map(|h| {
            json!({
                "id": h.id.to_string(),
                "name": h.name,
                "description": h.description,
                "status": h.status,
            })
        })
        .collect();

    let mut value = Value::Array(summary);
    link_builder(state).decorate_value_links(&mut value);
    serde_json::to_string(&value).map_err(|e| format!("Serialization error: {e}"))
}

async fn read_models(org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let ctx = mcp_ctx(org, state);
    let providers = crate::domains::providers::ListProviders {}
        .run(&ctx)
        .await
        .map_err(|e| resource_error("providers", e))?;

    let summary: Vec<Value> = providers
        .into_iter()
        .map(|p| {
            json!({
                "id": p.id.to_string(),
                "name": p.name,
                "status": p.status,
            })
        })
        .collect();

    let mut value = Value::Array(summary);
    link_builder(state).decorate_value_links(&mut value);
    serde_json::to_string(&value).map_err(|e| format!("Serialization error: {e}"))
}

async fn read_agents(org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    let ctx = mcp_ctx(org, state);
    let agents = crate::domains::agents::ListAgents {
        search: None,
        include_archived: false,
        offset: Some(0),
        limit: Some(100),
    }
    .run(&ctx)
    .await
    .map_err(|e| resource_error("agents", e))?;

    let summary: Vec<Value> = agents
        .data
        .into_iter()
        .map(|a| {
            json!({
                "id": a.public_id,
                "name": a.name,
                "description": a.description,
            })
        })
        .collect();

    let mut value = Value::Array(summary);
    link_builder(state).decorate_value_links(&mut value);
    serde_json::to_string(&value).map_err(|e| format!("Serialization error: {e}"))
}

async fn handle_tools_call(
    id: Option<Value>,
    params: Value,
    auth_user: &AuthUser,
    org: &ResolvedOrg,
    state: &AppState,
    protocol_version: &str,
) -> JsonRpcResponse {
    // MCP 2026-07-28 Tasks extension: when the client advertised the extension
    // and the negotiated protocol is 2026-07-28, agent_run / session_send_message
    // long-running calls answer with a CreateTaskResult (`resultType: "task"`)
    // task handle alongside their existing fields. `params` (not `arguments`)
    // carries the per-request `_meta` opt-in, so we evaluate it here.
    let tasks_enabled = tasks::tasks_enabled(protocol_version, &params);

    let tool_name = match params.get("name").and_then(|v| v.as_str()) {
        Some(name) => name,
        None => return JsonRpcResponse::invalid_params(id, "Missing 'name' in params"),
    };

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));

    let Some(tool_def) = find_tool_definition(tool_name, protocol_version) else {
        let msg = format!("Unknown tool: {tool_name}");
        let envelope = McpExecuteError::new(McpErrorCode::ToolNotFound, &msg).with_hint(
            "Call tools/list to discover the current tool catalog for this protocol version.",
        );
        return JsonRpcResponse::success(id, error_result_payload(&msg, Some(&envelope)));
    };

    // Card tools return an MCP content array directly (resource + summary
    // text) and skip the JSON-string wrapping path used by other tools.
    // See knowledge/ui/mcp-cards.md.
    if tool_name == "agent_get_card" {
        let card_result = tokio::time::timeout(
            std::time::Duration::from_millis(tool_def.timeout_ms()),
            async {
                match resolve_org_override(&arguments, auth_user, org, state).await {
                    Ok(org) => tool_agent_get_card(&arguments, &org, state).await,
                    Err(e) => Err(e),
                }
            },
        )
        .await
        .unwrap_or_else(|_| Err(format!("Tool timed out after {}ms", tool_def.timeout_ms())));

        return match card_result {
            Ok(content_array) => JsonRpcResponse::success(id, json!({ "content": content_array })),
            Err(msg) => {
                let envelope = classify_mcp_execute_error(&msg);
                JsonRpcResponse::success(id, error_result_payload(&msg, Some(&envelope)))
            }
        };
    }

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(tool_def.timeout_ms()),
        async {
            match tool_name {
                // Tier 0: identity & org context
                "me" => tool_me(auth_user, org, state).await,
                "list_organizations" => tool_list_organizations(auth_user, state).await,
                // Tier 1: agent conversation (org-scoped — accept organization_id override)
                "agent_run" => {
                    match resolve_org_override(&arguments, auth_user, org, state).await {
                        Ok(org) => tool_agent_run(&arguments, &org, state).await,
                        Err(e) => Err(e),
                    }
                }
                "session_send_message" => {
                    match resolve_org_override(&arguments, auth_user, org, state).await {
                        Ok(org) => tool_session_send_message(&arguments, &org, state).await,
                        Err(e) => Err(e),
                    }
                }
                "session_get_status" => {
                    match resolve_org_override(&arguments, auth_user, org, state).await {
                        Ok(org) => tool_session_get_status(&arguments, &org, state).await,
                        Err(e) => Err(e),
                    }
                }
                // Tier 2: catalog & scripting (org-scoped)
                "discover" => match resolve_org_override(&arguments, auth_user, org, state).await {
                    Ok(org) => tool_discover(&arguments, &org, state).await,
                    Err(e) => Err(e),
                },
                "query" => match resolve_org_override(&arguments, auth_user, org, state).await {
                    Ok(org) => tool_query(&arguments, &org, state).await,
                    Err(e) => Err(e),
                },
                "execute" => match resolve_org_override(&arguments, auth_user, org, state).await {
                    Ok(org) => tool_execute(&arguments, &org, state).await,
                    Err(e) => Err(e),
                },
                _ => Err(format!("Unknown tool: {tool_name}")),
            }
        },
    )
    .await
    .unwrap_or_else(|_| Err(format!("Tool timed out after {}ms", tool_def.timeout_ms())));

    match result {
        Ok(content) => {
            let structured_content =
                if supports_rich_tool_shape(protocol_version) && tool_def.has_output_schema() {
                    serde_json::from_str::<Value>(&content).ok()
                } else {
                    None
                };

            let mut result = json!({
                "content": [{ "type": "text", "text": content }]
            });
            if let Some(structured_content) = structured_content {
                result["structuredContent"] = structured_content;
            }

            // Tasks extension: for the long-running conversation tools, add the
            // CreateTaskResult task-handle fields (taskId = session_id) so a 2026
            // client that opted in can treat the call as a standard Task. The
            // legacy `content`/`structuredContent` fields stay untouched, so this
            // is strictly additive.
            if tasks_enabled && matches!(tool_name, "agent_run" | "session_send_message") {
                augment_with_task_handle(&mut result, &content);
            }

            JsonRpcResponse::success(id, result)
        }
        Err(msg) => {
            let envelope = classify_mcp_execute_error(&msg);
            JsonRpcResponse::success(id, error_result_payload(&msg, Some(&envelope)))
        }
    }
}

/// Merge Tasks-extension `CreateTaskResult` fields (`resultType`, `taskId`,
/// `status`, `ttlMs`, `pollIntervalMs`) into a successful tools/call result,
/// deriving them from the tool's JSON `content`. `agent_run` exposes the session
/// as `session_id`/`status`; `session_send_message` as `session_id`/
/// `session_status`. The task handle is `session_id`. If the content isn't
/// parseable JSON with a session id (it always is for these tools), we leave the
/// result unchanged rather than fabricate a handle.
fn augment_with_task_handle(result: &mut Value, content: &str) {
    let Ok(parsed) = serde_json::from_str::<Value>(content) else {
        return;
    };
    let Some(session_id) = parsed.get("session_id").and_then(Value::as_str) else {
        return;
    };
    let session_status = parsed
        .get("status")
        .or_else(|| parsed.get("session_status"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let task = tasks::create_task_result(session_id, session_status);
    if let (Some(result_obj), Some(task_obj)) = (result.as_object_mut(), task.as_object()) {
        for (key, value) in task_obj {
            result_obj.insert(key.clone(), value.clone());
        }
    }
}

// ============================================================================
// MCP 2026-07-28 Tasks extension methods (SEP-2663)
// ============================================================================
//
// A task handle is a `session_id`; each method delegates to the session logic
// the equivalent tool already uses. See `tasks.rs` for the mapping rationale.

async fn handle_tasks_method(
    method: &str,
    id: Option<Value>,
    params: Value,
    auth_user: &AuthUser,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let Some(task_id) = params.get("taskId").and_then(Value::as_str) else {
        return JsonRpcResponse::invalid_params(id, "Missing 'taskId' in params");
    };

    // A taskId is a session_id; org override rides the same `_meta`-free path as
    // the tools by reading `organization_id` from params when present.
    let org = match resolve_org_override(&params, auth_user, org, state).await {
        Ok(org) => org,
        Err(e) => return JsonRpcResponse::invalid_params(id, e),
    };

    match method {
        "tasks/get" => handle_tasks_get(id, task_id, &params, &org, state).await,
        "tasks/cancel" => handle_tasks_cancel(id, task_id, &org, state).await,
        "tasks/update" => handle_tasks_update(id, task_id, &params, &org, state).await,
        _ => JsonRpcResponse::method_not_found(id),
    }
}

/// `tasks/get` → session status + events. Reuses `tool_session_get_status` and
/// wraps its JSON into the Tasks `Task` object. On `completed`, the underlying
/// tool's structured JSON is surfaced as `result`.
async fn handle_tasks_get(
    id: Option<Value>,
    task_id: &str,
    params: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    // Reshape into the args `tool_session_get_status` expects (session_id +
    // optional since_event_id/event_types passthrough).
    let mut args = json!({ "session_id": task_id });
    if let Some(since) = params.get("since_event_id") {
        args["since_event_id"] = since.clone();
    }
    if let Some(types) = params.get("event_types") {
        args["event_types"] = types.clone();
    }

    match tool_session_get_status(&args, org, state).await {
        Ok(status_json) => {
            let mut status_value: Value = serde_json::from_str(&status_json).unwrap_or(Value::Null);
            let session_status = status_value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("");
            let task_status = tasks::task_status_from_session_status(session_status);

            // EVE-728: when the task reported a deterministic structured result
            // (result.json via a `result_schema`, EVE-678), surface it under
            // `result.structured_result` so Tasks clients get the machine result
            // instead of only last-message / status text. The read is org-scoped
            // (same org `tool_session_get_status` already validated), preserving
            // tenant isolation. Parse failures / absent results are silently
            // skipped — the status payload is still returned.
            if let Ok(session_id) = task_id.parse::<everruns_core::typed_id::SessionId>()
                && let Ok(Some(structured)) =
                    crate::domains::session_tasks::read_structured_task_result(
                        &state.db, org.org_id, session_id,
                    )
                    .await
                && let Some(obj) = status_value.as_object_mut()
            {
                obj.insert("structured_result".to_string(), structured);
            }

            let mut task = tasks::task_handle(task_id, task_status);
            // The full session_get_status payload is the task's result view. For
            // terminal `completed`, this is what the original tools/call would
            // have returned; for `working`/`input_required` it's a progress
            // snapshot the client can inspect.
            task["result"] = status_value;
            JsonRpcResponse::success(id, task)
        }
        Err(msg) => {
            let envelope = classify_mcp_execute_error(&msg);
            JsonRpcResponse::success(id, error_result_payload(&msg, Some(&envelope)))
        }
    }
}

/// `tasks/cancel` → cancel the running turn. Cooperative per SEP-2663: we
/// acknowledge intent; the session may still reach a non-`cancelled` terminal.
async fn handle_tasks_cancel(
    id: Option<Value>,
    task_id: &str,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    match dispatch_command(
        "cancel_session",
        json!({ "session_id": task_id }),
        org,
        state,
    )
    .await
    {
        Ok(_) => {
            // Report the post-cancel session status as the task status rather
            // than asserting `cancelled`: cancellation returns the session to
            // idle, and SEP-2663 explicitly allows a non-`cancelled` terminal.
            let task_status =
                match dispatch_command("get_session", json!({ "session_id": task_id }), org, state)
                    .await
                {
                    Ok(session) => tasks::task_status_from_session_status(
                        session.get("status").and_then(Value::as_str).unwrap_or(""),
                    ),
                    Err(_) => tasks::TaskStatus::Cancelled,
                };
            JsonRpcResponse::success(id, tasks::task_handle(task_id, task_status))
        }
        Err(msg) => {
            let envelope = classify_mcp_execute_error(&msg);
            JsonRpcResponse::success(id, error_result_payload(&msg, Some(&envelope)))
        }
    }
}

/// `tasks/update` → provide input to a task in `input_required`. Maps to sending
/// a user message (`session_send_message`). SEP-2663 keys input under
/// `inputResponses`; we accept either a single `message` string or the first
/// string value found in the `inputResponses` map.
async fn handle_tasks_update(
    id: Option<Value>,
    task_id: &str,
    params: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> JsonRpcResponse {
    let message = params.get("message").and_then(Value::as_str).or_else(|| {
        params
            .get("inputResponses")
            .and_then(Value::as_object)
            .and_then(|m| m.values().find_map(Value::as_str))
    });

    let Some(message) = message else {
        return JsonRpcResponse::invalid_params(
            id,
            "tasks/update requires a 'message' string or an 'inputResponses' map with a string value",
        );
    };

    let args = json!({ "session_id": task_id, "message": message });
    match tool_session_send_message(&args, org, state).await {
        Ok(send_json) => {
            let parsed: Value = serde_json::from_str(&send_json).unwrap_or(Value::Null);
            let session_status = parsed
                .get("session_status")
                .and_then(Value::as_str)
                .unwrap_or("");
            let task_status = tasks::task_status_from_session_status(session_status);
            let mut task = tasks::task_handle(task_id, task_status);
            task["result"] = parsed;
            JsonRpcResponse::success(id, task)
        }
        Err(msg) => {
            let envelope = classify_mcp_execute_error(&msg);
            JsonRpcResponse::success(id, error_result_payload(&msg, Some(&envelope)))
        }
    }
}

/// Build the `result` payload for a tools/call error response. Always
/// emits the legacy `content[0].text` + `isError: true` so MCP clients
/// that predate the structured envelope keep working; when an envelope
/// is supplied, also emits `structuredContent` carrying the typed
/// [`McpExecuteError`] so newer SDKs can branch on a machine-readable
/// `code`/`category`/`retryable` triple. See `knowledge/integrations/mcp.md`.
fn error_result_payload(message: &str, envelope: Option<&McpExecuteError>) -> Value {
    let mut payload = json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    });
    if let Some(envelope) = envelope
        && let Ok(structured) = serde_json::to_value(envelope)
    {
        payload["structuredContent"] = structured;
    }
    payload
}

// ============================================================================
// Org override helper
// ============================================================================

/// If the tool arguments contain `organization_id`, validate the user's membership
/// and return a `ResolvedOrg` targeting that org. Otherwise return the default.
///
/// This is the core mechanism for multi-org MCP support: since MCP clients
/// can't set cookies, they pass `organization_id` per-tool-call instead.
async fn resolve_org_override(
    args: &Value,
    auth_user: &AuthUser,
    default_org: &ResolvedOrg,
    state: &AppState,
) -> Result<ResolvedOrg, String> {
    // THREAT[TM-MCP-001]: External MCP clients can pass organization_id on
    // server tools; resolve it through fresh membership before dispatch.
    let org_public_id = match args.get("organization_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return Ok(default_org.clone()),
    };

    enforce_org_override_auth_scope(org_public_id, auth_user, default_org)?;
    if org_public_id == default_org.public_id {
        return Ok(default_org.clone());
    }

    resolve_org_by_id(org_public_id, auth_user, state).await
}

fn enforce_org_override_auth_scope(
    org_public_id: &str,
    auth_user: &AuthUser,
    default_org: &ResolvedOrg,
) -> Result<(), String> {
    if auth_user.auth_method == AuthMethod::PersonalAccessToken
        && org_public_id != default_org.public_id
    {
        return Err(
            "organization_id override is not allowed for personal access token authentication"
                .into(),
        );
    }

    Ok(())
}

/// Resolve and validate an organization by public ID for the given user.
async fn resolve_org_by_id(
    org_public_id: &str,
    auth_user: &AuthUser,
    state: &AppState,
) -> Result<ResolvedOrg, String> {
    if !validate_org_public_id(org_public_id) {
        return Err(format!("Invalid organization_id format: {org_public_id}"));
    }

    // Query DB for fresh membership (JWT orgs may be stale)
    let user_orgs = state
        .db
        .list_user_organizations(auth_user.id)
        .await
        .map_err(|e| format!("Failed to list user organizations: {e}"))?;

    let org_row = user_orgs
        .iter()
        .find(|o| o.public_id == org_public_id)
        .ok_or_else(|| {
            format!("Organization not found or you are not a member: {org_public_id}")
        })?;

    let role = org_row.role.parse::<OrgRole>().unwrap_or(OrgRole::Member);

    let feature_flags = crate::services::org_feature_flags::resolve_org_feature_flags(
        &state.db,
        org_row.org_id,
        &state.auth.system_feature_flags,
    )
    .await
    .unwrap_or_else(|_| {
        everruns_platform::FeatureFlags::for_org(
            &state.auth.system_feature_flags,
            &std::collections::HashMap::new(),
        )
    });

    Ok(ResolvedOrg {
        org_id: org_row.org_id,
        public_id: org_row.public_id.clone(),
        name: org_row.name.clone(),
        user_id: Some(auth_user.id),
        role,
        is_platform_user: auth_user.is_platform_user,
        feature_flags,
    })
}

// ============================================================================
// Tier 0: me
// ============================================================================

async fn tool_me(
    auth_user: &AuthUser,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<String, String> {
    // Get fresh org memberships from DB
    let user_orgs = state
        .db
        .list_user_organizations(auth_user.id)
        .await
        .map_err(|e| format!("Failed to list user organizations: {e}"))?;

    let orgs: Vec<Value> = user_orgs
        .iter()
        .map(|o| {
            let is_current = o.public_id == org.public_id;
            json!({
                "id": o.public_id,
                "name": o.name,
                "role": o.role,
                "current": is_current,
            })
        })
        .collect();

    Ok(serde_json::to_string_pretty(&json!({
        "user": {
            "id": auth_user.id.to_string(),
            "email": auth_user.email,
            "name": auth_user.name,
        },
        "current_organization": {
            "id": org.public_id,
            "name": org.name,
            "role": org.role.to_string(),
        },
        "organizations": orgs,
    }))
    .unwrap())
}

// ============================================================================
// Tier 0: list_organizations
// ============================================================================

async fn tool_list_organizations(auth_user: &AuthUser, state: &AppState) -> Result<String, String> {
    let user_orgs = state
        .db
        .list_user_organizations(auth_user.id)
        .await
        .map_err(|e| format!("Failed to list user organizations: {e}"))?;

    let orgs: Vec<Value> = user_orgs
        .iter()
        .map(|o| {
            json!({
                "id": o.public_id,
                "name": o.name,
                "role": o.role,
            })
        })
        .collect();

    Ok(serde_json::to_string_pretty(&json!({
        "organizations": orgs,
        "count": orgs.len(),
    }))
    .unwrap())
}

async fn dispatch_command(
    name: &str,
    params: Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Value, String> {
    let ctx = catalog_context(org, state).to_domain_ctx();
    let result = crate::domains::common::dispatch(name, params, &ctx)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&result).map_err(|e| format!("Internal error: {e}"))
}

fn pretty_json(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| format!("Internal error: {e}"))
}

fn link_builder(state: &AppState) -> crate::api::common::UrlBuilder {
    crate::api::common::UrlBuilder::from_auth_config(&state.auth.config)
}

// ============================================================================
// Tier 1: agent_run
// ============================================================================

async fn tool_agent_run(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<String, String> {
    let message_text = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: message")?;

    let session_params = json!({
        "agent_id": args.get("agent_id"),
        "harness_id": args.get("harness_id"),
        "title": args.get("title"),
        "model_id": args.get("model_id"),
    });
    let session = dispatch_command("create_session", session_params, org, state).await?;
    let session_id = session["id"]
        .as_str()
        .ok_or("Internal error: no session ID in response")?;

    // Create budget if budget_limit is specified
    let budget_id = if let Some(budget_limit) = args.get("budget_limit").and_then(|v| v.as_f64()) {
        if budget_limit <= 0.0 {
            return Err("budget_limit must be positive".to_string());
        }
        let budget_soft_limit = args.get("budget_soft_limit").and_then(|v| v.as_f64());
        if budget_soft_limit.is_some_and(|s| s <= 0.0 || s > budget_limit) {
            return Err(
                "budget_soft_limit must be greater than 0 and at most budget_limit".to_string(),
            );
        }
        let budget_params = json!({
            "subject_type": "session",
            "subject_id": session_id,
            "currency": args.get("budget_currency").and_then(|v| v.as_str()).unwrap_or("usd"),
            "limit": budget_limit,
            "soft_limit": budget_soft_limit,
        });
        let budget = dispatch_command("create_budget", budget_params, org, state)
            .await
            .map_err(|e| format!("Session created but budget creation failed: {e}"))?;
        budget["id"].as_str().map(ToString::to_string)
    } else {
        None
    };

    // Send first message via catalog handler
    let msg_params = json!({
        "session_id": session_id,
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": message_text}]
        }
    });
    let msg = dispatch_command("create_message", msg_params, org, state).await?;

    let mut result = json!({
        "session_id": session_id,
        "message_id": msg["id"],
        "status": session["status"],
        "hint": "Use session_get_status to poll for the agent's response, or connect to SSE at /api/v1/sessions/{session_id}/sse"
    });
    if let Some(bid) = budget_id {
        result["budget_id"] = json!(bid);
    }
    link_builder(state).decorate_value_links(&mut result);

    pretty_json(&result)
}

// ============================================================================
// Tier 1: session_send_message
// ============================================================================

async fn tool_session_send_message(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<String, String> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: session_id")?;

    let message_text = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: message")?;

    let msg_params = json!({
        "session_id": session_id,
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": message_text}]
        }
    });
    let msg = dispatch_command("create_message", msg_params, org, state).await?;

    // Fetch session to get current status (create_message returns message, not session)
    let session_params = json!({ "session_id": session_id });
    let session = dispatch_command("get_session", session_params, org, state).await?;

    let mut result = json!({
        "message_id": msg["id"],
        "session_status": session["status"],
        "session_id": session_id,
        "hint": "Use session_get_status to poll for completion"
    });
    link_builder(state).decorate_value_links(&mut result);
    pretty_json(&result)
}

// ============================================================================
// Tier 1: session_get_status
// ============================================================================

async fn tool_session_get_status(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<String, String> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: session_id")?;

    let session_params = json!({ "session_id": session_id });
    let session = dispatch_command("get_session", session_params, org, state).await?;

    // Get recent events via handler
    let event_types: Vec<String> = args
        .get("event_types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let since_event_id = args
        .get("since_event_id")
        .and_then(|v| v.as_str())
        .map(|s| {
            s.parse::<everruns_core::typed_id::EventId>()
                .map(|id| id.to_string())
                .or_else(|_| {
                    s.parse::<uuid::Uuid>()
                        .map(|u| everruns_core::typed_id::EventId::from_uuid(u).to_string())
                })
                .map_err(|_| format!("Invalid since_event_id: {s}"))
        })
        .transpose()?;

    let event_params = json!({
        "session_id": session_id,
        "since_id": since_event_id,
        "types": event_types,
        "limit": 50,
    });
    let events_data = dispatch_command("list_events", event_params, org, state).await?;
    let events = events_data["data"].as_array();

    // Extract latest output from events
    let empty = vec![];
    let events_arr = events.unwrap_or(&empty);
    let latest_output = events_arr
        .iter()
        .rev()
        .find(|e| e.get("event_type").and_then(|v| v.as_str()) == Some("output.message.completed"))
        .and_then(|e| e.get("data"))
        .and_then(|d| d.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
        .and_then(|parts| {
            parts
                .iter()
                .find(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                .and_then(|p| p.get("text").and_then(|t| t.as_str()))
        });

    let last_event_id = events_arr
        .last()
        .and_then(|e| e.get("id").and_then(|v| v.as_str()));

    let mut result = json!({
        "session_id": session_id,
        "status": session["status"],
        "agent_id": session["agent_id"],
        "title": session["title"],
        "latest_output": latest_output,
        "last_event_id": last_event_id,
        "event_count": events_arr.len(),
        "events": events_arr.iter().map(|e| json!({
            "id": e["id"],
            "type": e["event_type"],
            "ts": e["ts"],
        })).collect::<Vec<_>>()
    });
    link_builder(state).decorate_value_links(&mut result);
    pretty_json(&result)
}

// ============================================================================
// Tier 1: agent_get_card — MCP-Apps card resource for an agent
// See knowledge/ui/mcp-cards.md for the card standard.
// ============================================================================

async fn tool_agent_get_card(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Value, String> {
    let agent_ref = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: agent_id")?;

    let ctx = mcp_ctx(org, state);
    let agent = crate::domains::agents::GetAgent {
        id: agent_ref.to_string(),
    }
    .run(&ctx)
    .await
    .map_err(|e| resource_error("agent", e))?;

    // Cumulative session count for this agent in the resolved org. We
    // intentionally hit the storage layer directly here instead of going
    // through a list-and-count: callers asking for a card don't need the
    // list payload, and `count_sessions_for_agent` is a single COUNT query.
    let agent_id = everruns_core::AgentId::from_uuid(agent.internal_id);
    let session_count = state
        .db
        .count_sessions_for_agent(org.org_id, agent_id)
        .await
        .map_err(|e| format!("Failed to count sessions: {e}"))?;

    let stats = cards::AgentCardStats { session_count };
    let card = cards::agent_card(&agent, stats);
    let summary = cards::agent_card_summary(&agent, stats);
    cards::card_tool_content(&card, &summary)
}

// ============================================================================
// Tier 2: discover — structured catalog from inventory descriptors
// ============================================================================

async fn tool_discover(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<String, String> {
    crate::services::platform_command_surface::invoke(
        crate::services::platform_command_surface::Operation::Discover,
        args,
        catalog_context(org, state),
    )
    .await
}

// ============================================================================
// Tier 2: query/execute — delegate to ScriptedTool
// ============================================================================

async fn tool_query(args: &Value, org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    crate::services::platform_command_surface::invoke(
        crate::services::platform_command_surface::Operation::Query,
        args,
        catalog_context(org, state),
    )
    .await
}

async fn tool_execute(args: &Value, org: &ResolvedOrg, state: &AppState) -> Result<String, String> {
    crate::services::platform_command_surface::invoke(
        crate::services::platform_command_surface::Operation::Execute,
        args,
        catalog_context(org, state),
    )
    .await
}

// ============================================================================
// Catalog command-context helpers
// ============================================================================

/// Build a CatalogContext for the given org.
fn catalog_context(org: &ResolvedOrg, state: &AppState) -> catalog::CatalogContext {
    catalog::CatalogContext {
        domain_ctx: domain_context(Caller::from(org), state)
            .with_feature_flags(org.feature_flags.clone()),
        link_builder: link_builder(state),
    }
}

pub(crate) fn domain_context(caller: Caller, state: &AppState) -> crate::domains::common::Ctx {
    let mut ctx = crate::domains::common::Ctx::new(
        caller,
        state.db.clone(),
        state.capability_service.clone(),
        state.encryption.clone(),
        state.auth.permission_resolver.clone(),
    )
    .with_connector_registry(state.connector_registry.clone())
    .with_org_rate_limiter(state.org_rate_limiter.clone())
    .with_session_service(state.session_service.clone())
    .with_message_service(state.message_service.clone())
    .with_event_service(state.event_service.clone())
    .with_reporting_service(state.reporting_service.clone())
    .with_session_file_service(state.session_file_service.clone())
    .with_runner(state.runner.clone())
    .with_fallback_harness_name(state.fallback_default_harness_name.clone())
    .with_chat_harness_name(state.chat_harness_name.clone())
    .with_chat_session_title(state.chat_session_title.clone())
    .with_utility_llm_service(state.utility_llm_service.clone());
    if let Some(service) = &state.health_check_service {
        ctx = ctx.with_health_check_service(service.clone());
    }
    if let Some(service) = &state.session_sandbox_service {
        ctx = ctx.with_session_sandbox_service(service.clone());
    }
    if let Some(store) = &state.sqldb_store {
        ctx = ctx.with_sqldb_store(store.clone());
    }
    ctx.with_workflow_store(state.workflow_store.clone())
}

#[cfg(test)]
mod org_override_scope_tests {
    use super::*;
    use everruns_platform::OrgMembership;
    use uuid::Uuid;

    fn test_auth_user(auth_method: AuthMethod) -> AuthUser {
        AuthUser {
            id: Uuid::new_v4(),
            email: "user@example.com".to_string(),
            name: "Test User".to_string(),
            roles: vec!["member".to_string()],
            is_platform_user: false,
            auth_method,
            organizations: vec![OrgMembership {
                org_id: 1,
                public_id: "org_default_12345678".to_string(),
                name: "Default".to_string(),
                role: OrgRole::Member,
            }],
        }
    }

    fn default_org() -> ResolvedOrg {
        ResolvedOrg {
            org_id: 1,
            public_id: "org_default_12345678".to_string(),
            name: "Default".to_string(),
            user_id: Some(Uuid::new_v4()),
            role: OrgRole::Member,
            is_platform_user: false,
            feature_flags: everruns_platform::FeatureFlags::default(),
        }
    }

    #[test]
    fn api_key_auth_rejects_cross_org_override() {
        let auth_user = test_auth_user(AuthMethod::PersonalAccessToken);
        let result =
            enforce_org_override_auth_scope("org_other_12345678", &auth_user, &default_org());

        assert!(result.is_err());
    }

    #[test]
    fn api_key_auth_allows_default_org_override() {
        let auth_user = test_auth_user(AuthMethod::PersonalAccessToken);
        let org = default_org();
        let result = enforce_org_override_auth_scope(&org.public_id, &auth_user, &org);

        assert!(result.is_ok());
    }

    #[test]
    fn jwt_auth_allows_cross_org_override() {
        let auth_user = test_auth_user(AuthMethod::Jwt);
        let result =
            enforce_org_override_auth_scope("org_other_12345678", &auth_user, &default_org());

        assert!(result.is_ok());
    }
}

#[cfg(test)]
mod www_authenticate_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::response::IntoResponse;
    use axum::routing::any;
    use tower::ServiceExt;

    const METADATA_URL: &str = "https://example.com/.well-known/oauth-protected-resource/mcp";

    fn app_with_layer(status: StatusCode) -> Router {
        let header_value = parse_www_authenticate_value(METADATA_URL).expect("valid header");
        Router::new()
            .route("/mcp", any(move || async move { status.into_response() }))
            .layer(axum::middleware::from_fn(
                move |req: Request, next: Next| {
                    let header_value = header_value.clone();
                    async move { inject_www_authenticate(req, next, header_value).await }
                },
            ))
    }

    #[test]
    fn header_value_matches_rfc9728_format() {
        let value = build_www_authenticate_value(METADATA_URL);
        assert_eq!(
            value,
            format!("Bearer realm=\"mcp\", resource_metadata=\"{METADATA_URL}\"")
        );
    }

    #[tokio::test]
    async fn adds_header_on_401() {
        let app = app_with_layer(StatusCode::UNAUTHORIZED);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/mcp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let header = response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .expect("WWW-Authenticate header present on 401")
            .to_str()
            .unwrap();
        assert!(header.starts_with("Bearer realm=\"mcp\""));
        assert!(header.contains(&format!("resource_metadata=\"{METADATA_URL}\"")));
    }

    #[tokio::test]
    async fn skips_header_on_non_401() {
        let app = app_with_layer(StatusCode::OK);
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/mcp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::WWW_AUTHENTICATE).is_none());
    }

    #[tokio::test]
    async fn preserves_existing_header() {
        let header_value = parse_www_authenticate_value(METADATA_URL).expect("valid header");
        let app = Router::new()
            .route(
                "/mcp",
                any(|| async {
                    let mut response = StatusCode::UNAUTHORIZED.into_response();
                    response.headers_mut().insert(
                        header::WWW_AUTHENTICATE,
                        "Basic realm=\"x\"".parse().unwrap(),
                    );
                    response
                }),
            )
            .layer(axum::middleware::from_fn(
                move |req: Request, next: Next| {
                    let header_value = header_value.clone();
                    async move { inject_www_authenticate(req, next, header_value).await }
                },
            ));
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/mcp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response
                .headers()
                .get(header::WWW_AUTHENTICATE)
                .unwrap()
                .to_str()
                .unwrap(),
            "Basic realm=\"x\""
        );
    }
}

// Regression tests for fix(mcp): enforce policies for resources/read metadata (#1516).
//
// Prior to the fix, `resources/read` enumerated harnesses/agents/providers via
// direct storage calls that bypassed policy checks. The fix routes each read
// through a domain `List*` command via `Command::run`, which evaluates the
// command's policy against `Ctx.permission_resolver`. These tests lock in:
//   1. `resource_error` maps every `CommandError` variant to the expected
//      user-facing text without leaking internal shapes.
//   2. The policy-gated list commands the mcp endpoint now dispatches to
//      reject a caller whose resolver denies the required view permission,
//      proving the policy gate is consulted and the bypass is closed.
#[cfg(test)]
mod resources_read_policy_tests {
    use super::*;
    use crate::domains::common::{Command, CommandError};
    use crate::services::CapabilityService;
    use crate::storage::StorageBackend;
    use everruns_core::{Caller, OrgRole, Permission, PermissionResolver};
    use std::sync::Arc;

    /// Resolver that refuses every permission.
    struct DenyAllResolver;

    impl PermissionResolver for DenyAllResolver {
        fn has_permission(&self, _caller: &Caller, _permission: &Permission) -> bool {
            false
        }

        fn caller_permissions(&self, _caller: &Caller) -> Vec<Permission> {
            Vec::new()
        }
    }

    fn test_caller() -> Caller {
        Caller {
            org_id: 1,
            org_public_id: "org_test".to_string(),
            user_id: None,
            // Member-level, so role rules don't accidentally pass in isolation.
            role: OrgRole::Member,
            is_platform_user: false,
            is_internal: false,
        }
    }

    fn deny_all_ctx() -> crate::domains::common::Ctx {
        let db = Arc::new(StorageBackend::in_memory());
        let capabilities = Arc::new(CapabilityService::new(db.clone(), None));
        crate::domains::common::Ctx::new(
            test_caller(),
            db,
            capabilities,
            None,
            Arc::new(DenyAllResolver),
        )
    }

    // ----- `resource_error` mapping -----

    #[test]
    fn resource_error_forbidden_returns_policy_message() {
        let msg = resource_error(
            "harnesses",
            CommandError::forbidden("access denied: harness.view"),
        );
        assert_eq!(msg, "access denied: harness.view");
    }

    #[test]
    fn resource_error_not_found_returns_message() {
        let msg = resource_error("agents", CommandError::not_found_msg("agent missing"));
        assert_eq!(msg, "agent missing");
    }

    #[test]
    fn resource_error_bad_request_returns_message() {
        let msg = resource_error("harnesses", CommandError::bad_request("bad param"));
        assert_eq!(msg, "bad param");
    }

    #[test]
    fn resource_error_conflict_returns_message() {
        let msg = resource_error("providers", CommandError::conflict("dup"));
        assert_eq!(msg, "dup");
    }

    #[test]
    fn resource_error_unprocessable_returns_message() {
        let msg = resource_error("capabilities", CommandError::unprocessable("unprocessable"));
        assert_eq!(msg, "unprocessable");
    }

    #[test]
    fn resource_error_internal_prefixes_resource_name() {
        let msg = resource_error(
            "providers",
            CommandError::internal(anyhow::anyhow!("connection refused")),
        );
        assert_eq!(msg, "Failed to list providers: connection refused");
    }

    // ----- Policy enforcement on list commands -----

    #[tokio::test]
    async fn list_harnesses_blocked_when_resolver_denies() {
        let ctx = deny_all_ctx();
        let result = crate::domains::harnesses::ListHarnesses {
            search: None,
            include_archived: false,
        }
        .run(&ctx)
        .await;

        let err = result.expect_err("denying resolver must block list_harnesses");
        assert!(
            matches!(err, CommandError { kind: CommandErrorKind::Forbidden(ref msg), .. } if msg.contains("harness.view")),
            "expected Forbidden(harness.view), got {err:?}"
        );
    }

    #[tokio::test]
    async fn list_providers_blocked_when_resolver_denies() {
        let ctx = deny_all_ctx();
        let result = crate::domains::providers::ListProviders {}.run(&ctx).await;

        let err = result.expect_err("denying resolver must block list_providers");
        assert!(
            matches!(err, CommandError { kind: CommandErrorKind::Forbidden(ref msg), .. } if msg.contains("provider.view")),
            "expected Forbidden(provider.view), got {err:?}"
        );
    }

    #[tokio::test]
    async fn list_agents_blocked_when_resolver_denies() {
        let ctx = deny_all_ctx();
        let result = crate::domains::agents::ListAgents {
            search: None,
            include_archived: false,
            offset: Some(0),
            limit: Some(100),
        }
        .run(&ctx)
        .await;

        let err = result.expect_err("denying resolver must block list_agents");
        assert!(
            matches!(err, CommandError { kind: CommandErrorKind::Forbidden(ref msg), .. } if msg.contains("agent.view")),
            "expected Forbidden(agent.view), got {err:?}"
        );
    }
}

#[cfg(test)]
mod protocol_version_tests {
    use super::*;

    #[test]
    fn negotiate_echoes_each_supported_version() {
        assert_eq!(
            negotiate_protocol_version(Some(MCP_PROTOCOL_VERSION_LATEST)),
            MCP_PROTOCOL_VERSION_LATEST
        );
        assert_eq!(
            negotiate_protocol_version(Some(MCP_PROTOCOL_VERSION_2025_06)),
            MCP_PROTOCOL_VERSION_2025_06
        );
        assert_eq!(
            negotiate_protocol_version(Some(MCP_PROTOCOL_VERSION_FALLBACK)),
            MCP_PROTOCOL_VERSION_FALLBACK
        );
    }

    #[test]
    fn negotiate_missing_version_falls_back() {
        assert_eq!(
            negotiate_protocol_version(None),
            MCP_PROTOCOL_VERSION_FALLBACK
        );
    }

    #[test]
    fn negotiate_future_version_offers_latest() {
        // A client newer than us gets our latest and decides what to do.
        assert_eq!(
            negotiate_protocol_version(Some("2099-01-01")),
            MCP_PROTOCOL_VERSION_LATEST
        );
    }

    #[test]
    fn negotiate_in_between_version_picks_highest_supported_below() {
        // Between 2025-06-18 and 2026-07-28 → 2025-06-18.
        assert_eq!(
            negotiate_protocol_version(Some("2025-12-01")),
            MCP_PROTOCOL_VERSION_2025_06
        );
        // Below the floor → fallback.
        assert_eq!(
            negotiate_protocol_version(Some("2024-01-01")),
            MCP_PROTOCOL_VERSION_FALLBACK
        );
    }

    #[test]
    fn header_accepts_supported_versions() {
        for version in SUPPORTED_PROTOCOL_VERSIONS {
            let mut headers = HeaderMap::new();
            headers.insert("MCP-Protocol-Version", version.parse().unwrap());
            assert_eq!(protocol_version_from_headers(&headers), Ok(*version));
        }
    }

    #[test]
    fn header_missing_falls_back() {
        let headers = HeaderMap::new();
        assert_eq!(
            protocol_version_from_headers(&headers),
            Ok(MCP_PROTOCOL_VERSION_FALLBACK)
        );
    }

    #[test]
    fn header_unsupported_version_is_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("MCP-Protocol-Version", "2099-01-01".parse().unwrap());
        let err = protocol_version_from_headers(&headers).expect_err("must reject");
        assert!(err.contains("Unsupported"));
        assert!(err.contains(MCP_PROTOCOL_VERSION_LATEST));
    }

    #[test]
    fn rich_tool_shape_gated_by_version() {
        assert!(supports_rich_tool_shape(MCP_PROTOCOL_VERSION_LATEST));
        assert!(supports_rich_tool_shape(MCP_PROTOCOL_VERSION_2025_06));
        assert!(!supports_rich_tool_shape(MCP_PROTOCOL_VERSION_FALLBACK));
    }

    #[test]
    fn card_tools_present_for_2026_absent_for_fallback() {
        let names = |version| {
            tool_definitions(version)
                .into_iter()
                .map(|t| t.name)
                .collect::<Vec<_>>()
        };
        assert!(names(MCP_PROTOCOL_VERSION_LATEST).contains(&"agent_get_card".to_string()));
        assert!(names(MCP_PROTOCOL_VERSION_2025_06).contains(&"agent_get_card".to_string()));
        assert!(!names(MCP_PROTOCOL_VERSION_FALLBACK).contains(&"agent_get_card".to_string()));
    }

    #[test]
    fn client_info_parsed_from_meta() {
        let params = json!({
            "_meta": {
                "io.modelcontextprotocol/clientInfo": { "name": "my-app", "version": "1.2.3" }
            }
        });
        assert_eq!(
            client_info_from_params(&params),
            Some(("my-app".to_string(), "1.2.3".to_string()))
        );
    }

    #[test]
    fn client_info_defaults_missing_fields() {
        let params = json!({
            "_meta": { "io.modelcontextprotocol/clientInfo": {} }
        });
        assert_eq!(
            client_info_from_params(&params),
            Some(("unknown".to_string(), "unknown".to_string()))
        );
    }

    #[test]
    fn client_info_absent_returns_none() {
        assert_eq!(client_info_from_params(&json!({})), None);
    }

    fn req(method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: method.to_string(),
            params,
        }
    }

    #[test]
    fn routing_headers_absent_is_ok() {
        let headers = HeaderMap::new();
        assert!(validate_routing_headers(&headers, &req("tools/list", json!({}))).is_ok());
    }

    #[test]
    fn routing_headers_matching_is_ok() {
        let mut headers = HeaderMap::new();
        headers.insert("Mcp-Method", "tools/call".parse().unwrap());
        headers.insert("Mcp-Name", "agent_run".parse().unwrap());
        let request = req("tools/call", json!({ "name": "agent_run" }));
        assert!(validate_routing_headers(&headers, &request).is_ok());
    }

    #[test]
    fn duplicate_routing_header_method_is_rejected() {
        let mut headers = HeaderMap::new();
        headers.append("Mcp-Method", "tools/call".parse().unwrap());
        headers.append("Mcp-Method", "tools/list".parse().unwrap());
        let request = req("tools/call", json!({ "name": "agent_run" }));
        let err = validate_routing_headers(&headers, &request)
            .expect_err("duplicate method header must reject");
        assert!(err.contains("Duplicate Mcp-Method"));
    }

    #[test]
    fn duplicate_routing_header_name_is_rejected() {
        let mut headers = HeaderMap::new();
        headers.append("Mcp-Name", "agent_run".parse().unwrap());
        headers.append("Mcp-Name", "session_get_status".parse().unwrap());
        let request = req("tools/call", json!({ "name": "agent_run" }));
        let err = validate_routing_headers(&headers, &request)
            .expect_err("duplicate name header must reject");
        assert!(err.contains("Duplicate Mcp-Name"));
    }

    #[test]
    fn routing_header_method_mismatch_is_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("Mcp-Method", "tools/list".parse().unwrap());
        let err = validate_routing_headers(&headers, &req("tools/call", json!({})))
            .expect_err("mismatch must reject");
        assert!(err.contains("Mcp-Method"));
    }

    #[test]
    fn routing_header_name_mismatch_is_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("Mcp-Name", "discover".parse().unwrap());
        let request = req("tools/call", json!({ "name": "agent_run" }));
        let err =
            validate_routing_headers(&headers, &request).expect_err("name mismatch must reject");
        assert!(err.contains("Mcp-Name"));
    }

    #[test]
    fn routing_header_name_ignored_for_non_tools_call() {
        // `Mcp-Name` only cross-checks against `tools/call` bodies.
        let mut headers = HeaderMap::new();
        headers.insert("Mcp-Name", "anything".parse().unwrap());
        assert!(validate_routing_headers(&headers, &req("tools/list", json!({}))).is_ok());
    }
}
