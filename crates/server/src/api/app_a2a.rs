// App A2A (Agent2Agent) ingress — JSON-RPC + API key authenticated invocation.
//
// Design Decision: A2A channels are app-scoped and channel-scoped
// (`POST /v1/apps/{app_id}/a2a/{channel_id}`) so a single app can expose
// multiple agent-to-agent endpoints with independent keys, agent cards, and
// session routing.
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
    invoke_a2a_app_channel_with_hook, queries as app_queries,
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
        runner: Arc<dyn everruns_scale::RunController>,
        notifications_enabled: bool,
        event_delivery: EventDelivery,
        sse_tracker: Arc<SseConnectionTracker>,
        rate_limiter: ChannelRateLimiter,
        replay_store: A2aReplayStore,
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
        }
    }
}

pub fn routes(state: AppA2aState) -> Router {
    Router::new()
        .route("/v1/apps/{app_id}/a2a/{channel_id}", post(invoke_a2a))
        .route(
            "/v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json",
            get(agent_card),
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
pub async fn invoke_a2a(
    State(state): State<AppA2aState>,
    Path((app_id, channel_id)): Path<(String, String)>,
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
    session_mode: everruns_platform::app::InvocationSessionMode,
}

async fn authenticate_request(
    state: &AppA2aState,
    app_id: &str,
    channel_id: &str,
    headers: &HeaderMap,
    peer_addr: Option<std::net::SocketAddr>,
    body: &[u8],
) -> Result<AuthorizedA2a, (StatusCode, Json<ErrorResponse>)> {
    let app = app_queries::get_by_public_id_unscoped(&state.db, state.encryption.as_ref(), app_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;

    // THREAT[TM-TENANT-002]: An unauthenticated caller must not be able to tell
    // "app does not exist" apart from "app exists but is not published / the
    // channel is disabled / misconfigured". Every such case collapses to a
    // single generic 404 (matching the FCP channel in `api/fcp.rs`); the real
    // reason is logged server-side only.
    if app.status != everruns_platform::AppStatus::Published {
        tracing::debug!(app_id = %app.public_id, status = ?app.status, "A2A request rejected: app not published");
        return Err(not_found());
    }

    let channel_id_typed = channel_id
        .parse::<everruns_provider::typed_id::AppChannelId>()
        .map_err(|e| bad_request(format!("Invalid channel ID: {e}")))?;
    let channel = app.channel_by_id(&channel_id_typed).ok_or_else(not_found)?;
    if channel.channel_type != everruns_platform::ChannelType::A2a {
        return Err(not_found());
    }
    // THREAT[TM-AUTHZ-006]: Anonymous A2A ingress must never reach draft or
    // disabled app channels, and every request must present the per-channel
    // API key before session creation.
    if !channel.enabled {
        tracing::debug!(app_id = %app.public_id, "A2A request rejected: channel disabled");
        return Err(not_found());
    }

    let Some(config) = channel.a2a_config() else {
        tracing::error!(app_id = %app.public_id, "A2A channel config did not deserialize");
        return Err(not_found());
    };

    if let Some(auth) = config.auth.as_ref() {
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
    if text.trim().is_empty() {
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
    })
}

async fn handle_message_send(
    state: &AppA2aState,
    _auth: AuthorizedA2a,
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

    let state_label = match derive_task_state_from_events(&state.db, session_id).await {
        Ok(label) => label,
        Err(err) => return internal_error(err).into_response(),
    };

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

    let mut task = build_task_json(session.id, state_label, None);
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
    use everruns_core::message::Message;
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
        InputMessageData::new(Message::user("A2A client requested cancellation.")),
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
    if auth.session_mode != everruns_platform::app::InvocationSessionMode::SessionPerInvocation {
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
            return ErrorResponse::new(rejection.to_string())
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
                translate_session_event(&event.data, &s.task_id, &s.context_id)
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
    finished: bool,
    terminal_emitted: bool,
}

/// Translate a small allowlist of session events into the A2A frame body
/// (the JSON that goes inside the JSON-RPC `result`). Returning `None` means
/// the event should be filtered out of the A2A stream.
fn translate_session_event(data: &EventData, task_id: &str, context_id: &str) -> Option<Value> {
    match data {
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
pub async fn agent_card(
    State(state): State<AppA2aState>,
    OriginalUri(original_uri): OriginalUri,
    Path((app_id, channel_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let app = app_queries::get_by_public_id_unscoped(&state.db, state.encryption.as_ref(), &app_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    // Agent Card is only published when the app is live and the channel is on.
    if app.status != everruns_platform::AppStatus::Published {
        return Err(not_found());
    }
    let channel_id_typed = channel_id
        .parse::<everruns_provider::typed_id::AppChannelId>()
        .map_err(|_| not_found())?;
    let channel = app.channel_by_id(&channel_id_typed).ok_or_else(not_found)?;
    if channel.channel_type != everruns_platform::ChannelType::A2a || !channel.enabled {
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

    let (security_schemes, security) = a2a_security_for_config(&config);
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
            "streaming": config.session_mode == everruns_platform::app::InvocationSessionMode::SessionPerInvocation,
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

fn a2a_security_for_config(config: &everruns_platform::A2aChannelConfig) -> (Value, Value) {
    let (mut schemes, mut requirements) = base_a2a_security(config);
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

fn base_a2a_security(config: &everruns_platform::A2aChannelConfig) -> (Value, Value) {
    let Some(auth) = config.auth.as_ref() else {
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
            session_mode: everruns_platform::app::InvocationSessionMode::SharedSession,
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

        let (schemes, requirements) = a2a_security_for_config(&config);

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
        let frame = translate_session_event(&data, "task-1", "ctx-1").unwrap();
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
        let frame = translate_session_event(&data, "task-1", "ctx-1").unwrap();
        assert_eq!(frame["status"]["state"], "failed");
        assert_eq!(frame["final"], true);
    }

    #[test]
    fn translate_unrelated_event_returns_none() {
        use everruns_core::events::OutputMessageStartedData;
        use everruns_provider::typed_id::{MessageId, TurnId};
        let data = EventData::OutputMessageStarted(OutputMessageStartedData {
            turn_id: TurnId::new(),
            message_id: MessageId::new(),
            model: None,
            iteration: None,
            phase: None,
        });
        assert!(translate_session_event(&data, "t", "c").is_none());
    }

    #[test]
    fn jsonrpc_sse_frame_wraps_result_in_envelope() {
        let frame = jsonrpc_sse_frame(&Value::String("req-1".into()), json!({"hello": "world"}));
        let json_field = format!("{frame:?}");
        assert!(json_field.contains("req-1"));
        assert!(json_field.contains("hello"));
    }
}
