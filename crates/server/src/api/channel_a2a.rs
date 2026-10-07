// Agent channel A2A (Agent2Agent) ingress — JSON-RPC + API key authenticated invocation.
//
// Design Decision: A2A channels use channel-scoped routes
// (`POST /v1/channels/{channel_id}/a2a`) so a single app can expose multiple
// agent-to-agent channels with independent keys, agent cards, and session
// routing. App-and-channel routes remain permanent aliases.
//
// Speaks A2A 1.0 and 0.3 on the same URL, negotiated per request by the
// `A2A-Version` service parameter (`wire.rs`). The same URL also carries the
// A2A 1.0 HTTP+JSON binding (`http_json.rs`) in front of the same handlers. Supported operations: send
// (blocking by default in 1.0), streaming send, get task, cancel task. Task
// identity is the underlying SessionId; state and outputs are derived from the
// session's latest turn (`task_view.rs`). Other operations return the A2A error
// for an unsupported operation. See `knowledge/integrations/a2a-channel.md`.

use crate::domains::common::CommandErrorKind;
use std::sync::Arc;

use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{ConnectInfo, OriginalUri, Path, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::a2a_signing::{
    A2A_SIGNATURE_HEADER, A2A_TIMESTAMP_HEADER, A2aReplayStore, SignatureCheckError,
    now_unix_seconds, verify_signature,
};
use crate::api::channel_auth::{ChannelAuthError, ChannelAuthVerifier, LegacyChannelAuth};
use crate::api::channel_ingress;
use crate::api::channel_rate_limit::ChannelRateLimiter;
use crate::api::common::ErrorResponse;
use crate::api::sse::SseConnectionTracker;
use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::domains::agent_channels::{
    A2aInvocationRequest, hash_a2a_api_key, invoke_channel_a2a_with_hook,
};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::event_delivery::EventDelivery;
use crate::middleware::RequestId;
use crate::security::constant_time_eq;
use crate::storage::{EncryptionService, StorageBackend};

// Two cohesive pieces live beside this file rather than in it: the Agent Card
// (discovery, no request path) and the `ask_user` projection (EVE-1062).
// `agent_card` is `pub` so `openapi.rs` can name its documented handlers.
pub mod agent_card;
pub use push::A2aPushListener;
pub(crate) mod ask_user;
mod http_json;
mod push;
mod stream;
mod task_view;
mod tasks;
mod wire;

use wire::{Binding, WireVersion};

const A2A_AGENT_VERSION: &str = "0.1";
const A2A_PROTOCOL_BINDING_JSONRPC: &str = "JSONRPC";
const A2A_PROTOCOL_BINDING_HTTP_JSON: &str = "HTTP+JSON";

// THREAT[TM-A2A-005]: Method gating — only the listed methods reach the
// session pipeline. Allowing arbitrary A2A methods would expose code paths we
// have not audited for prompt injection or task-state forgery.
const METHOD_MESSAGE_SEND: &str = "message/send";
const METHOD_MESSAGE_STREAM: &str = "message/stream";
const METHOD_TASKS_GET: &str = "tasks/get";
const METHOD_TASKS_CANCEL: &str = "tasks/cancel";
const METHOD_TASKS_LIST: &str = "tasks/list";
const METHOD_TASKS_SUBSCRIBE: &str = "tasks/resubscribe";
// A2A 1.0 names the same operations in PascalCase (spec §9.4). Both spellings
// map to the same audited handlers; the response shape follows the negotiated
// wire version, not the spelling.
const METHOD_MESSAGE_SEND_V1: &str = "SendMessage";
const METHOD_MESSAGE_STREAM_V1: &str = "SendStreamingMessage";
const METHOD_TASKS_GET_V1: &str = "GetTask";
const METHOD_TASKS_CANCEL_V1: &str = "CancelTask";
const METHOD_TASKS_LIST_V1: &str = "ListTasks";
const METHOD_TASKS_SUBSCRIBE_V1: &str = "SubscribeToTask";

#[derive(Clone)]
pub struct ChannelA2aState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_delivery: EventDelivery,
    pub sse_tracker: Arc<SseConnectionTracker>,
    pub rate_limiter: ChannelRateLimiter,
    pub auth_verifier: ChannelAuthVerifier,
    pub replay_store: A2aReplayStore,
    /// UI origin, used to hand a `secret` question to a human instead of
    /// asking the calling agent for the credential (EVE-1062). Empty when no
    /// frontend URL is configured; the projection then carries no link.
    pub frontend_url: String,
}

struct MessageSendContext {
    app_id: String,
    channel_id: String,
    req_id: Option<axum::Extension<RequestId>>,
    version: WireVersion,
    binding: Binding,
}

impl ChannelA2aState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Arc<dyn everruns_core::host::TurnBackend>,
        notifications_enabled: bool,
        event_delivery: EventDelivery,
        sse_tracker: Arc<SseConnectionTracker>,
        rate_limiter: ChannelRateLimiter,
        replay_store: A2aReplayStore,
        frontend_url: String,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(
                db.clone(),
                runner,
                notifications_enabled,
                event_delivery.clone(),
            )),
            db,
            encryption,
            event_delivery,
            sse_tracker,
            rate_limiter,
            auth_verifier: ChannelAuthVerifier::new(),
            replay_store,
            frontend_url,
        }
    }
}

pub fn routes(state: ChannelA2aState) -> Router {
    http_json::routes(Router::new())
        .route(
            "/v1/apps/{app_id}/a2a/{channel_id}",
            post(invoke_a2a_legacy),
        )
        .route(
            "/v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json",
            get(agent_card::agent_card_legacy),
        )
        .route("/v1/channels/{channel_id}/a2a", post(invoke_a2a_channel))
        .route("/v1/e/{channel_id}/a2a", post(invoke_a2a_channel))
        .route(
            "/v1/channels/{channel_id}/a2a/.well-known/agent-card.json",
            get(agent_card::agent_card_channel),
        )
        .route(
            "/v1/e/{channel_id}/a2a/.well-known/agent-card.json",
            get(agent_card::agent_card_channel),
        )
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    code: i32,
    message: String,
}

fn rpc_success(id: Value, result: Value) -> Json<JsonRpcResponse> {
    Json(JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    })
}

fn rpc_error(id: Value, code: i32, message: impl Into<String>) -> Json<JsonRpcResponse> {
    Json(JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
        }),
    })
}

fn normalize_a2a_method(method: &str) -> &str {
    match method {
        METHOD_MESSAGE_SEND | METHOD_MESSAGE_SEND_V1 => METHOD_MESSAGE_SEND,
        METHOD_MESSAGE_STREAM | METHOD_MESSAGE_STREAM_V1 => METHOD_MESSAGE_STREAM,
        METHOD_TASKS_GET | METHOD_TASKS_GET_V1 => METHOD_TASKS_GET,
        METHOD_TASKS_CANCEL | METHOD_TASKS_CANCEL_V1 => METHOD_TASKS_CANCEL,
        METHOD_TASKS_LIST | METHOD_TASKS_LIST_V1 => METHOD_TASKS_LIST,
        METHOD_TASKS_SUBSCRIBE | METHOD_TASKS_SUBSCRIBE_V1 => METHOD_TASKS_SUBSCRIBE,
        other => other,
    }
}

/// The A2A error for a defined operation this endpoint does not offer (spec
/// §5.4), so a client learns why rather than seeing a bare "method not found".
fn unsupported_operation(method: &str) -> Option<(i32, &'static str)> {
    match method {
        "GetExtendedAgentCard" | "agent/getAuthenticatedExtendedCard" => {
            Some((-32007, "No extended Agent Card is configured"))
        }
        _ => None,
    }
}

/// POST /v1/apps/{app_id}/a2a/{channel_id}
#[utoipa::path(
    post,
    path = "/v1/apps/{app_id}/a2a/{channel_id}",
    params(
        ("app_id" = String, Path, description = "App ID"),
        ("channel_id" = String, Path, description = "A2A channel ID")
    ),
    request_body(content = serde_json::Value, content_type = "application/json"),
    responses(
        (status = 200, description = "JSON-RPC 2.0 response. For message/send and tasks/* the body is a single JSON envelope; for message/stream the body is text/event-stream of JSON-RPC envelopes. tasks/get and tasks/cancel surface -32001 Task not found for unknown task ids."),
        (status = 401, description = "Missing or invalid API key", body = ErrorResponse),
        (status = 404, description = "App or channel not found, not published, or channel disabled (collapsed to a single generic 404 to prevent app-existence enumeration)", body = ErrorResponse),
        (status = 429, description = "Per-channel A2A rate limit exceeded, or SSE connection limit reached for the org/session", body = ErrorResponse),
    ),
    tag = "apps"
)]
pub async fn invoke_a2a_legacy(
    State(state): State<ChannelA2aState>,
    Path((app_id, channel_id)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    req_id: Option<axum::Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request = A2aHttpRequest {
        uri,
        req_id,
        connect_info,
        headers,
        body,
    };
    invoke_a2a(state, app_id, channel_id, request).await
}

#[utoipa::path(
    description = "Invoke a published A2A endpoint with JSON-RPC 2.0. Authentication follows the endpoint channel configuration.",
    post,
    path = "/v1/channels/{channel_id}/a2a",
    params(("channel_id" = String, Path, description = "A2A endpoint channel ID")),
    request_body(content = serde_json::Value, content_type = "application/json"),
    responses(
        (status = 200, description = "JSON-RPC response or event stream"),
        (status = 400, description = "Invalid JSON-RPC request"),
        (status = 401, description = "Missing or invalid endpoint credentials", body = ErrorResponse),
        (status = 404, description = "Channel not found, app not published, or channel disabled", body = ErrorResponse),
        (status = 429, description = "Per-channel or SSE connection limit exceeded", body = ErrorResponse)
    ),
    tag = "apps"
)]
pub async fn invoke_a2a_channel(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    req_id: Option<axum::Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let app_id = match channel_app_id(&state, &channel_id).await {
        Ok(app_id) => app_id,
        Err(err) => return err.into_response(),
    };
    let request = A2aHttpRequest {
        uri,
        req_id,
        connect_info,
        headers,
        body,
    };
    invoke_a2a(state, app_id, channel_id, request).await
}

/// The parts of the HTTP request the JSON-RPC handler needs.
struct A2aHttpRequest {
    uri: Uri,
    req_id: Option<axum::Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
}

async fn channel_app_id(
    state: &ChannelA2aState,
    channel_id: &str,
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    channel_ingress::resolve_channel(&state.db, state.encryption.as_ref(), channel_id)
        .await
        .map_err(internal_error)?
        .map(|(app, _)| app.public_id.to_string())
        .ok_or_else(not_found)
}

async fn invoke_a2a(
    state: ChannelA2aState,
    app_id: String,
    channel_id: String,
    request: A2aHttpRequest,
) -> Response {
    let A2aHttpRequest {
        uri,
        req_id,
        connect_info,
        headers,
        body,
    } = request;
    // Parse JSON-RPC envelope first so we can return structured errors with the
    // original `id` echoed back. The raw body bytes are also kept around so
    // the optional HMAC signing check (TM-A2A-010) can verify against the
    // bytes the client actually sent — re-serializing through serde would
    // change whitespace and break the signature.
    let envelope: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                rpc_error(Value::Null, -32600, format!("Invalid Request: {err}")),
            )
                .into_response();
        }
    };
    let parsed: JsonRpcRequest = match serde_json::from_value(envelope) {
        Ok(parsed) => parsed,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                rpc_error(Value::Null, -32600, format!("Invalid Request: {err}")),
            )
                .into_response();
        }
    };
    let rpc_id = parsed.id.clone().unwrap_or(Value::Null);

    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let auth = match authenticate_request(&state, &app_id, &channel_id, &headers, peer_addr, &body)
        .await
    {
        Ok(auth) => auth,
        Err(err) => return err.into_response(),
    };

    let version = match wire::negotiate(&headers, &uri, &parsed.method) {
        Ok(version) => version,
        Err(requested) => {
            return (
                StatusCode::OK,
                rpc_error(
                    rpc_id,
                    wire::VERSION_NOT_SUPPORTED,
                    wire::version_not_supported_message(&requested),
                ),
            )
                .into_response();
        }
    };

    let ctx = MessageSendContext {
        app_id,
        channel_id,
        req_id,
        version,
        binding: Binding::JsonRpc,
    };
    dispatch(&state, auth, parsed, rpc_id, ctx).await
}

/// Run one authenticated A2A operation. Both bindings land here.
async fn dispatch(
    state: &ChannelA2aState,
    auth: AuthorizedA2a,
    parsed: JsonRpcRequest,
    rpc_id: Value,
    ctx: MessageSendContext,
) -> Response {
    let version = ctx.version;
    // Method gate. THREAT[TM-A2A-005]: only the audited methods reach the
    // session pipeline; everything else returns an error with no side effects.
    match normalize_a2a_method(&parsed.method) {
        METHOD_MESSAGE_SEND => handle_message_send(state, auth, parsed, rpc_id, ctx).await,
        METHOD_MESSAGE_STREAM => handle_message_stream(state, auth, parsed, rpc_id, ctx).await,
        METHOD_TASKS_GET => handle_tasks_get(state, auth, parsed, rpc_id, version).await,
        METHOD_TASKS_CANCEL => handle_tasks_cancel(state, auth, parsed, rpc_id, version).await,
        METHOD_TASKS_LIST => tasks::handle_list_tasks(state, auth, parsed, rpc_id, version).await,
        METHOD_TASKS_SUBSCRIBE => {
            tasks::handle_subscribe(state, auth, parsed, rpc_id, version, ctx.binding).await
        }
        other => {
            if let Some(method) = push::push_method(other) {
                return push::handle(state, auth, method, parsed, rpc_id, version).await;
            }
            let (code, message) = unsupported_operation(other).unwrap_or((
                -32601,
                "Method not found (supported: SendMessage, SendStreamingMessage, GetTask, \
                 CancelTask, ListTasks, SubscribeToTask, the push notification config \
                 methods, and their 0.3 names)",
            ));
            (StatusCode::OK, rpc_error(rpc_id, code, message)).into_response()
        }
    }
}

/// Authenticated request context — the app and channel resolution + API key
/// check that both `message/send` and `message/stream` need to perform up
/// front before any session work. Carries the public ids that downstream
/// handlers use to bind a per-call session lookup back to the
/// authenticated channel (TM-A2A-012).
struct AuthorizedA2a {
    org_id: i64,
    app_public_id: String,
    channel_public_id: crate::records::AgentChannelId,
    session_mode: everruns_core::channel::SessionBinding,
}

async fn authenticate_request(
    state: &ChannelA2aState,
    app_id: &str,
    channel_id: &str,
    headers: &HeaderMap,
    peer_addr: Option<std::net::SocketAddr>,
    body: &[u8],
) -> Result<AuthorizedA2a, (StatusCode, Json<ErrorResponse>)> {
    let (app, channel) =
        channel_ingress::resolve_channel(&state.db, state.encryption.as_ref(), channel_id)
            .await
            .map_err(internal_error)?
            .ok_or_else(not_found)?;
    if !app.matches_legacy_app_id(app_id) {
        return Err(not_found());
    }

    // THREAT[TM-TENANT-002]: An unauthenticated caller must not be able to tell
    // "app does not exist" apart from "app exists but is not published / the
    // channel is disabled / misconfigured". Every such case collapses to a
    // single generic 404 (matching the FCP channel in `api/fcp.rs`); the real
    // reason is logged server-side only.
    let channel_id_typed = channel.public_id;
    if channel.channel_type != crate::records::ChannelType::A2a {
        return Err(not_found());
    }
    // THREAT[TM-AUTHZ-006]: Anonymous A2A ingress must never reach a non-live
    // endpoint, and every request must present the per-channel API key before
    // session creation. Liveness is resolved before auth so a caller cannot
    // distinguish a misconfigured endpoint from a bad key.
    if let Err(reason) = channel_ingress::channel_liveness(&app, &channel) {
        tracing::debug!(
            app_id = %app.public_id,
            channel_id = %channel.public_id,
            reason = reason.as_str(),
            "A2A request rejected: endpoint not live"
        );
        return Err(not_found());
    }

    let Some(config) = channel.a2a_config() else {
        tracing::error!(app_id = %app.public_id, "A2A channel config did not deserialize");
        return Err(not_found());
    };

    if let Some(auth) = channel.auth.as_ref() {
        if auth.mode == crate::records::ChannelAuthMode::ApiKey {
            verify_a2a_api_key(headers, &config.api_key_hash)?;
        } else {
            state
                .auth_verifier
                .verify(
                    auth,
                    headers,
                    LegacyChannelAuth {
                        shared_secret: None,
                        api_key: None,
                    },
                )
                .await
                .map_err(a2a_auth_error_response)?;
        }
    } else {
        verify_a2a_api_key(headers, &config.api_key_hash)?;
    }

    // THREAT[TM-A2A-010]: Optional Slack-derived HMAC signing — when the
    // channel has a `signing_secret` configured, every request must carry
    // a `(timestamp, signature)` header pair signed against the basestring
    // `v0:{timestamp}:{channel_scope}:{body}` (where `channel_scope` is
    // `{app_id}:{channel_id}`). Channels without a `signing_secret` keep
    // the existing API-key / endpoint-auth behavior. The 5-minute
    // timestamp window plus the signature-keyed dedup store bound the
    // replay surface to the window even if an `Authorization: Bearer` is
    // captured. The scope inside the basestring also prevents
    // cross-channel replay when operators reuse the same `signing_secret`
    // across multiple A2A channels — the replay store is keyed
    // per-channel and would not catch a forwarded request that signed
    // only `v0:{ts}:{body}`.
    //
    // The check runs **after** primary authentication so an unauthenticated
    // caller cannot use signing failures to probe channel existence or
    // grow the replay store. Missing-header / mismatch / replay all
    // collapse to a single 401 response so a remote attacker cannot
    // distinguish the failure modes.
    let channel_scope = format!("{}:{}", app.public_id, channel_id_typed);

    let pending_signature = if let Some(signing_secret) = config.signing_secret.as_deref()
        && !signing_secret.is_empty()
    {
        let timestamp_header = headers
            .get(A2A_TIMESTAMP_HEADER)
            .and_then(|v| v.to_str().ok());
        let signature_header = headers
            .get(A2A_SIGNATURE_HEADER)
            .and_then(|v| v.to_str().ok());
        let signature = match verify_signature(
            timestamp_header,
            signature_header,
            &channel_scope,
            body,
            signing_secret,
            now_unix_seconds(),
        ) {
            Ok(sig) => sig,
            Err(err) => {
                tracing::warn!(
                    app_id = %app.public_id,
                    channel_id = %channel_id_typed,
                    reason = err.as_log_reason(),
                    "A2A signed-request verification failed"
                );
                return Err(unauthorized());
            }
        };
        Some(signature)
    } else {
        None
    };

    // THREAT[TM-A2A-013]: Unattended A2A traffic must respect a configurable
    // per-app, per-IP cap in addition to the global API limit. App owners
    // tune `rate_limit_per_minute` on the A2A channel to bound LLM/budget
    // burn from a runaway counterparty agent. The check runs after the
    // API key comparison so an unauthenticated caller cannot grow the
    // limiter cache or learn whether a channel exists from rate-limit
    // signals. It also runs **before** the signing replay-store record so
    // a rate-limited request does not consume a nonce slot — otherwise an
    // authenticated client could grow / churn the replay store with
    // unique signed traffic even while rate-limit responses bound the
    // session pipeline.
    if let Some(limit) = config.rate_limit_per_minute
        && limit > 0
    {
        let client_ip = extract_client_ip_from_parts(peer_addr, headers);
        // Scope must include the channel id — apps can expose multiple A2A
        // channels with independent `rate_limit_per_minute` settings, and
        // sharing a single `app_id`-keyed bucket would let an attacker
        // alternate between channels with different limits to flush the
        // cached limiter and bypass throttling (TM-A2A-013, Copilot review
        // on PR #1800).
        if state
            .rate_limiter
            .check(&channel_scope, client_ip, limit)
            .await
            .is_err()
        {
            return Err(too_many_requests(
                "A2A rate limit exceeded for this app channel",
            ));
        }
    }

    // Record the verified signature only after the rate limiter has
    // accepted the request — otherwise rate-limited traffic would still
    // burn replay-store slots.
    if let Some(signature) = pending_signature
        && !state
            .replay_store
            .try_record(&channel_scope, &signature)
            .await
    {
        tracing::warn!(
            app_id = %app.public_id,
            channel_id = %channel_id_typed,
            reason = SignatureCheckError::Replay.as_log_reason(),
            "A2A signed-request replay rejected"
        );
        return Err(unauthorized());
    }

    Ok(AuthorizedA2a {
        org_id: app.org_id,
        app_public_id: app.public_id.to_string(),
        channel_public_id: channel_id_typed,
        session_mode: config.session_mode,
    })
}

fn verify_a2a_api_key(
    headers: &HeaderMap,
    expected_hash: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    // THREAT[TM-A2A-001]: API keys are stored as SHA-256 hashes; plaintext is
    // never persisted. We hash the inbound key and compare against the stored
    // hash before doing anything else.
    let provided_key = extract_a2a_api_key(headers).ok_or_else(unauthorized)?;
    let provided_hash = hash_a2a_api_key(&provided_key);
    // THREAT[TM-A2A-002]: constant-time comparison of the SHA-256 hex digests
    // (canonical `crate::security::constant_time_eq`) avoids leaking
    // partial-match timing information to a remote attacker.
    if constant_time_eq(provided_hash.as_bytes(), expected_hash.as_bytes()) {
        Ok(())
    } else {
        Err(unauthorized())
    }
}

/// Verify that a session looked up by an A2A task id belongs to the same
/// app + channel that the API key authenticates against. Without this check
/// an API key for one A2A channel could read or cancel sessions created by
/// any other channel in the same org once the session id leaks.
/// THREAT[TM-A2A-012].
fn session_belongs_to_a2a_channel(
    session: &crate::storage::SessionRow,
    auth: &AuthorizedA2a,
) -> bool {
    let app_tag = format!("app:{}", auth.app_public_id);
    let channel_tag = format!("app_channel:{}", auth.channel_public_id);
    session.tags.iter().any(|t| t == &app_tag) && session.tags.iter().any(|t| t == &channel_tag)
}

/// Pull the joined text from `params.message.parts`, plus the `role`,
/// `messageId`, and `contextId` we propagate through the session template.
struct ParsedMessage {
    text: String,
    role: Option<String>,
    message_id: Option<String>,
    context_id: Option<String>,
    /// `message.taskId` — which task a reply belongs to. Only an `ask_user`
    /// answer needs it; an ordinary message keeps routing by channel binding.
    task_id: Option<String>,
    /// A typed answer to the question set this task is parked on (EVE-1062),
    /// carried as a `DataPart`. `None` for every message that is just a
    /// message, which is what keeps text-only callers unchanged.
    answer: Option<crate::api::question_answers::QuestionAnswersRequest>,
}

fn parse_message_params(params: &Value) -> Result<ParsedMessage, &'static str> {
    let message = params.get("message").cloned().unwrap_or(Value::Null);
    let parts = message
        .get("parts")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let text = parts
        .iter()
        .filter_map(|part| {
            // Spec uses `kind: "text"`; older drafts used `type: "text"`.
            // The linked Rust SDK serializes text parts as `{ "text": ... }`
            // without a discriminator, so accept missing kind/type only when
            // a string `text` field is present.
            let kind = part.get("kind").or_else(|| part.get("type"));
            let is_text = match kind {
                Some(kind) => kind.as_str() == Some("text"),
                None => part.get("text").and_then(Value::as_str).is_some(),
            };
            if is_text {
                part.get("text").and_then(Value::as_str).map(str::to_owned)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    // A typed answer is carried as data, so it is the one message shape that
    // legitimately has no text. Everything else keeps the original rule.
    let answer = ask_user::parse_ask_user_answer_part(&parts).map_err(
        |_| "Invalid params: message.parts carries a malformed everruns/ask_user_answer data part",
    )?;
    if answer.is_none() && text.trim().is_empty() {
        return Err("Invalid params: message.parts must contain at least one non-empty text part");
    }
    Ok(ParsedMessage {
        text,
        role: message
            .get("role")
            .and_then(Value::as_str)
            .map(str::to_owned),
        message_id: message
            .get("messageId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        context_id: message
            .get("contextId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        task_id: message
            .get("taskId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        answer,
    })
}

async fn handle_message_send(
    state: &ChannelA2aState,
    auth: AuthorizedA2a,
    parsed: JsonRpcRequest,
    rpc_id: Value,
    ctx: MessageSendContext,
) -> Response {
    let parsed_msg = match parse_message_params(&parsed.params) {
        Ok(parsed) => parsed,
        Err(msg) => {
            return (StatusCode::OK, rpc_error(rpc_id, -32602, msg)).into_response();
        }
    };

    // An answer to a parked question set is not a new turn (EVE-1062): it
    // completes the `ask_user` call this task is waiting on and resumes the
    // turn that asked. A text-only reply falls through to the ordinary path,
    // where the message supersedes the question.
    if let Some(answer) = parsed_msg.answer {
        return ask_user::handle_ask_user_answer(
            state,
            &auth,
            parsed_msg.task_id.as_deref(),
            answer,
            rpc_id,
            ctx.version,
        )
        .await;
    }

    let continue_session = match continued_session(state, &auth, &parsed_msg).await {
        Ok(session) => session,
        Err(response) => return response.into_response_with(rpc_id),
    };

    // A2A 1.0 §3.2.2: send is blocking unless the caller sets
    // `returnImmediately`; 0.3 blocks only when the caller sets `blocking`.
    let configuration = parsed.params.get("configuration");
    let flag = |name: &str| {
        configuration
            .and_then(|c| c.get(name))
            .and_then(Value::as_bool)
    };
    let blocking = match ctx.version {
        WireVersion::V1_0 => !flag("returnImmediately").unwrap_or(false),
        WireVersion::V0_3 => flag("blocking").unwrap_or(false),
    };
    let push_config = match push::send_config(configuration) {
        Ok(config) => config,
        Err(msg) => return (StatusCode::OK, rpc_error(rpc_id, -32602, msg)).into_response(),
    };

    // Subscribe before dispatch so a blocking call cannot miss the event that
    // settles its own turn.
    let subscription_slot: Arc<
        tokio::sync::Mutex<Option<crate::event_delivery::EventSubscription>>,
    > = Arc::new(tokio::sync::Mutex::new(None));
    let hook_slot = subscription_slot.clone();
    let event_delivery = state.event_delivery.clone();
    let (db, encryption, org_id) = (state.db.clone(), state.encryption.clone(), auth.org_id);
    let version = ctx.version;
    let request_id = ctx.req_id.map(|axum::Extension(id)| id.0);
    let result = match invoke_channel_a2a_with_hook(
        &state.db,
        state.encryption.as_ref(),
        &state.session_service,
        &state.message_service,
        A2aInvocationRequest {
            legacy_app_id: ctx.app_id,
            channel_id: ctx.channel_id,
            params: parsed.params,
            text: parsed_msg.text,
            message_id: parsed_msg.message_id,
            task_id: Uuid::now_v7().to_string(),
            context_id: parsed_msg.context_id,
            role: parsed_msg.role,
            continue_session,
        },
        request_id,
        move |session_id| async move {
            // Registered before dispatch, so the turn cannot settle unseen.
            if let Some(config) = push_config {
                push::store_config(
                    &db,
                    encryption.as_ref(),
                    org_id,
                    session_id,
                    version,
                    &config,
                )
                .await
                .map_err(crate::domains::common::CommandError::internal)?;
            }
            if blocking {
                let subscription = event_delivery
                    .subscribe(session_id.uuid())
                    .await
                    .map_err(crate::domains::common::CommandError::internal)?;
                *hook_slot.lock().await = Some(subscription);
            }
            Ok(())
        },
    )
    .await
    {
        Ok(result) => result,
        Err(err) => return command_error_response(err).into_response(),
    };

    // Non-blocking: the durable workflow runs asynchronously, so the task is
    // `submitted`; callers poll `tasks/get` or use streaming.
    let Some(mut subscription) = subscription_slot.lock().await.take() else {
        let task = build_task_json(result.session_id, "submitted", None);
        return (
            StatusCode::OK,
            rpc_success(rpc_id, wire::send_message_result(ctx.version, task)),
        )
            .into_response();
    };
    task_view::wait_until_settled(
        &mut subscription,
        result.session_id,
        task_view::BLOCKING_SEND_TIMEOUT,
    )
    .await;
    let session = match state.db.get_session(auth.org_id, result.session_id).await {
        Ok(Some(session)) => session,
        Ok(None) => return internal_error(anyhow::anyhow!("A2A session vanished")).into_response(),
        Err(err) => return internal_error(err).into_response(),
    };
    match task_view::load_task(state, &auth, &session).await {
        Ok(task) => (
            StatusCode::OK,
            rpc_success(rpc_id, wire::send_message_result(ctx.version, task)),
        )
            .into_response(),
        Err(err) => internal_error(err).into_response(),
    }
}

/// A JSON-RPC error decided before the request id is in scope.
struct RpcRejection(i32, &'static str);

impl RpcRejection {
    fn into_response_with(self, rpc_id: Value) -> Response {
        (StatusCode::OK, rpc_error(rpc_id, self.0, self.1)).into_response()
    }
}

/// The session a message continues, from its `taskId` or `contextId` (both
/// are the session id in this channel). A2A 3.4.2: a `taskId` MUST name an
/// existing task, so an unknown one is `TaskNotFoundError`. An unknown
/// `contextId` is ignored and the message starts a new context, as before.
/// THREAT[TM-A2A-012]: only a session bound to the authenticating channel can
/// be continued; any other collapses to "not found".
async fn continued_session(
    state: &ChannelA2aState,
    auth: &AuthorizedA2a,
    message: &ParsedMessage,
) -> Result<Option<everruns_contracts::typed_id::SessionId>, RpcRejection> {
    const NOT_FOUND: RpcRejection = RpcRejection(-32001, "Task not found");
    let bound = |raw: Option<&str>| {
        let parsed =
            raw.and_then(|raw| raw.parse::<everruns_contracts::typed_id::SessionId>().ok());
        async move {
            let session_id = parsed?;
            match state.db.get_session(auth.org_id, session_id).await {
                Ok(Some(session)) if session_belongs_to_a2a_channel(&session, auth) => {
                    Some(session.id)
                }
                _ => None,
            }
        }
    };
    if let Some(task_id) = message.task_id.as_deref() {
        return match bound(Some(task_id)).await {
            Some(session_id) => Ok(Some(session_id)),
            None => Err(NOT_FOUND),
        };
    }
    Ok(bound(message.context_id.as_deref()).await)
}

/// Map a `tasks/get` / `tasks/cancel` JSON-RPC params object to an Everruns
/// session id. Per A2A 0.3, the task lookup `params` carry an `id` field
/// that the client stored from a prior `message/send` / `message/stream`
/// response. We use the underlying session id as the task id, so the lookup
/// is just a session existence check followed by event-derived state
/// computation.
fn task_id_from_params(
    params: &Value,
) -> Result<everruns_contracts::typed_id::SessionId, &'static str> {
    let raw = params
        .get("id")
        .and_then(Value::as_str)
        .ok_or("Invalid params: missing required `id`")?;
    raw.parse::<everruns_contracts::typed_id::SessionId>()
        .map_err(|_| "Invalid params: `id` is not a known task id")
}

/// THREAT[TM-A2A-012]: `tasks/get` exposes session state to the API-key
/// holder. The lookup is restricted to the same org the API key
/// authenticates against, so a key from one channel cannot read tasks from
/// a session created by a different org. State derivation only consults
/// session lifecycle events; it never echoes prompts, tool args, or LLM
/// outputs back to the caller.
async fn handle_tasks_get(
    state: &ChannelA2aState,
    auth: AuthorizedA2a,
    parsed: JsonRpcRequest,
    rpc_id: Value,
    version: WireVersion,
) -> Response {
    let session = match bound_task_session(state, &auth, &parsed.params).await {
        Ok(session) => session,
        Err(rejection) => return rejection.into_response_with(rpc_id),
    };
    match task_view::load_task(state, &auth, &session).await {
        Ok(task) => (
            StatusCode::OK,
            rpc_success(rpc_id, wire::task(version, task)),
        )
            .into_response(),
        Err(err) => internal_error(err).into_response(),
    }
}

/// Resolve the `id` of a `tasks/get` / `tasks/cancel` to its session.
/// THREAT[TM-A2A-012]: org scoping is not enough. The API key is bound to one
/// channel and a session belongs to exactly one channel via its routing tags,
/// so a session from another channel collapses to `-32001` rather than
/// leaking its existence.
async fn bound_task_session(
    state: &ChannelA2aState,
    auth: &AuthorizedA2a,
    params: &Value,
) -> Result<crate::storage::SessionRow, RpcRejection> {
    let session_id = task_id_from_params(params).map_err(|msg| RpcRejection(-32602, msg))?;
    match state.db.get_session(auth.org_id, session_id).await {
        Ok(Some(session)) if session_belongs_to_a2a_channel(&session, auth) => Ok(session),
        Ok(_) => Err(RpcRejection(-32001, "Task not found")),
        Err(err) => {
            tracing::error!(error = %err, "A2A task lookup failed");
            Err(RpcRejection(-32603, "Internal error"))
        }
    }
}

/// Wrap a task's structured `result.json` (EVE-678) as an A2A `Artifact` with a
/// single `DataPart`, so `tasks/get` callers receive the deterministic machine
/// result rather than only the last-message / status text. Shape follows the
/// A2A `Artifact` model (`artifactId` + `name` + typed `parts`).
fn a2a_result_artifact(result: Value) -> Value {
    json!({
        "artifactId": "result",
        "name": "result",
        "parts": [{ "kind": "data", "data": result }],
    })
}

/// THREAT[TM-A2A-012]: `tasks/cancel` performs a destructive action on a
/// session — it must respect the same channel binding as `tasks/get`.
async fn handle_tasks_cancel(
    state: &ChannelA2aState,
    auth: AuthorizedA2a,
    parsed: JsonRpcRequest,
    rpc_id: Value,
    version: WireVersion,
) -> Response {
    let session = match bound_task_session(state, &auth, &parsed.params).await {
        Ok(session) => session,
        Err(rejection) => return rejection.into_response_with(rpc_id),
    };

    // A finished task cannot be canceled (spec §3.1.5): TaskNotCancelable.
    match derive_task_state_from_events(&state.db, session.id).await {
        Ok(current) if tasks::is_terminal(current) => {
            return RpcRejection(-32002, "Task cannot be canceled: it has already finished")
                .into_response_with(rpc_id);
        }
        Ok(_) => {}
        Err(err) => return internal_error(err).into_response(),
    }
    if let Err(err) = cancel_a2a_session_turn(state, session.id).await {
        return internal_error(err).into_response();
    }
    let label = "canceled";
    let task = build_task_json(session.id, label, None);
    (
        StatusCode::OK,
        rpc_success(rpc_id, wire::task(version, task)),
    )
        .into_response()
}

fn build_task_json(
    session_id: everruns_contracts::typed_id::SessionId,
    state_label: &str,
    error_message: Option<&str>,
) -> Value {
    let session_id_str = session_id.to_string();
    let mut status = json!({ "state": state_label });
    if let (Some(msg), Some(obj)) = (error_message, status.as_object_mut()) {
        obj.insert(
            "message".to_string(),
            json!({
                "role": "agent",
                "parts": [{ "kind": "text", "text": msg }],
            }),
        );
    }
    json!({
        "id": session_id_str,
        "contextId": session_id_str,
        "status": status,
        "kind": "task",
    })
}

/// The current task state, from the session's latest turn.
async fn derive_task_state_from_events(
    db: &Arc<StorageBackend>,
    session_id: everruns_contracts::typed_id::SessionId,
) -> anyhow::Result<&'static str> {
    Ok(task_view::read_latest_turn(db, session_id).await?.state)
}

async fn cancel_a2a_session_turn(
    state: &ChannelA2aState,
    session_id: everruns_contracts::typed_id::SessionId,
) -> anyhow::Result<()> {
    use everruns_contracts::typed_id::{MessageId, TurnId};
    use everruns_core::events::{EventContext, EventRequest, InputMessageData, TurnCancelledData};
    use everruns_core::message::RuntimeMessage;

    // Best-effort cancel of the active workflow run. Errors are logged but
    // not surfaced — the turn-cancelled event is what tasks/get keys off.
    if let Err(err) = state.message_service.runner().cancel(session_id).await {
        tracing::warn!(session_id = %session_id, error = %err, "A2A tasks/cancel: cancel failed");
    }

    // Re-check terminality before emitting a synthetic turn.cancelled event.
    // Between the pre-check in `handle_tasks_cancel` and this point the
    // workflow may have landed a real turn.completed/turn.failed event; if
    // we always emitted turn.cancelled here, derived state would race-flip
    // a completed task to canceled. Skip emission when the task has already
    // reached a terminal state.
    let already_terminal = matches!(
        derive_task_state_from_events(&state.db, session_id).await?,
        "completed" | "failed" | "canceled"
    );
    if already_terminal {
        return Ok(());
    }

    let turn_id = TurnId::from_uuid(session_id.uuid());
    let input_message_id = MessageId::new();
    let event_service = state.message_service.event_service();

    let cancelled_event = EventRequest::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        TurnCancelledData {
            turn_id,
            reason: Some("A2A tasks/cancel".to_string()),
            usage: None,
        },
    );
    if let Err(err) = event_service.emit(cancelled_event).await {
        tracing::warn!(session_id = %session_id, error = %err, "A2A tasks/cancel: emit turn.cancelled failed");
    }

    let user_message_event = EventRequest::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        InputMessageData::new(RuntimeMessage::user("A2A client requested cancellation.")),
    );
    if let Err(err) = event_service.emit(user_message_event).await {
        tracing::warn!(session_id = %session_id, error = %err, "A2A tasks/cancel: emit user message failed");
    }

    Ok(())
}

/// `message/stream` / `SendStreamingMessage`; auth and the method gate already ran.
async fn handle_message_stream(
    state: &ChannelA2aState,
    auth: AuthorizedA2a,
    parsed: JsonRpcRequest,
    rpc_id: Value,
    ctx: MessageSendContext,
) -> Response {
    if auth.session_mode != crate::records::agent_channel::SessionBinding::Ephemeral {
        return (
            StatusCode::OK,
            rpc_error(
                rpc_id,
                -32004,
                "Streaming requires session_mode=session_per_invocation",
            ),
        )
            .into_response();
    }

    let parsed_msg = match parse_message_params(&parsed.params) {
        Ok(parsed) => parsed,
        Err(msg) => {
            return (StatusCode::OK, rpc_error(rpc_id, -32602, msg)).into_response();
        }
    };

    // Streaming opens a new task; it never resumes the one that asked.
    // Refusing is better than dispatching the answer as a fresh prompt, which
    // would leave the question parked and put the answer in the wrong turn.
    if parsed_msg.answer.is_some() {
        return (
            StatusCode::OK,
            rpc_error(
                rpc_id,
                -32602,
                "Invalid params: answer an ask_user question set with message/send on its task id",
            ),
        )
            .into_response();
    }

    let continue_session = match continued_session(state, &auth, &parsed_msg).await {
        Ok(session) => session,
        Err(rejection) => return rejection.into_response_with(rpc_id),
    };

    // Subscribe to session events at the safe point, between session
    // resolution and message dispatch, so the stream cannot miss the first
    // `output.message.completed` / `turn.*` event of the turn it opened.
    let event_delivery = state.event_delivery.clone();
    let subscription_slot: Arc<
        tokio::sync::Mutex<Option<crate::event_delivery::EventSubscription>>,
    > = Arc::new(tokio::sync::Mutex::new(None));
    let hook_slot = subscription_slot.clone();
    let request_id = ctx.req_id.map(|axum::Extension(id)| id.0);
    let result = match invoke_channel_a2a_with_hook(
        &state.db,
        state.encryption.as_ref(),
        &state.session_service,
        &state.message_service,
        A2aInvocationRequest {
            legacy_app_id: ctx.app_id,
            channel_id: ctx.channel_id,
            params: parsed.params,
            text: parsed_msg.text,
            message_id: parsed_msg.message_id,
            // Request tracing only; the streamed task id is the session id.
            task_id: Uuid::now_v7().to_string(),
            context_id: parsed_msg.context_id,
            role: parsed_msg.role,
            continue_session,
        },
        request_id,
        move |session_id| async move {
            let subscription = event_delivery
                .subscribe(session_id.uuid())
                .await
                .map_err(crate::domains::common::CommandError::internal)?;
            *hook_slot.lock().await = Some(subscription);
            Ok(())
        },
    )
    .await
    {
        Ok(result) => result,
        Err(err) => return command_error_response(err).into_response(),
    };

    let session_id = result.session_id.uuid();
    let Some(subscription) = subscription_slot.lock().await.take() else {
        tracing::error!("A2A streaming hook ran but did not register a subscription");
        return internal_error(anyhow::anyhow!("subscription registration failed")).into_response();
    };

    // Bound the SSE connection against global / per-org / per-session limits
    // so a single API key cannot create unbounded concurrent streams.
    let sse_guard = match state.sse_tracker.try_acquire(auth.org_id, session_id) {
        Ok(guard) => guard,
        Err(rejection) => {
            return ErrorResponse::new(rejection.report("a2a", auth.org_id, &session_id))
                .into_response(StatusCode::TOO_MANY_REQUESTS)
                .into_response();
        }
    };

    // The task identity streamed to the client is the session id, so later
    // `tasks/get` / `tasks/cancel` calls resolve the same task.
    let task_id = result.session_id.to_string();
    stream::sse_response(
        stream::StreamContext {
            subscription,
            rpc_id,
            context_id: task_id.clone(),
            task_id,
            session_id,
            frontend_url: state.frontend_url.clone(),
            version: ctx.version,
            binding: ctx.binding,
            initial_task: build_task_json(result.session_id, "working", None),
        },
        sse_guard,
    )
}

fn command_error_response(
    err: crate::domains::common::CommandError,
) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        crate::domains::common::CommandError {
            kind: CommandErrorKind::BadRequest(msg),
            ..
        } => bad_request(msg),
        crate::domains::common::CommandError {
            kind: CommandErrorKind::Forbidden(msg),
            ..
        } => forbidden(msg),
        crate::domains::common::CommandError {
            kind: CommandErrorKind::NotFound(_),
            ..
        } => not_found(),
        crate::domains::common::CommandError {
            kind: CommandErrorKind::Conflict(msg),
            ..
        } => ErrorResponse::new(msg).into_response(StatusCode::CONFLICT),
        crate::domains::common::CommandError {
            kind: CommandErrorKind::RateLimited(msg),
            ..
        } => ErrorResponse::new(msg).into_response(StatusCode::TOO_MANY_REQUESTS),
        crate::domains::common::CommandError {
            kind: CommandErrorKind::Unprocessable(msg),
            ..
        } => ErrorResponse::new(msg).into_response(StatusCode::UNPROCESSABLE_ENTITY),
        error @ crate::domains::common::CommandError {
            kind: CommandErrorKind::Unavailable(_),
            ..
        } => error.into(),
        crate::domains::common::CommandError {
            kind: CommandErrorKind::Internal(error),
            ..
        } => internal_error(error),
    }
}

fn extract_a2a_api_key(headers: &HeaderMap) -> Option<String> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    auth.strip_prefix("Bearer ").map(ToOwned::to_owned)
}

fn bad_request(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new(message.into()).into_response(StatusCode::BAD_REQUEST)
}

fn forbidden(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new(message.into()).into_response(StatusCode::FORBIDDEN)
}

fn unauthorized() -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new("Invalid or missing A2A API key".to_string())
        .into_response(StatusCode::UNAUTHORIZED)
}

fn service_unavailable(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new(message.into()).into_response(StatusCode::SERVICE_UNAVAILABLE)
}

fn a2a_auth_error_response(error: ChannelAuthError) -> (StatusCode, Json<ErrorResponse>) {
    match error {
        ChannelAuthError::Unauthorized => unauthorized(),
        ChannelAuthError::Misconfigured => forbidden("A2A auth is misconfigured"),
        ChannelAuthError::ProviderUnavailable => {
            service_unavailable("A2A auth provider is unavailable")
        }
    }
}

fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new("App channel not found".to_string()).into_response(StatusCode::NOT_FOUND)
}

fn too_many_requests(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new(message.to_string()).into_response(StatusCode::TOO_MANY_REQUESTS)
}

fn internal_error(error: anyhow::Error) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %error, "Failed to invoke app A2A");
    ErrorResponse::new("Internal server error".to_string())
        .into_response(StatusCode::INTERNAL_SERVER_ERROR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_a2a_api_key_reads_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer evra2a_abc".parse().unwrap(),
        );
        assert_eq!(extract_a2a_api_key(&headers).as_deref(), Some("evra2a_abc"));
    }

    #[test]
    fn extract_a2a_api_key_rejects_missing() {
        let headers = HeaderMap::new();
        assert!(extract_a2a_api_key(&headers).is_none());
    }

    #[test]
    fn parse_message_params_joins_text_parts_and_extracts_metadata() {
        let params = json!({
            "message": {
                "role": "user",
                "messageId": "msg-1",
                "contextId": "ctx-1",
                "parts": [
                    { "kind": "text", "text": "hello" },
                    { "kind": "text", "text": "world" },
                    { "kind": "image", "url": "https://example.com/x.png" },
                ],
            }
        });
        let parsed = parse_message_params(&params).unwrap();
        assert_eq!(parsed.text, "hello\nworld");
        assert_eq!(parsed.role.as_deref(), Some("user"));
        assert_eq!(parsed.message_id.as_deref(), Some("msg-1"));
        assert_eq!(parsed.context_id.as_deref(), Some("ctx-1"));
    }

    #[test]
    fn parse_message_params_rejects_empty_text() {
        let params = json!({
            "message": {
                "role": "user",
                "parts": [{ "kind": "text", "text": "  " }],
            }
        });
        assert!(parse_message_params(&params).is_err());
    }
}
