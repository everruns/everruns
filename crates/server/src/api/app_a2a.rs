// App A2A (Agent2Agent) ingress — JSON-RPC + API key authenticated invocation.
//
// Design Decision: A2A channels use endpoint-scoped routes
// (`POST /v1/e/{channel_id}/a2a`) so a single app can expose multiple
// agent-to-agent endpoints with independent keys, agent cards, and session
// routing. App-and-channel routes remain permanent aliases.
//
// Supported methods: `message/send` (single JSON-RPC response),
// `message/stream` (SSE stream of JSON-RPC frames), `tasks/get` (poll task
// state), and `tasks/cancel` (terminate the in-flight task). Task identity
// is the underlying SessionId; state is derived from session turn lifecycle
// events. Other methods return JSON-RPC `-32601 Method not found`.
// See `knowledge/integrations/a2a-channel.md`.

use crate::domains::common::CommandErrorKind;
use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{ConnectInfo, OriginalUri, Path, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, post},
};
use everruns_core::events::EventData;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::a2a_signing::{
    A2A_SIGNATURE_HEADER, A2A_TIMESTAMP_HEADER, A2aReplayStore, SignatureCheckError,
    now_unix_seconds, verify_signature,
};
use crate::api::app_endpoint_auth::{
    AppEndpointAuthError, AppEndpointAuthVerifier, LegacyEndpointAuth,
};
use crate::api::channel_rate_limit::ChannelRateLimiter;
use crate::api::common::ErrorResponse;
use crate::api::sse::SseConnectionTracker;
use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::domains::apps::{
    A2aInvocationRequest, hash_a2a_api_key, invoke_a2a_app_channel,
    invoke_a2a_app_channel_with_hook,
};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::event_delivery::EventDelivery;
use crate::middleware::RequestId;
use crate::security::constant_time_eq;
use crate::storage::{EncryptionService, StorageBackend};

const A2A_PROTOCOL_VERSION: &str = "1.0";
const A2A_AGENT_VERSION: &str = "0.1";
const A2A_PROTOCOL_BINDING_JSONRPC: &str = "JSONRPC";

// THREAT[TM-A2A-005]: Method gating — only the listed methods reach the
// session pipeline. Allowing arbitrary A2A methods would expose code paths we
// have not audited for prompt injection or task-state forgery.
const METHOD_MESSAGE_SEND: &str = "message/send";
const METHOD_MESSAGE_STREAM: &str = "message/stream";
const METHOD_TASKS_GET: &str = "tasks/get";
const METHOD_TASKS_CANCEL: &str = "tasks/cancel";
// The linked Rust A2A client still emits legacy PascalCase JSON-RPC method
// names while the current A2A endpoint contract uses slash-delimited names.
// Keep the compatibility aliases at the method gate so they map to the same
// audited handlers without widening the accepted method surface.
const METHOD_MESSAGE_SEND_LEGACY: &str = "SendMessage";
const METHOD_MESSAGE_STREAM_LEGACY: &str = "SendStreamingMessage";
const METHOD_TASKS_GET_LEGACY: &str = "GetTask";
const METHOD_TASKS_CANCEL_LEGACY: &str = "CancelTask";

#[derive(Clone)]
pub struct AppA2aState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_delivery: EventDelivery,
    pub sse_tracker: Arc<SseConnectionTracker>,
    pub rate_limiter: ChannelRateLimiter,
    pub auth_verifier: AppEndpointAuthVerifier,
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
}

impl AppA2aState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Arc<dyn everruns_worker::AgentRunner>,
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
            auth_verifier: AppEndpointAuthVerifier::new(),
            replay_store,
            frontend_url,
        }
    }
}

pub fn routes(state: AppA2aState) -> Router {
    Router::new()
        .route(
            "/v1/apps/{app_id}/a2a/{channel_id}",
            post(invoke_a2a_legacy),
        )
        .route(
            "/v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json",
            get(agent_card_legacy),
        )
        .route("/v1/e/{channel_id}/a2a", post(invoke_a2a_endpoint))
        .route(
            "/v1/e/{channel_id}/a2a/.well-known/agent-card.json",
            get(agent_card_endpoint),
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
        METHOD_MESSAGE_SEND | METHOD_MESSAGE_SEND_LEGACY => METHOD_MESSAGE_SEND,
        METHOD_MESSAGE_STREAM | METHOD_MESSAGE_STREAM_LEGACY => METHOD_MESSAGE_STREAM,
        METHOD_TASKS_GET | METHOD_TASKS_GET_LEGACY => METHOD_TASKS_GET,
        METHOD_TASKS_CANCEL | METHOD_TASKS_CANCEL_LEGACY => METHOD_TASKS_CANCEL,
        other => other,
    }
}

fn legacy_task_json(mut task: Value) -> Value {
    if let Some(obj) = task.as_object_mut() {
        obj.remove("kind");
        if let Some(state) = obj
            .get_mut("status")
            .and_then(Value::as_object_mut)
            .and_then(|status| status.get_mut("state"))
            && let Some(state_label) = state.as_str()
        {
            let legacy = match state_label {
                "submitted" => "TASK_STATE_SUBMITTED",
                "working" => "TASK_STATE_WORKING",
                "completed" => "TASK_STATE_COMPLETED",
                "failed" => "TASK_STATE_FAILED",
                "canceled" => "TASK_STATE_CANCELED",
                "input_required" => "TASK_STATE_INPUT_REQUIRED",
                "rejected" => "TASK_STATE_REJECTED",
                "auth_required" => "TASK_STATE_AUTH_REQUIRED",
                _ => "TASK_STATE_UNSPECIFIED",
            };
            *state = Value::String(legacy.to_string());
        }
    }
    task
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
    State(state): State<AppA2aState>,
    Path((app_id, channel_id)): Path<(String, String)>,
    req_id: Option<axum::Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    invoke_a2a(
        state,
        app_id,
        channel_id,
        req_id,
        connect_info,
        headers,
        body,
    )
    .await
}

#[utoipa::path(
    description = "Invoke a published A2A endpoint with JSON-RPC 2.0. Authentication follows the endpoint channel configuration.",
    post,
    path = "/v1/e/{channel_id}/a2a",
    params(("channel_id" = String, Path, description = "A2A endpoint channel ID")),
    request_body(content = serde_json::Value, content_type = "application/json"),
    responses(
        (status = 200, description = "JSON-RPC response or event stream"),
        (status = 400, description = "Invalid JSON-RPC request"),
        (status = 401, description = "Missing or invalid endpoint credentials", body = ErrorResponse),
        (status = 404, description = "Endpoint not found, app not published, or channel disabled", body = ErrorResponse),
        (status = 429, description = "Per-channel or SSE connection limit exceeded", body = ErrorResponse)
    ),
    tag = "apps"
)]
pub async fn invoke_a2a_endpoint(
    State(state): State<AppA2aState>,
    Path(channel_id): Path<String>,
    req_id: Option<axum::Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let app_id = match endpoint_app_id(&state, &channel_id).await {
        Ok(app_id) => app_id,
        Err(err) => return err.into_response(),
    };
    invoke_a2a(
        state,
        app_id,
        channel_id,
        req_id,
        connect_info,
        headers,
        body,
    )
    .await
}

async fn invoke_a2a(
    state: AppA2aState,
    app_id: String,
    channel_id: String,
    req_id: Option<axum::Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
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

    // Method gate. THREAT[TM-A2A-005]: only the audited methods reach the
    // session pipeline; everything else returns -32601 with no side effects.
    let requested_method = parsed.method.clone();
    match normalize_a2a_method(&requested_method) {
        METHOD_MESSAGE_SEND => handle_message_send(
            &state,
            auth,
            parsed,
            rpc_id,
            MessageSendContext {
                app_id,
                channel_id,
                req_id,
            },
            requested_method == METHOD_MESSAGE_SEND_LEGACY,
        )
        .await,
        METHOD_MESSAGE_STREAM => {
            handle_message_stream(&state, auth, parsed, rpc_id, app_id, channel_id, req_id).await
        }
        METHOD_TASKS_GET => {
            handle_tasks_get(
                &state,
                auth,
                parsed,
                rpc_id,
                requested_method == METHOD_TASKS_GET_LEGACY,
            )
            .await
        }
        METHOD_TASKS_CANCEL => {
            handle_tasks_cancel(
                &state,
                auth,
                parsed,
                rpc_id,
                requested_method == METHOD_TASKS_CANCEL_LEGACY,
            )
            .await
        }
        other => (
            StatusCode::OK,
            rpc_error(
                rpc_id,
                -32601,
                format!(
                    "Method not found: {other} (supported: message/send, message/stream, tasks/get, tasks/cancel)",
                ),
            ),
        )
            .into_response(),
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
    channel_public_id: everruns_provider::typed_id::AppChannelId,
    session_mode: everruns_platform::SessionBinding,
}

async fn authenticate_request(
    state: &AppA2aState,
    app_id: &str,
    channel_id: &str,
    headers: &HeaderMap,
    peer_addr: Option<std::net::SocketAddr>,
    body: &[u8],
) -> Result<AuthorizedA2a, (StatusCode, Json<ErrorResponse>)> {
    let (app, channel) =
        crate::api::app_ingress::resolve_endpoint(&state.db, state.encryption.as_ref(), channel_id)
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
    if channel.channel_type != everruns_platform::ChannelType::A2a {
        return Err(not_found());
    }
    // THREAT[TM-AUTHZ-006]: Anonymous A2A ingress must never reach a non-live
    // endpoint, and every request must present the per-channel API key before
    // session creation. Liveness is resolved before auth so a caller cannot
    // distinguish a misconfigured endpoint from a bad key.
    if let Err(reason) = crate::api::app_ingress::endpoint_liveness(&app, &channel) {
        tracing::debug!(
            app_id = %app.public_id,
            endpoint_id = %channel.public_id,
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
        if auth.mode == everruns_platform::AppEndpointAuthMode::ApiKey {
            verify_a2a_api_key(headers, &config.api_key_hash)?;
        } else {
            state
                .auth_verifier
                .verify(
                    auth,
                    headers,
                    LegacyEndpointAuth {
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
    let answer = parse_ask_user_answer_part(&parts).map_err(
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

/// Find the `ask_user` answer a caller put on the message, if any.
///
/// The payload under [`ASK_USER_ANSWER_KEY`] is the
/// `/v1/sessions/{id}/question-answers` request body verbatim. Reusing that
/// type rather than minting an A2A-shaped twin is what keeps every surface
/// answering `ask_user` in one vocabulary; the schema advertised on the way
/// out describes exactly this shape.
fn parse_ask_user_answer_part(
    parts: &[Value],
) -> Result<Option<crate::api::question_answers::QuestionAnswersRequest>, serde_json::Error> {
    for part in parts {
        // Same kind/type/duck-typing tolerance as the text parts above.
        let kind = part.get("kind").or_else(|| part.get("type"));
        let is_data = match kind {
            Some(kind) => kind.as_str() == Some("data"),
            None => part.get("data").is_some(),
        };
        if !is_data {
            continue;
        }
        let Some(payload) = part
            .get("data")
            .and_then(|data| data.get(ASK_USER_ANSWER_KEY))
        else {
            continue;
        };
        return serde_json::from_value(payload.clone()).map(Some);
    }
    Ok(None)
}

async fn handle_message_send(
    state: &AppA2aState,
    auth: AuthorizedA2a,
    parsed: JsonRpcRequest,
    rpc_id: Value,
    ctx: MessageSendContext,
    wrap_legacy_send_response: bool,
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
        return handle_ask_user_answer(
            state,
            &auth,
            parsed_msg.task_id.as_deref(),
            answer,
            rpc_id,
            wrap_legacy_send_response,
        )
        .await;
    }

    // task_id is generated up front; the durable workflow that this dispatch
    // schedules is async, so the initial response is always non-terminal
    // (`submitted`). Subsequent `tasks/get` polls derive the current state
    // from the session's turn lifecycle events, where the task corresponds
    // to the most recent turn for the underlying session.
    let task_id = Uuid::now_v7().to_string();
    let request_id = ctx.req_id.map(|axum::Extension(id)| id.0);

    let result = match invoke_a2a_app_channel(
        &state.db,
        state.encryption.as_ref(),
        &state.session_service,
        &state.message_service,
        A2aInvocationRequest {
            app_id: ctx.app_id,
            channel_id: ctx.channel_id,
            params: parsed.params,
            text: parsed_msg.text,
            message_id: parsed_msg.message_id,
            task_id: task_id.clone(),
            context_id: parsed_msg.context_id,
            role: parsed_msg.role,
        },
        request_id,
    )
    .await
    {
        Ok(result) => result,
        Err(err) => return command_error_response(err).into_response(),
    };

    let task = build_task_json(result.session_id, "submitted", None);
    let result = if wrap_legacy_send_response {
        json!({ "task": legacy_task_json(task) })
    } else {
        task
    };
    (StatusCode::OK, rpc_success(rpc_id, result)).into_response()
}

/// Map a `tasks/get` / `tasks/cancel` JSON-RPC params object to an Everruns
/// session id. Per A2A 0.3, the task lookup `params` carry an `id` field
/// that the client stored from a prior `message/send` / `message/stream`
/// response. We use the underlying session id as the task id, so the lookup
/// is just a session existence check followed by event-derived state
/// computation.
fn task_id_from_params(
    params: &Value,
) -> Result<everruns_provider::typed_id::SessionId, &'static str> {
    let raw = params
        .get("id")
        .and_then(Value::as_str)
        .ok_or("Invalid params: missing required `id`")?;
    raw.parse::<everruns_provider::typed_id::SessionId>()
        .map_err(|_| "Invalid params: `id` is not a known task id")
}

/// THREAT[TM-A2A-012]: `tasks/get` exposes session state to the API-key
/// holder. The lookup is restricted to the same org the API key
/// authenticates against, so a key from one channel cannot read tasks from
/// a session created by a different org. State derivation only consults
/// session lifecycle events; it never echoes prompts, tool args, or LLM
/// outputs back to the caller.
async fn handle_tasks_get(
    state: &AppA2aState,
    auth: AuthorizedA2a,
    parsed: JsonRpcRequest,
    rpc_id: Value,
    legacy_response: bool,
) -> Response {
    let session_id = match task_id_from_params(&parsed.params) {
        Ok(id) => id,
        Err(msg) => return (StatusCode::OK, rpc_error(rpc_id, -32602, msg)).into_response(),
    };

    let session = match state.db.get_session(auth.org_id, session_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (StatusCode::OK, rpc_error(rpc_id, -32001, "Task not found")).into_response();
        }
        Err(err) => return internal_error(err).into_response(),
    };

    // THREAT[TM-A2A-012]: org-level scoping is not enough — the API key is
    // bound to a specific app/channel, and a session belongs to exactly one
    // channel via its routing tags. Reject with -32001 (rather than leaking
    // existence) when the session was created by a different channel.
    if !session_belongs_to_a2a_channel(&session, &auth) {
        return (StatusCode::OK, rpc_error(rpc_id, -32001, "Task not found")).into_response();
    }

    let mut state_label = match derive_task_state_from_events(&state.db, session_id).await {
        Ok(label) => label,
        Err(err) => return internal_error(err).into_response(),
    };

    // EVE-1062: a session parked on `ask_user` is `input_required`, and the
    // question rides `TaskStatus.message` — the turn that asked is still open,
    // so the lifecycle events above report `working` on their own.
    let mut status_message = None;
    match pending_ask_user(&state.db, &session).await {
        Ok(Some(pending)) => {
            let projection =
                project_ask_user(&pending, &session.id.to_string(), &state.frontend_url);
            state_label = projection.state;
            status_message = Some(projection.message);
        }
        Ok(None) => {}
        Err(err) => return internal_error(err).into_response(),
    }

    // EVE-728: surface the task's deterministic structured result (result.json
    // reported via a `result_schema`, EVE-678) as an A2A artifact. Reading is
    // org-scoped (TM-A2A-012) and the channel-binding check above already
    // fenced the session to this API key's channel, so a leaked session id from
    // another channel cannot exfiltrate its result.
    let structured_result = match crate::domains::session_tasks::read_structured_task_result(
        &state.db,
        auth.org_id,
        session_id,
    )
    .await
    {
        Ok(result) => result,
        Err(err) => return internal_error(err).into_response(),
    };

    let mut task = with_status_message(
        build_task_json(session.id, state_label, None),
        status_message,
    );
    if let Some(result) = structured_result
        && let Some(obj) = task.as_object_mut()
    {
        obj.insert(
            "artifacts".to_string(),
            json!([a2a_result_artifact(result)]),
        );
    }
    if legacy_response {
        task = legacy_task_json(task);
    }
    (StatusCode::OK, rpc_success(rpc_id, task)).into_response()
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

// --- `ask_user` over the inbound A2A channel (EVE-1062) ---------------------
//
// A2A 0.3 has no elicitation or form primitive, so a parked `ask_user` call is
// projected onto the extension point the protocol does have: a namespaced,
// schema-declared `DataPart` on `TaskStatus.message`. The same message always
// carries a **text part** rendering the identical question in prose — every
// A2A consumer that shipped before this projection reads text and nothing
// else, so the data part adds information and never replaces it.
//
// The answer travels back as a `DataPart` on a `message/send` naming the same
// task, and lands on the shared question-resolution operation (EVE-1054) that
// the browser card and `/mcp` already use. A text-only reply keeps its current
// meaning: the message supersedes the question, which `MessageService::create`
// resolves as `cancelled`.

/// Envelope key carrying a question set out to an A2A caller.
const ASK_USER_QUESTION_KEY: &str = "everruns/ask_user";
/// Envelope key an A2A caller answers with, on the same task id.
const ASK_USER_ANSWER_KEY: &str = "everruns/ask_user_answer";
/// Envelope key naming where a human completes a `secret` question.
const ASK_USER_AUTH_KEY: &str = "everruns/auth_required";
/// Version of the data-part contract, so a consumer can refuse a shape it does
/// not know rather than guess.
const ASK_USER_DATA_VERSION: u32 = 1;

/// A parked `ask_user` call, projected onto the A2A task.
struct AskUserProjection {
    /// `input_required`, or `auth_required` when a credential is wanted.
    state: &'static str,
    /// `TaskStatus.message`: prose first, typed data second.
    message: Value,
}

/// The `ask_user` question set a session is parked on, if any.
///
/// Turn lifecycle events cannot see this — the turn that asked is still open,
/// so `derive_task_state_from_events` reports `working` for a session that is
/// in fact waiting on a person. The session status is what says it parked.
async fn pending_ask_user(
    db: &Arc<StorageBackend>,
    session: &crate::storage::SessionRow,
) -> anyhow::Result<Option<crate::api::question_answers::PendingQuestions>> {
    if everruns_platform::SessionStatus::from(session.status.as_str())
        != everruns_platform::SessionStatus::WaitingForToolResults
    {
        return Ok(None);
    }
    let events = db
        .list_events(
            session.id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(crate::api::question_answers::QUESTION_LOOKBACK_EVENTS),
        )
        .await?;
    Ok(crate::api::question_answers::pending_from_events(
        &events, None,
    ))
}

/// The `ask_user` call inside a streamed `tool.call_requested`, if there is one.
///
/// The typed twin of `pending_from_events`, which reads the same payload back
/// out of a stored row.
fn pending_ask_user_from_request(
    requested: &everruns_core::events::ToolCallRequestedData,
) -> Option<crate::api::question_answers::PendingQuestions> {
    let call = requested
        .tool_calls
        .iter()
        .find(|call| call.name == everruns_builtins::ask_user::ASK_USER_TOOL_NAME)?;
    let questions = serde_json::from_value(call.arguments.get("questions")?.clone()).ok()?;
    let expires_at = call
        .arguments
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc));
    Some(crate::api::question_answers::PendingQuestions {
        tool_call_id: call.id.clone(),
        questions,
        expires_at,
    })
}

/// Where a person completes a `secret` question, when a UI origin is configured.
fn session_human_url(frontend_url: &str, task_id: &str) -> Option<String> {
    let base = frontend_url.trim_end_matches('/');
    (!base.is_empty()).then(|| format!("{base}/sessions/{task_id}/chat"))
}

/// Project a parked question set onto an A2A task status.
fn project_ask_user(
    pending: &crate::api::question_answers::PendingQuestions,
    task_id: &str,
    frontend_url: &str,
) -> AskUserProjection {
    use everruns_builtins::ask_user::AskUserQuestionKind;

    // THREAT[TM-AGENT-016]: a remote agent is never prompted for a human's
    // credential. A `secret` question projects as `auth_required` carrying a
    // URL a person opens — never as a data part with a field to fill in, and
    // never with anywhere for a value to travel back through.
    if pending
        .questions
        .iter()
        .any(|question| question.kind == AskUserQuestionKind::Secret)
    {
        let url = session_human_url(frontend_url, task_id);
        let wanted: Vec<Value> = pending
            .questions
            .iter()
            .filter(|question| question.kind == AskUserQuestionKind::Secret)
            .map(|question| {
                json!({
                    "header": question.header,
                    "question": question.question,
                    "secret_name": question.secret_name,
                    "purpose": question.purpose,
                })
            })
            .collect();
        let data = json!({
            ASK_USER_AUTH_KEY: {
                "version": ASK_USER_DATA_VERSION,
                "task_id": task_id,
                "reason": "secret_question",
                "url": url.clone(),
                "credentials": wanted,
            }
        });
        return AskUserProjection {
            state: "auth_required",
            message: a2a_status_message(
                task_id,
                vec![
                    json!({ "kind": "text", "text": render_secret_prose(&pending.questions, url.as_deref()) }),
                    json!({ "kind": "data", "data": data }),
                ],
            ),
        };
    }

    let data = json!({
        ASK_USER_QUESTION_KEY: {
            "version": ASK_USER_DATA_VERSION,
            "task_id": task_id,
            "tool_call_id": pending.tool_call_id,
            "expires_at": pending.expires_at.map(|at| at.to_rfc3339()),
            "questions": pending.questions,
            "answer_schema": ask_user_answer_schema(pending),
        }
    });
    AskUserProjection {
        state: "input_required",
        message: a2a_status_message(
            task_id,
            vec![
                json!({ "kind": "text", "text": render_questions_prose(pending, task_id) }),
                json!({ "kind": "data", "data": data }),
            ],
        ),
    }
}

/// An A2A `Message` carrying the projection, for `TaskStatus.message`.
fn a2a_status_message(task_id: &str, parts: Vec<Value>) -> Value {
    json!({
        "kind": "message",
        "role": "agent",
        "messageId": Uuid::now_v7().to_string(),
        "taskId": task_id,
        "contextId": task_id,
        "parts": parts,
    })
}

/// JSON Schema for the answer data part, declared alongside the question.
///
/// This is what makes a bare `DataPart` usable in place of the elicitation
/// primitive A2A does not have: the caller is told the exact object to send
/// back, down to the option labels it is allowed to select.
fn ask_user_answer_schema(pending: &crate::api::question_answers::PendingQuestions) -> Value {
    let answers: Vec<Value> = pending
        .questions
        .iter()
        .map(|question| {
            let labels: Vec<&str> = question
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect();
            let mut properties = serde_json::Map::new();
            properties.insert(
                "id".to_string(),
                json!({ "const": question.id.clone().unwrap_or_default() }),
            );
            let mut selected = json!({
                "type": "array",
                "items": { "enum": labels },
            });
            if !question.multi_select
                && let Some(object) = selected.as_object_mut()
            {
                object.insert("maxItems".to_string(), json!(1));
            }
            properties.insert("selected".to_string(), selected);
            if question.allow_other {
                properties.insert(
                    "other_text".to_string(),
                    json!({
                        "type": ["string", "null"],
                        "description": "Free text, when none of the options fit.",
                    }),
                );
            }
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["id"],
                "properties": properties,
                "description": question.question,
            })
        })
        .collect();

    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": ASK_USER_ANSWER_KEY,
        "type": "object",
        "additionalProperties": false,
        "required": ["answers"],
        "properties": {
            "tool_call_id": { "const": pending.tool_call_id },
            "status": {
                "enum": ["answered", "declined"],
                "default": "answered",
                "description": "`declined` is a finished decision the agent must not re-ask.",
            },
            "answers": {
                "type": "array",
                "minItems": answers.len(),
                "maxItems": answers.len(),
                "description": "One answer per asked question, in any order.",
                "items": { "oneOf": answers },
            },
        },
    })
}

/// The question set in prose, for the caller that reads only text.
fn render_questions_prose(
    pending: &crate::api::question_answers::PendingQuestions,
    task_id: &str,
) -> String {
    let mut out =
        String::from("This task is waiting on an answer before the agent can continue.\n");
    if let Some(expires_at) = pending.expires_at {
        out.push_str(&format!(
            "Unanswered by {}, it continues with the defaults marked below.\n",
            expires_at.to_rfc3339()
        ));
    }
    for (index, question) in pending.questions.iter().enumerate() {
        out.push_str(&format!(
            "\n{}. {} — {}\n",
            index + 1,
            question.header,
            question.question
        ));
        if !question.options.is_empty() {
            out.push_str(if question.multi_select {
                "   Choose any that apply:\n"
            } else {
                "   Choose one:\n"
            });
            for option in &question.options {
                out.push_str(&format!(
                    "   - {}: {}{}\n",
                    option.label,
                    option.description,
                    if option.is_default { " (default)" } else { "" }
                ));
            }
        }
        if question.allow_other {
            out.push_str("   Or answer in your own words with `other_text`.\n");
        }
    }
    out.push_str(&format!(
        "\nAnswer with message/send on taskId {task_id}, carrying a data part whose `data` holds \
         `{ASK_USER_ANSWER_KEY}` — `answer_schema` in the data part of this message gives the \
         exact shape. Replying with text only cancels the question and reaches the agent as an \
         ordinary message instead.\n"
    ));
    out
}

/// A credential request in prose. It says what is wanted and who provides it,
/// and never invites the reader to supply the value.
fn render_secret_prose(
    questions: &[everruns_builtins::ask_user::AskUserQuestion],
    url: Option<&str>,
) -> String {
    use everruns_builtins::ask_user::AskUserQuestionKind;

    let mut out = String::from(
        "This task needs a credential, which is never requested from a calling agent and never \
         travels over A2A. A person with access to the session provides it directly to Everruns; \
         the task stays in auth_required until they do.\n",
    );
    for question in questions
        .iter()
        .filter(|question| question.kind == AskUserQuestionKind::Secret)
    {
        out.push_str(&format!("\n{} — {}\n", question.header, question.question));
        if let Some(name) = &question.secret_name {
            out.push_str(&format!("   Stored as: {name}\n"));
        }
        if let Some(purpose) = &question.purpose {
            out.push_str(&format!("   Used for: {purpose}\n"));
        }
    }
    match url {
        Some(url) => out.push_str(&format!("\nOpen {url} to provide it.\n")),
        None => out.push_str(
            "\nNo public UI origin is configured on this deployment, so there is no link to \
             follow; reach the session owner another way.\n",
        ),
    }
    out
}

/// Attach a projected `TaskStatus.message` to a task object.
fn with_status_message(mut task: Value, message: Option<Value>) -> Value {
    if let Some(message) = message
        && let Some(status) = task.get_mut("status").and_then(Value::as_object_mut)
    {
        status.insert("message".to_string(), message);
    }
    task
}

/// Complete the question set this task is parked on, and resume the turn.
///
/// Validation, attribution and idempotency all belong to the shared resolution
/// operation (EVE-1054) — this function only establishes that the caller is
/// allowed to answer *this* task, and that what it is answering is not a
/// credential.
async fn handle_ask_user_answer(
    state: &AppA2aState,
    auth: &AuthorizedA2a,
    task_id: Option<&str>,
    submission: crate::api::question_answers::QuestionAnswersRequest,
    rpc_id: Value,
    wrap_legacy_send_response: bool,
) -> Response {
    use crate::api::question_answers::{QuestionResolver, ResolveError, SubmittedStatus};
    use everruns_builtins::ask_user::{AskUserAnswer, AskUserQuestionKind, AskUserStatus};

    let invalid = |rpc_id: Value, detail: &str| -> Response {
        (
            StatusCode::OK,
            rpc_error(rpc_id, -32602, format!("Invalid params: {detail}")),
        )
            .into_response()
    };

    let Some(task_id) = task_id else {
        return invalid(
            rpc_id,
            "an ask_user answer must carry message.taskId naming the task that asked",
        );
    };
    let Ok(session_id) = task_id.parse::<everruns_provider::typed_id::SessionId>() else {
        return invalid(rpc_id, "message.taskId is not a known task id");
    };

    let session = match state.db.get_session(auth.org_id, session_id).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            return (StatusCode::OK, rpc_error(rpc_id, -32001, "Task not found")).into_response();
        }
        Err(err) => return internal_error(err).into_response(),
    };
    // THREAT[TM-A2A-012]: the same channel binding `tasks/get` enforces. An API
    // key for one channel must not be able to answer — and so steer — a turn
    // running behind another channel in the same org.
    if !session_belongs_to_a2a_channel(&session, auth) {
        return (StatusCode::OK, rpc_error(rpc_id, -32001, "Task not found")).into_response();
    }

    // THREAT[TM-AGENT-016]: refuse a credential answer at the channel boundary,
    // whatever the payload claims, so "a remote agent is never asked for a
    // human's credential" is a property of A2A rather than of validation
    // downstream.
    match pending_ask_user(&state.db, &session).await {
        Ok(Some(pending))
            if pending
                .questions
                .iter()
                .any(|question| question.kind == AskUserQuestionKind::Secret) =>
        {
            return invalid(
                rpc_id,
                "this task is waiting on a credential, which A2A never carries. A person \
                 completes it from the URL in the task status message.",
            );
        }
        Ok(_) => {}
        Err(err) => return internal_error(err).into_response(),
    }
    if submission
        .answers
        .iter()
        .any(|answer| answer.secret_ref.is_some())
    {
        return invalid(rpc_id, "secret_ref is not accepted over A2A");
    }

    let status = match submission.status {
        SubmittedStatus::Answered => AskUserStatus::Answered,
        SubmittedStatus::Declined => AskUserStatus::Declined,
    };
    let answers: Vec<AskUserAnswer> = submission
        .answers
        .into_iter()
        .map(AskUserAnswer::from)
        .collect();

    let resolver = QuestionResolver {
        db: &state.db,
        session_service: state.session_service.as_ref(),
        event_service: state.message_service.event_service(),
        runner: state.message_service.runner().clone(),
    };
    // Attribution is the channel's, never the payload's: the answer came from
    // an API-key holder, which is exactly what `Caller::internal` records.
    let caller = everruns_core::Caller::internal(auth.org_id);
    if let Err(error) = crate::api::question_answers::resolve_question_answers(
        &resolver,
        &caller,
        session_id,
        submission.tool_call_id.as_deref(),
        status,
        &answers,
    )
    .await
    {
        let detail = match error {
            ResolveError::NoPendingQuestions => {
                "this task is not waiting on a question set".to_string()
            }
            ResolveError::WrongPendingCall => {
                "this task is waiting on a different call".to_string()
            }
            ResolveError::AlreadyResolved => {
                "this question set has already been answered".to_string()
            }
            ResolveError::NotWaiting(current) => {
                format!("this task is not waiting for an answer (status: {current})")
            }
            ResolveError::Invalid(detail) => detail,
            ResolveError::Internal(detail) => {
                tracing::error!(error = %detail, "A2A ask_user answer failed to resolve");
                return internal_error(anyhow::anyhow!("failed to resolve the ask_user answer"))
                    .into_response();
            }
        };
        return invalid(rpc_id, &detail);
    }

    let state_label = match derive_task_state_from_events(&state.db, session_id).await {
        Ok(label) => label,
        Err(err) => return internal_error(err).into_response(),
    };
    let task = build_task_json(session_id, state_label, None);
    let result = if wrap_legacy_send_response {
        json!({ "task": legacy_task_json(task) })
    } else {
        task
    };
    (StatusCode::OK, rpc_success(rpc_id, result)).into_response()
}

/// THREAT[TM-A2A-012]: `tasks/cancel` performs a destructive action on a
/// session — it must respect the same channel binding as `tasks/get`.
async fn handle_tasks_cancel(
    state: &AppA2aState,
    auth: AuthorizedA2a,
    parsed: JsonRpcRequest,
    rpc_id: Value,
    legacy_response: bool,
) -> Response {
    let session_id = match task_id_from_params(&parsed.params) {
        Ok(id) => id,
        Err(msg) => return (StatusCode::OK, rpc_error(rpc_id, -32602, msg)).into_response(),
    };

    let session = match state.db.get_session(auth.org_id, session_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (StatusCode::OK, rpc_error(rpc_id, -32001, "Task not found")).into_response();
        }
        Err(err) => return internal_error(err).into_response(),
    };

    // THREAT[TM-A2A-012]: same channel-binding check as tasks/get.
    if !session_belongs_to_a2a_channel(&session, &auth) {
        return (StatusCode::OK, rpc_error(rpc_id, -32001, "Task not found")).into_response();
    }

    // Determine current task state. If terminal already, return idempotently
    // without re-cancelling — A2A spec requires `tasks/cancel` on a finished
    // task to return the task in its terminal state, not error.
    let current = match derive_task_state_from_events(&state.db, session_id).await {
        Ok(label) => label,
        Err(err) => return internal_error(err).into_response(),
    };

    if matches!(current, "completed" | "canceled" | "failed") {
        let mut task = build_task_json(session.id, current, None);
        if legacy_response {
            task = legacy_task_json(task);
        }
        return (StatusCode::OK, rpc_success(rpc_id, task)).into_response();
    }

    if let Err(err) = cancel_a2a_session_turn(state, session_id).await {
        return internal_error(err).into_response();
    }

    let mut task = build_task_json(session.id, "canceled", None);
    if legacy_response {
        task = legacy_task_json(task);
    }
    (StatusCode::OK, rpc_success(rpc_id, task)).into_response()
}

fn build_task_json(
    session_id: everruns_provider::typed_id::SessionId,
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

/// Walk the session event tail and derive the current task state from the
/// most recent turn lifecycle event.
async fn derive_task_state_from_events(
    db: &Arc<StorageBackend>,
    session_id: everruns_provider::typed_id::SessionId,
) -> anyhow::Result<&'static str> {
    use everruns_core::events::{TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED, TURN_STARTED};
    let filter_types = vec![
        TURN_STARTED.to_string(),
        TURN_COMPLETED.to_string(),
        TURN_FAILED.to_string(),
        TURN_CANCELLED.to_string(),
    ];
    // List events in default (ascending) order; we only need the most recent
    // turn event so a small page is enough. Cap at 64 — turn events are
    // sparse and the most recent one wins.
    let events = db
        .list_events(session_id, None, None, &filter_types, &[], None, Some(64))
        .await?;

    let mut latest: Option<&str> = None;
    for evt in &events {
        latest = Some(evt.event_type.as_str());
    }

    let label = match latest {
        Some(t) if t == TURN_COMPLETED => "completed",
        Some(t) if t == TURN_FAILED => "failed",
        Some(t) if t == TURN_CANCELLED => "canceled",
        Some(t) if t == TURN_STARTED => "working",
        _ => "submitted",
    };
    Ok(label)
}

async fn cancel_a2a_session_turn(
    state: &AppA2aState,
    session_id: everruns_provider::typed_id::SessionId,
) -> anyhow::Result<()> {
    use everruns_core::events::{EventContext, EventRequest, InputMessageData, TurnCancelledData};
    use everruns_core::message::RuntimeMessage;
    use everruns_provider::typed_id::{MessageId, TurnId};

    // Best-effort cancel of the active workflow run. Errors are logged but
    // not surfaced — the turn-cancelled event is what tasks/get keys off.
    if let Err(err) = state.message_service.runner().cancel_run(session_id).await {
        tracing::warn!(session_id = %session_id, error = %err, "A2A tasks/cancel: cancel_run failed");
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

// THREAT[TM-A2A-011]: Streaming widens the per-channel ingress surface from
// a single JSON-RPC response to a long-lived SSE connection that mirrors
// session events. The same auth + method gate runs before the stream opens
// (no events leak before authn). Per-event mapping only translates a small
// allowlist of session events into A2A frames; raw event bodies are not
// echoed back. The stream is bounded by the durable turn lifecycle: we close
// after the first turn-completed/turn-failed event for the session. The
// shared `SseConnectionTracker` enforces global/per-org/per-session limits
// so a single API key cannot open unbounded concurrent streams.
async fn handle_message_stream(
    state: &AppA2aState,
    auth: AuthorizedA2a,
    parsed: JsonRpcRequest,
    rpc_id: Value,
    app_id: String,
    channel_id: String,
    req_id: Option<axum::Extension<RequestId>>,
) -> Response {
    if auth.session_mode != everruns_platform::app::SessionBinding::Ephemeral {
        return (
            StatusCode::OK,
            rpc_error(
                rpc_id,
                -32600,
                "message/stream requires session_mode=session_per_invocation",
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

    // `message/stream` opens a new task; it never resumes the one that asked.
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

    // Per-invocation correlation id used by `A2aInvocationRequest` for
    // request tracing only. The *streamed* `taskId` (which clients use for
    // `tasks/get` / `tasks/cancel`) is set further down to the resolved
    // session/context id so the streaming task identity matches the
    // session-scoped task identity, not this random per-invocation id.
    let invocation_task_id = Uuid::now_v7().to_string();
    let request_id = req_id.map(|axum::Extension(id)| id.0);

    // Subscribe to session events at the safe point — between session
    // resolution and message dispatch. The hook below runs *before* the
    // durable workflow that the dispatched message will trigger, so it
    // cannot miss `output.message.completed` / `turn.*` frames.
    let event_delivery = state.event_delivery.clone();
    let subscription_slot: std::sync::Arc<
        tokio::sync::Mutex<Option<crate::event_delivery::EventSubscription>>,
    > = std::sync::Arc::new(tokio::sync::Mutex::new(None));
    let subscription_slot_hook = subscription_slot.clone();

    let result = match invoke_a2a_app_channel_with_hook(
        &state.db,
        state.encryption.as_ref(),
        &state.session_service,
        &state.message_service,
        A2aInvocationRequest {
            app_id,
            channel_id,
            params: parsed.params,
            text: parsed_msg.text,
            message_id: parsed_msg.message_id,
            task_id: invocation_task_id,
            context_id: parsed_msg.context_id,
            role: parsed_msg.role,
        },
        request_id,
        move |session_id| {
            let event_delivery = event_delivery.clone();
            let slot = subscription_slot_hook.clone();
            async move {
                let sub = event_delivery
                    .subscribe(session_id.uuid())
                    .await
                    .map_err(crate::domains::common::CommandError::internal)?;
                *slot.lock().await = Some(sub);
                Ok(())
            }
        },
    )
    .await
    {
        Ok(result) => result,
        Err(err) => return command_error_response(err).into_response(),
    };

    let session_id_uuid = result.session_id.uuid();
    let context_id = result.session_id.to_string();
    // EVE-A2A: the task identity exposed to A2A clients via SSE must match
    // the session/context id so subsequent `tasks/get` / `tasks/cancel`
    // calls (which look up the task by session id) resolve correctly.
    let stream_task_id = context_id.clone();

    let subscription = match subscription_slot.lock().await.take() {
        Some(sub) => sub,
        None => {
            tracing::error!("A2A streaming hook ran but did not register a subscription");
            return internal_error(anyhow::anyhow!("subscription registration failed"))
                .into_response();
        }
    };

    // Bound the SSE connection against global / per-org / per-session limits
    // so a single API key cannot create unbounded concurrent streams. The
    // guard is held for the lifetime of the stream below.
    let sse_guard = match state.sse_tracker.try_acquire(auth.org_id, session_id_uuid) {
        Ok(guard) => guard,
        Err(rejection) => {
            return ErrorResponse::new(rejection.report("a2a", auth.org_id, &session_id_uuid))
                .into_response(StatusCode::TOO_MANY_REQUESTS)
                .into_response();
        }
    };

    // Initial frame: status-update with state=working so clients see the task
    // immediately even if the runtime takes a moment to emit its first event.
    let initial = stream::iter(vec![Ok::<SseEvent, Infallible>(jsonrpc_sse_frame(
        &rpc_id,
        json!({
            "kind": "status-update",
            "taskId": stream_task_id,
            "contextId": context_id,
            "status": { "state": "working" },
            "final": false,
        }),
    ))]);

    let stream_state = A2aStreamState {
        subscription,
        rpc_id,
        task_id: stream_task_id,
        context_id,
        session_id: session_id_uuid,
        frontend_url: state.frontend_url.clone(),
        finished: false,
        terminal_emitted: false,
    };

    let body_stream = stream::unfold(stream_state, move |mut s| async move {
        if s.finished {
            return None;
        }
        loop {
            let Some(event) = s.subscription.recv().await else {
                if s.terminal_emitted {
                    return None;
                }
                // Subscription closed without a terminal turn event — emit a
                // synthetic failed status-update so clients don't hang.
                let frame = jsonrpc_sse_frame(
                    &s.rpc_id,
                    json!({
                        "kind": "status-update",
                        "taskId": s.task_id,
                        "contextId": s.context_id,
                        "status": { "state": "failed" },
                        "final": true,
                    }),
                );
                s.finished = true;
                s.terminal_emitted = true;
                return Some((Ok::<SseEvent, Infallible>(frame), s));
            };

            if event.session_id.uuid() != s.session_id {
                continue;
            }

            if let Some(frame_value) =
                translate_session_event(&event.data, &s.task_id, &s.context_id, &s.frontend_url)
            {
                let is_final = frame_value
                    .get("final")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let frame = jsonrpc_sse_frame(&s.rpc_id, frame_value);
                if is_final {
                    s.finished = true;
                    s.terminal_emitted = true;
                }
                return Some((Ok::<SseEvent, Infallible>(frame), s));
            }
        }
    });

    // Hold `sse_guard` for the lifetime of the stream so the slot in
    // SseConnectionTracker is released only when the client disconnects.
    let stream_with_guard = initial.chain(body_stream).map(move |event| {
        let _guard = &sse_guard;
        event
    });

    Sse::new(stream_with_guard)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response()
}

struct A2aStreamState {
    subscription: crate::event_delivery::EventSubscription,
    rpc_id: Value,
    task_id: String,
    context_id: String,
    session_id: Uuid,
    /// UI origin, for the `auth_required` projection of a secret question.
    frontend_url: String,
    finished: bool,
    terminal_emitted: bool,
}

/// Translate a small allowlist of session events into the A2A frame body
/// (the JSON that goes inside the JSON-RPC `result`). Returning `None` means
/// the event should be filtered out of the A2A stream.
fn translate_session_event(
    data: &EventData,
    task_id: &str,
    context_id: &str,
    frontend_url: &str,
) -> Option<Value> {
    match data {
        // EVE-1062: a parked `ask_user` call is the one non-terminal stop this
        // stream has. Without a frame the caller waits on a question it cannot
        // see, and the turn never completes; `final: true` closes the stream
        // because the answer arrives as a fresh `message/send` on this task.
        EventData::ToolCallRequested(requested) => {
            let pending = pending_ask_user_from_request(requested)?;
            let projection = project_ask_user(&pending, task_id, frontend_url);
            Some(json!({
                "kind": "status-update",
                "taskId": task_id,
                "contextId": context_id,
                "status": { "state": projection.state, "message": projection.message },
                "final": true,
            }))
        }
        EventData::OutputMessageCompleted(d) => {
            let text = d
                .message
                .content
                .iter()
                .filter_map(|part| match part {
                    everruns_core::ContentPart::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                return None;
            }
            // Emit the assistant text as a Message frame. Using `kind: "message"`
            // matches the A2A streaming envelope that wraps a Message result.
            Some(json!({
                "kind": "message",
                "taskId": task_id,
                "contextId": context_id,
                "messageId": d.message.id.to_string(),
                "role": "agent",
                "parts": [{ "kind": "text", "text": text }],
            }))
        }
        EventData::TurnCompleted(_) => Some(json!({
            "kind": "status-update",
            "taskId": task_id,
            "contextId": context_id,
            "status": { "state": "completed" },
            "final": true,
        })),
        EventData::TurnFailed(_) => Some(json!({
            "kind": "status-update",
            "taskId": task_id,
            "contextId": context_id,
            "status": { "state": "failed" },
            "final": true,
        })),
        EventData::TurnCancelled(_) => Some(json!({
            "kind": "status-update",
            "taskId": task_id,
            "contextId": context_id,
            "status": { "state": "canceled" },
            "final": true,
        })),
        _ => None,
    }
}

fn jsonrpc_sse_frame(rpc_id: &Value, result: Value) -> SseEvent {
    let envelope = json!({
        "jsonrpc": "2.0",
        "id": rpc_id,
        "result": result,
    });
    SseEvent::default().data(envelope.to_string())
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
        crate::domains::common::CommandError {
            kind: CommandErrorKind::Internal(error),
            ..
        } => internal_error(error),
    }
}

/// GET /v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json
#[utoipa::path(
    get,
    path = "/v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json",
    params(
        ("app_id" = String, Path, description = "App ID"),
        ("channel_id" = String, Path, description = "A2A channel ID")
    ),
    responses(
        (status = 200, description = "Agent Card JSON"),
        (status = 404, description = "App or channel not found / unpublished / disabled", body = ErrorResponse),
    ),
    tag = "apps"
)]
pub async fn agent_card_legacy(
    State(state): State<AppA2aState>,
    OriginalUri(original_uri): OriginalUri,
    Path((app_id, channel_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    agent_card(state, original_uri, app_id, channel_id, headers).await
}

#[utoipa::path(
    description = "Get the public Agent Card for a published A2A endpoint.",
    get,
    path = "/v1/e/{channel_id}/a2a/.well-known/agent-card.json",
    params(("channel_id" = String, Path, description = "A2A endpoint channel ID")),
    responses(
        (status = 200, description = "Agent Card JSON"),
        (status = 404, description = "Endpoint not found, app not published, or channel disabled", body = ErrorResponse)
    ),
    tag = "apps"
)]
pub async fn agent_card_endpoint(
    State(state): State<AppA2aState>,
    OriginalUri(original_uri): OriginalUri,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = endpoint_app_id(&state, &channel_id).await?;
    agent_card(state, original_uri, app_id, channel_id, headers).await
}

async fn endpoint_app_id(
    state: &AppA2aState,
    channel_id: &str,
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    crate::api::app_ingress::resolve_endpoint(&state.db, state.encryption.as_ref(), channel_id)
        .await
        .map_err(internal_error)?
        .map(|(app, _)| app.public_id.to_string())
        .ok_or_else(not_found)
}

async fn agent_card(
    state: AppA2aState,
    original_uri: axum::http::Uri,
    app_id: String,
    channel_id: String,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let (app, channel) = crate::api::app_ingress::resolve_endpoint(
        &state.db,
        state.encryption.as_ref(),
        &channel_id,
    )
    .await
    .map_err(internal_error)?
    .ok_or_else(not_found)?;
    if !app.matches_legacy_app_id(&app_id) {
        return Err(not_found());
    }
    if channel.channel_type != everruns_platform::ChannelType::A2a {
        return Err(not_found());
    }
    // The Agent Card is only served for a live endpoint: it advertises the
    // invocation URL and security scheme, so publishing it for a draft or
    // suspended endpoint would leak a surface that refuses traffic.
    if crate::api::app_ingress::endpoint_liveness(&app, &channel).is_err() {
        return Err(not_found());
    }
    let config = channel.a2a_config().ok_or_else(not_found)?;

    // Build the absolute endpoint URL from the actual request URI and inbound
    // Host header. Test and proxy deployments can mount API routes under a
    // prefix such as `/api`; deriving from the original URI preserves it.
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("https");
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok());
    let endpoint_path = original_uri
        .path()
        .strip_suffix("/.well-known/agent-card.json")
        .unwrap_or_else(|| original_uri.path());
    let endpoint = match host {
        Some(host) => format!("{scheme}://{host}{endpoint_path}"),
        None => endpoint_path.to_string(),
    };

    let name = config
        .agent_card_name
        .clone()
        .unwrap_or_else(|| app.name.clone());
    let description = config
        .agent_card_description
        .clone()
        .or_else(|| app.description.clone())
        .unwrap_or_default();

    let (security_schemes, security) = a2a_security_for_config(&config, channel.auth.as_deref());
    let card = json!({
        "name": name,
        "description": description,
        "version": A2A_AGENT_VERSION,
        "supportedInterfaces": [
            {
                "url": endpoint,
                "protocolBinding": A2A_PROTOCOL_BINDING_JSONRPC,
                "protocolVersion": A2A_PROTOCOL_VERSION,
            }
        ],
        "capabilities": {
            // Streaming is only supported on session_per_invocation channels.
            // Shared-session channels reject message/stream because events
            // cannot be safely correlated across concurrent callers.
            "streaming": config.session_mode == everruns_platform::app::SessionBinding::Ephemeral,
            "pushNotifications": false,
            "stateTransitionHistory": false,
        },
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [
            {
                "id": "default",
                "name": app.name,
                "description": description,
                "tags": ["everruns", "a2a"],
            }
        ],
        "securitySchemes": security_schemes,
        "securityRequirements": security,
    });
    Ok(Json(card))
}

fn a2a_security_for_config(
    config: &everruns_platform::A2aChannelConfig,
    auth: Option<&everruns_platform::AppEndpointAuthConfig>,
) -> (Value, Value) {
    let (mut schemes, mut requirements) = base_a2a_security(auth);
    // THREAT[TM-A2A-010]: When the channel opts into HMAC signing, advertise
    // a vendor `everrunsHmacSignature` scheme alongside whichever primary
    // scheme is in use so the calling A2A client knows it must sign on top
    // of authentication.
    if config
        .signing_secret
        .as_deref()
        .is_some_and(|s| !s.is_empty())
    {
        if let Value::Object(map) = &mut schemes {
            map.insert(
                "everrunsHmacSignature".to_string(),
                json!({
                    "apiKeySecurityScheme": {
                        "location": "header",
                        "name": A2A_SIGNATURE_HEADER,
                        "description": "HMAC-SHA256 over v0:{timestamp}:{channel_scope}:{body}; pair with X-Everruns-A2A-Timestamp",
                    }
                }),
            );
        }
        if let Value::Array(arr) = &mut requirements {
            if let Some(Value::Object(first)) = arr.first_mut() {
                first.insert("everrunsHmacSignature".to_string(), json!([]));
            } else {
                arr.push(json!({ "everrunsHmacSignature": [] }));
            }
        }
    }
    (schemes, requirements)
}

fn base_a2a_security(auth: Option<&everruns_platform::AppEndpointAuthConfig>) -> (Value, Value) {
    let Some(auth) = auth else {
        return (
            json!({ "apiKey": { "httpAuthSecurityScheme": { "scheme": "bearer" } } }),
            json!([{ "apiKey": [] }]),
        );
    };
    match (&auth.mode, auth.provider.as_ref()) {
        (everruns_platform::AppEndpointAuthMode::HttpBasic, _) => (
            json!({ "httpBasic": { "httpAuthSecurityScheme": { "scheme": "basic" } } }),
            json!([{ "httpBasic": [] }]),
        ),
        (
            everruns_platform::AppEndpointAuthMode::GoogleOidc,
            Some(everruns_platform::AppEndpointAuthProviderConfig::GoogleOidc { .. }),
        ) => (
            json!({
                "googleOidc": {
                    "openIdConnectSecurityScheme": {
                        "openIdConnectUrl": "https://accounts.google.com/.well-known/openid-configuration"
                    }
                }
            }),
            json!([{ "googleOidc": auth.requirements.scopes.clone() }]),
        ),
        (
            everruns_platform::AppEndpointAuthMode::Oidc,
            Some(everruns_platform::AppEndpointAuthProviderConfig::Oidc { issuer, .. }),
        ) => {
            let discovery = format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            );
            (
                json!({
                    "oidc": {
                        "openIdConnectSecurityScheme": {
                            "openIdConnectUrl": discovery
                        }
                    }
                }),
                json!([{ "oidc": auth.requirements.scopes.clone() }]),
            )
        }
        // The linked A2A schema models OAuth2 as concrete OpenAPI flows. An
        // introspection-only channel has no token URL to publish, so advertise
        // generic bearer auth rather than fabricating an unusable OAuth flow.
        (everruns_platform::AppEndpointAuthMode::OAuth2Introspection, _) => (
            json!({ "oauth2Bearer": { "httpAuthSecurityScheme": { "scheme": "bearer" } } }),
            json!([{ "oauth2Bearer": auth.requirements.scopes.clone() }]),
        ),
        (everruns_platform::AppEndpointAuthMode::Mtls, _) => (
            json!({ "mtls": { "mtlsSecurityScheme": {} } }),
            json!([{ "mtls": [] }]),
        ),
        (everruns_platform::AppEndpointAuthMode::Anonymous, _) => (json!({}), json!([])),
        _ => (
            json!({ "apiKey": { "httpAuthSecurityScheme": { "scheme": "bearer" } } }),
            json!([{ "apiKey": [] }]),
        ),
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

fn a2a_auth_error_response(error: AppEndpointAuthError) -> (StatusCode, Json<ErrorResponse>) {
    match error {
        AppEndpointAuthError::Unauthorized => unauthorized(),
        AppEndpointAuthError::Misconfigured => forbidden("A2A auth is misconfigured"),
        AppEndpointAuthError::ProviderUnavailable => {
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
    fn oauth2_introspection_security_advertises_bearer_auth() {
        let config = everruns_platform::A2aChannelConfig {
            api_key_hash: "hash".to_string(),
            api_key_prefix: "evra2a_abcd...".to_string(),
            session_mode: everruns_platform::app::SessionBinding::Endpoint,
            message: "{{a2a.text}}".to_string(),
            agent_card_name: None,
            agent_card_description: None,
            rate_limit_per_minute: None,
            auth: Some(everruns_platform::AppEndpointAuthConfig {
                mode: everruns_platform::AppEndpointAuthMode::OAuth2Introspection,
                provider: Some(
                    everruns_platform::AppEndpointAuthProviderConfig::OAuth2Introspection {
                        introspection_url: "https://auth.example.test/introspect".to_string(),
                        client_id: None,
                        client_secret: None,
                        client_secret_configured: false,
                    },
                ),
                requirements: everruns_platform::AppEndpointAuthRequirements {
                    audiences: vec![],
                    scopes: vec!["app:invoke".to_string()],
                    claims: serde_json::Map::new(),
                    subjects: vec![],
                    groups: vec![],
                    domains: vec![],
                },
            }),
            signing_secret: None,
        };

        let (schemes, requirements) = a2a_security_for_config(&config, config.auth.as_ref());

        assert_eq!(
            schemes["oauth2Bearer"]["httpAuthSecurityScheme"]["scheme"],
            "bearer"
        );
        assert_eq!(requirements, json!([{ "oauth2Bearer": ["app:invoke"] }]));
        serde_json::from_value::<std::collections::HashMap<String, a2a::SecurityScheme>>(schemes)
            .expect("securitySchemes should parse as linked A2A security schemes");
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

    #[test]
    fn translate_turn_completed_emits_terminal_status_update() {
        use everruns_core::events::TurnCompletedData;
        use everruns_provider::typed_id::TurnId;
        let data = EventData::TurnCompleted(TurnCompletedData {
            turn_id: TurnId::new(),
            iterations: 1,
            duration_ms: Some(10),
            usage: None,
            input_content: None,
            final_message_id: None,
            final_answer_preview: None,
            time_to_first_token_ms: None,
            tool_call_count: None,
            llm_call_count: None,
            status: None,
        });
        let frame = translate_session_event(&data, "task-1", "ctx-1", "").unwrap();
        assert_eq!(frame["kind"], "status-update");
        assert_eq!(frame["taskId"], "task-1");
        assert_eq!(frame["contextId"], "ctx-1");
        assert_eq!(frame["status"]["state"], "completed");
        assert_eq!(frame["final"], true);
    }

    #[test]
    fn translate_turn_failed_emits_terminal_status_update() {
        use everruns_core::events::TurnFailedData;
        use everruns_provider::typed_id::TurnId;
        let data = EventData::TurnFailed(TurnFailedData {
            turn_id: TurnId::new(),
            error: "boom".into(),
            error_code: None,
            error_fields: None,
            error_disclosure: None,
        });
        let frame = translate_session_event(&data, "task-1", "ctx-1", "").unwrap();
        assert_eq!(frame["status"]["state"], "failed");
        assert_eq!(frame["final"], true);
    }

    #[test]
    fn translate_unrelated_event_returns_none() {
        use everruns_core::events::OutputMessageStartedData;
        use everruns_provider::typed_id::{MessageId, TurnId};
        let data = EventData::OutputMessageStarted(OutputMessageStartedData {
            reasoning_state: None,
            turn_id: TurnId::new(),
            message_id: MessageId::new(),
            model: None,
            iteration: None,
            phase: None,
        });
        assert!(translate_session_event(&data, "t", "c", "").is_none());
    }

    fn choice_question() -> everruns_builtins::ask_user::AskUserQuestion {
        everruns_builtins::ask_user::AskUserQuestion {
            kind: everruns_builtins::ask_user::AskUserQuestionKind::Choice,
            id: Some("target".to_string()),
            header: "Target".to_string(),
            question: "Which environment should I deploy to?".to_string(),
            multi_select: false,
            allow_other: true,
            options: vec![
                everruns_builtins::ask_user::AskUserOption {
                    label: "Staging".to_string(),
                    description: "Safe, reversible.".to_string(),
                    is_default: true,
                },
                everruns_builtins::ask_user::AskUserOption {
                    label: "Production".to_string(),
                    description: "Live traffic.".to_string(),
                    is_default: false,
                },
            ],
            secret_name: None,
            purpose: None,
        }
    }

    fn secret_question() -> everruns_builtins::ask_user::AskUserQuestion {
        everruns_builtins::ask_user::AskUserQuestion {
            kind: everruns_builtins::ask_user::AskUserQuestionKind::Secret,
            id: Some("stripe_key".to_string()),
            header: "Stripe key".to_string(),
            question: "Which Stripe restricted key should I use?".to_string(),
            multi_select: false,
            allow_other: false,
            options: Vec::new(),
            secret_name: Some("STRIPE_API_KEY".to_string()),
            purpose: Some("Read-only charge lookups.".to_string()),
        }
    }

    fn pending(
        questions: Vec<everruns_builtins::ask_user::AskUserQuestion>,
    ) -> crate::api::question_answers::PendingQuestions {
        crate::api::question_answers::PendingQuestions {
            tool_call_id: "call_1".to_string(),
            questions,
            expires_at: None,
        }
    }

    /// The text part is the whole compatibility story: a consumer that only
    /// reads text must still learn what was asked.
    #[test]
    fn ask_user_projection_carries_prose_and_typed_data() {
        let projection = project_ask_user(&pending(vec![choice_question()]), "task-1", "");
        assert_eq!(projection.state, "input_required");
        let parts = projection.message["parts"].as_array().unwrap();

        let text = parts
            .iter()
            .find(|part| part["kind"] == "text")
            .and_then(|part| part["text"].as_str())
            .expect("a text part");
        assert!(text.contains("Which environment should I deploy to?"));
        assert!(text.contains("Staging: Safe, reversible. (default)"));
        assert!(text.contains(ASK_USER_ANSWER_KEY));

        let data = parts
            .iter()
            .find(|part| part["kind"] == "data")
            .map(|part| &part["data"][ASK_USER_QUESTION_KEY])
            .expect("a data part");
        assert_eq!(data["tool_call_id"], "call_1");
        assert_eq!(data["questions"][0]["id"], "target");
        assert_eq!(
            data["answer_schema"]["properties"]["answers"]["items"]["oneOf"][0]["properties"]["selected"]
                ["items"]["enum"],
            json!(["Staging", "Production"])
        );
        // Single-select is expressed in the schema, not only in the prose.
        assert_eq!(
            data["answer_schema"]["properties"]["answers"]["items"]["oneOf"][0]["properties"]["selected"]
                ["maxItems"],
            json!(1)
        );
    }

    /// THREAT[TM-AGENT-016]: the credential question is never projected as
    /// something the calling agent could answer.
    #[test]
    fn secret_question_projects_as_auth_required_with_a_url() {
        let projection = project_ask_user(
            &pending(vec![secret_question()]),
            "session_1",
            "https://app.example.test/",
        );
        assert_eq!(projection.state, "auth_required");
        let rendered = projection.message.to_string();
        assert!(
            !rendered.contains(ASK_USER_QUESTION_KEY),
            "no answerable question set: {rendered}"
        );
        assert!(!rendered.contains("answer_schema"), "{rendered}");
        let parts = projection.message["parts"].as_array().unwrap();
        let auth = parts
            .iter()
            .find(|part| part["kind"] == "data")
            .map(|part| &part["data"][ASK_USER_AUTH_KEY])
            .expect("a data part");
        assert_eq!(auth["reason"], "secret_question");
        assert_eq!(
            auth["url"],
            "https://app.example.test/sessions/session_1/chat"
        );
    }

    /// A deployment with no configured UI origin still parks correctly; it just
    /// has no link to hand over.
    #[test]
    fn secret_question_without_a_frontend_url_still_projects_auth_required() {
        let projection = project_ask_user(&pending(vec![secret_question()]), "session_1", "");
        assert_eq!(projection.state, "auth_required");
        let parts = projection.message["parts"].as_array().unwrap();
        let auth = parts
            .iter()
            .find(|part| part["kind"] == "data")
            .map(|part| &part["data"][ASK_USER_AUTH_KEY])
            .expect("a data part");
        assert!(auth["url"].is_null());
    }

    /// A typed answer is the one message shape that legitimately has no text.
    #[test]
    fn parse_message_params_accepts_a_data_part_answer_without_text() {
        let params = json!({
            "message": {
                "role": "user",
                "taskId": "session_1",
                "parts": [{
                    "kind": "data",
                    "data": {
                        ASK_USER_ANSWER_KEY: {
                            "tool_call_id": "call_1",
                            "answers": [{ "id": "target", "selected": ["Staging"] }]
                        }
                    }
                }],
            }
        });
        let parsed = parse_message_params(&params).unwrap();
        assert_eq!(parsed.task_id.as_deref(), Some("session_1"));
        let answer = parsed.answer.expect("the answer was recognised");
        assert_eq!(answer.tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(answer.answers[0].selected, vec!["Staging".to_string()]);
    }

    /// A data part that is not an answer leaves the text rule alone.
    #[test]
    fn parse_message_params_ignores_unrelated_data_parts() {
        let params = json!({
            "message": {
                "role": "user",
                "parts": [{ "kind": "data", "data": { "something/else": { "a": 1 } } }],
            }
        });
        assert!(parse_message_params(&params).is_err());
    }

    #[test]
    fn parse_message_params_rejects_a_malformed_answer_part() {
        let params = json!({
            "message": {
                "role": "user",
                "parts": [{
                    "kind": "data",
                    "data": { ASK_USER_ANSWER_KEY: { "answers": "not an array" } }
                }],
            }
        });
        assert!(parse_message_params(&params).is_err());
    }

    /// A parked question is the one non-terminal stop the stream has; without
    /// this frame the caller waits on a question it cannot see.
    #[test]
    fn translate_parked_ask_user_emits_a_final_input_required_frame() {
        use everruns_core::events::ToolCallRequestedData;
        let data = EventData::ToolCallRequested(ToolCallRequestedData {
            tool_calls: vec![everruns_provider::tool_types::ToolCall {
                id: "call_1".to_string(),
                name: everruns_builtins::ask_user::ASK_USER_TOOL_NAME.to_string(),
                arguments: json!({ "questions": [choice_question()] }),
            }],
            tool_summaries: Vec::new(),
            headline: None,
            completed_headline: None,
        });
        let frame = translate_session_event(&data, "task-1", "ctx-1", "").unwrap();
        assert_eq!(frame["status"]["state"], "input_required");
        assert_eq!(frame["final"], true);
        assert!(
            frame["status"]["message"]["parts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|part| part["kind"] == "text")
        );
    }

    /// Any other client-side tool keeps its current meaning: no A2A frame.
    #[test]
    fn translate_non_ask_user_tool_call_is_filtered_out() {
        use everruns_core::events::ToolCallRequestedData;
        let data = EventData::ToolCallRequested(ToolCallRequestedData {
            tool_calls: vec![everruns_provider::tool_types::ToolCall {
                id: "call_1".to_string(),
                name: "setup_connection".to_string(),
                arguments: json!({}),
            }],
            tool_summaries: Vec::new(),
            headline: None,
            completed_headline: None,
        });
        assert!(translate_session_event(&data, "task-1", "ctx-1", "").is_none());
    }

    #[test]
    fn jsonrpc_sse_frame_wraps_result_in_envelope() {
        let frame = jsonrpc_sse_frame(&Value::String("req-1".into()), json!({"hello": "world"}));
        let json_field = format!("{frame:?}");
        assert!(json_field.contains("req-1"));
        assert!(json_field.contains("hello"));
    }
}
