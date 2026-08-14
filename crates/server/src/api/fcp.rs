// FCP (Free Communication Protocol) app channel — minimal text-first ingress.
//
// Design Decision: FCP is intentionally schema-free at the wire layer. `POST`
// accepts plain text (or `{"message": "..."}`), `GET` returns a Markdown
// handshake, and the response is the final assistant text. Streaming and
// structured envelopes are deliberately omitted; clients negotiate everything
// in natural language during the handshake (see `knowledge/integrations/fcp-channel.md`).
//
// Design Decision: FCP runs its own minimal auth stack — anonymous access
// plus a single shared bearer token, validated by constant-time comparison
// inline. It deliberately does **not** call into `AppEndpointAuthVerifier`
// (used by AG-UI/A2A) or the platform's user-session auth. This keeps the
// public FCP surface narrow, predictable, and decoupled from changes to
// other channels.
//
// Design Decision: FCP has its own `ChannelRateLimiter` namespace. The
// per-app cap counts in a bucket that no other channel can share or
// exhaust. The global API limit still applies; this is an additional
// per-channel layer.
//
// Design Decision: Every response — success **and** error — is rendered as
// Markdown with actionable instructions that point back at the `GET`
// handshake. We never expose internal state (app existence, lifecycle,
// channel config, durable engine details). Error bodies tell the caller
// what to do next, not what failed inside us.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Extension, Router,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::get,
};
use everruns_core::events::{
    OUTPUT_MESSAGE_COMPLETED, OutputMessageCompletedData, TURN_CANCELLED, TURN_FAILED,
    TurnCancelledData, TurnFailedData,
};
use everruns_core::{Caller, ContentPart, ExternalActor};
use everruns_platform::{App, AppStatus, FcpChannelConfig};
use everruns_provider::execution_phase::ExecutionPhase;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::channel_rate_limit::ChannelRateLimiter;
use crate::api::messages::{
    CreateMessageRequest, InputContentPart, InputMessage, MessageRole as ApiMessageRole,
};
use crate::api::public::{PublicError, PublicErrorCode};
use crate::api::sessions::CreateSessionRequest;
use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::event_delivery::EventDelivery;
use crate::execution_metadata;
use crate::middleware::RequestId;
use crate::security::constant_time_eq;
use crate::storage::{EncryptionService, StorageBackend};

const FCP_SESSION_COOKIE: &str = "fcp_session";
const FCP_TOKEN_HEADER: &str = "x-everruns-fcp-token";
const MAX_FCP_BODY_BYTES: usize = 256 * 1024;
const FCP_CONTENT_TYPE: &str = "text/markdown; charset=utf-8";

#[derive(Clone)]
pub struct FcpState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_delivery: EventDelivery,
    /// FCP-only rate limiter. Constructed with a dedicated namespace so it
    /// cannot collide with AG-UI/A2A limiter buckets.
    pub rate_limiter: ChannelRateLimiter,
}

impl FcpState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Arc<dyn everruns_worker::AgentRunner>,
        notifications_enabled: bool,
        event_delivery: EventDelivery,
        rate_limiter: ChannelRateLimiter,
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
            rate_limiter,
        }
    }
}

pub fn routes(state: FcpState) -> Router {
    Router::new()
        .route("/v1/apps/{app_id}/fcp", get(handshake).post(message))
        .with_state(state)
}

/// Resolved channel context, used by both `GET` and `POST`. The lookup is
/// shared so we apply the same "exists at all?" sanitization in one place.
struct FcpContext {
    app: App,
    config: FcpChannelConfig,
}

async fn resolve_context(state: &FcpState, app_id: &str) -> Result<FcpContext, Response> {
    // Sanitization rule: any failure path here returns the same generic
    // not-found body. We must not reveal:
    //   - whether the app id was malformed vs. unknown
    //   - whether an app exists but is not published
    //   - whether an app is published but has no FCP channel
    //   - whether a channel exists but is disabled
    let app = match crate::domains::apps::queries::get_by_public_id_unscoped(
        &state.db,
        state.encryption.as_ref(),
        app_id,
    )
    .await
    {
        Ok(Some(app)) => app,
        Ok(None) => return Err(not_found_response()),
        Err(err) => {
            tracing::error!(error = %err, "FCP app lookup failed");
            return Err(internal_error_response());
        }
    };
    if app.status != AppStatus::Published {
        return Err(not_found_response());
    }
    let channel = match app.fcp_channel() {
        Some(channel) => channel,
        None => return Err(not_found_response()),
    };
    let config = match channel.fcp_config() {
        Some(config) => config,
        None => {
            tracing::error!(app_id = %app.public_id, "FCP channel config did not deserialize");
            return Err(internal_error_response());
        }
    };
    Ok(FcpContext { app, config })
}

/// Verify the caller's token (if one is required) using constant-time
/// comparison. Returns an actionable 401 body on mismatch so clients learn
/// how to authenticate without us echoing the configured token.
fn check_token(headers: &HeaderMap, config: &FcpChannelConfig) -> Result<(), Box<Response>> {
    let token_required = !config.anonymous
        || config
            .token
            .as_deref()
            .map(|t| !t.is_empty())
            .unwrap_or(false);
    if !token_required {
        return Ok(());
    }
    let Some(expected) = config.token.as_deref().filter(|t| !t.is_empty()) else {
        // anonymous=false but no token configured — the app owner has
        // misconfigured the channel. We still don't accept the request and
        // we still don't tell the caller what's wrong.
        return Err(Box::new(unauthorized_response()));
    };
    let provided = match extract_token(headers) {
        Some(t) => t,
        None => return Err(Box::new(unauthorized_response())),
    };
    if !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        return Err(Box::new(unauthorized_response()));
    }
    Ok(())
}

/// Enforce the per-app FCP rate limit. Returns a 429 body that tells the
/// caller exactly when to retry.
async fn check_rate_limit(
    state: &FcpState,
    headers: &HeaderMap,
    peer_addr: Option<std::net::SocketAddr>,
    app: &App,
    config: &FcpChannelConfig,
) -> Result<(), Response> {
    let Some(limit) = config.rate_limit_per_minute else {
        return Ok(());
    };
    if limit == 0 {
        return Ok(());
    }
    let client_ip = extract_client_ip_from_parts(peer_addr, headers);
    if state
        .rate_limiter
        .check(&app.public_id.to_string(), client_ip, limit)
        .await
        .is_err()
    {
        return Err(rate_limited_response(limit));
    }
    Ok(())
}

/// `GET /v1/apps/{app_id}/fcp` — handshake. Always returns the same
/// generic 404 body for unknown apps so the endpoint cannot be used to
/// probe which app ids are real.
#[utoipa::path(
    get,
    path = "/v1/apps/{app_id}/fcp",
    params(("app_id" = String, Path, description = "App ID")),
    responses(
        (
            status = 200,
            description = "Markdown handshake describing the FCP endpoint and how to authenticate. \
                           Always `text/markdown`.",
            content_type = "text/markdown",
        ),
        (
            status = 404,
            description = "No FCP endpoint at this URL. Single sanitized body covers \
                           unknown apps, unpublished apps, apps without an FCP channel, \
                           and disabled FCP channels — operator state is never disclosed.",
            content_type = "text/markdown",
        ),
        (
            status = 429,
            description = "Per-app FCP rate limit exceeded. `Retry-After: 60` header is set.",
            content_type = "text/markdown",
        ),
    ),
    tag = "apps"
)]
pub async fn handshake(
    State(state): State<FcpState>,
    Path(app_id): Path<String>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
) -> Response {
    // Per the FCP SPEC, the handshake is meant to be open. We still apply
    // the per-app rate limit (when configured) so a single client can't
    // hammer the handshake either.
    let context = match resolve_context(&state, &app_id).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    if let Err(resp) =
        check_rate_limit(&state, &headers, peer_addr, &context.app, &context.config).await
    {
        return resp;
    }
    let body = render_handshake(&context.app, &context.config);
    markdown_response(StatusCode::OK, body)
}

/// `POST /v1/apps/{app_id}/fcp` — text-in, text-out.
#[utoipa::path(
    post,
    path = "/v1/apps/{app_id}/fcp",
    params(("app_id" = String, Path, description = "App ID")),
    request_body(
        content = String,
        description = "Plain UTF-8 text, or `application/json` of shape `{\"message\": \"...\"}`. \
                       Maximum 256 KiB.",
        content_type = "text/plain",
    ),
    responses(
        (
            status = 200,
            description = "Agent reply as Markdown. The `Set-Cookie: fcp_session=...` header is \
                           emitted so subsequent POSTs can resume the same conversation.",
            content_type = "text/markdown",
        ),
        (
            status = 400,
            description = "Malformed body. Distinct messages for empty body, non-UTF-8 bytes, \
                           and JSON that did not parse into the expected shape — each Markdown \
                           body points back at the handshake.",
            content_type = "text/markdown",
        ),
        (
            status = 401,
            description = "Token required but missing or wrong. Markdown body explains both \
                           accepted headers (`Authorization: Bearer` and `X-Everruns-FCP-Token`) \
                           and points at the handshake.",
            content_type = "text/markdown",
        ),
        (
            status = 404,
            description = "Same generic 404 as the handshake — operator state is never disclosed.",
            content_type = "text/markdown",
        ),
        (
            status = 410,
            description = "FCP session expired. Body tells the client to drop the `fcp_session` \
                           cookie and POST again.",
            content_type = "text/markdown",
        ),
        (
            status = 413,
            description = "Body exceeds 256 KiB.",
            content_type = "text/markdown",
        ),
        (
            status = 429,
            description = "Per-app FCP rate limit exceeded. `Retry-After: 60` header is set.",
            content_type = "text/markdown",
        ),
        (
            status = 504,
            description = "Agent did not reply within the configured timeout. The same \
                           `fcp_session` cookie is set so the client can retry the same \
                           conversation.",
            content_type = "text/markdown",
        ),
    ),
    tag = "apps"
)]
pub async fn message(
    State(state): State<FcpState>,
    Path(app_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let request_id = req_id.map(|Extension(r)| r.0);

    if body.len() > MAX_FCP_BODY_BYTES {
        return payload_too_large_response();
    }

    let context = match resolve_context(&state, &app_id).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };

    if let Err(resp) =
        check_rate_limit(&state, &headers, peer_addr, &context.app, &context.config).await
    {
        return resp;
    }
    if let Err(resp) = check_token(&headers, &context.config) {
        return *resp;
    }

    let user_text = match extract_user_text(&headers, &body) {
        Ok(text) => text,
        Err(resp) => return *resp,
    };
    if user_text.trim().is_empty() {
        return empty_message_response();
    }

    let resolved = match resolve_session(
        &state,
        &context.app,
        &context.config,
        parse_session_cookie(&headers),
    )
    .await
    {
        Ok(resolved) => resolved,
        Err(resp) => return resp,
    };

    let mut subscription = match state
        .event_delivery
        .clone()
        .subscribe(resolved.session_id)
        .await
    {
        Ok(sub) => sub,
        Err(err) => {
            tracing::error!(error = %err, "FCP event subscription failed");
            return internal_error_response();
        }
    };

    let input_message = match state
        .message_service
        .create(
            CreateMessageContext {
                org_id: context.app.org_id,
                user_id: None,
                harness_id: context.app.harness_id.uuid(),
                agent_id: context.app.agent_id.map(|agent_id| agent_id.uuid()),
                session_id: resolved.session_id,
                event_metadata: Some(execution_metadata::app_message_metadata(
                    context.app.public_id,
                    context.app.owner_principal_id,
                    context.app.agent_identity_id,
                )),
                request_id,
            },
            CreateMessageRequest {
                message: InputMessage {
                    role: ApiMessageRole::User,
                    content: vec![InputContentPart::text(user_text)],
                },
                addressed_participant_id: None,
                controls: None,
                metadata: Some(fcp_message_metadata(&context.app)),
                tags: None,
                external_actor: Some(ExternalActor {
                    actor_id: "fcp".to_string(),
                    actor_name: None,
                    source: "fcp".to_string(),
                    metadata: None,
                }),
            },
        )
        .await
    {
        Ok(msg) => msg,
        Err(err) => {
            tracing::error!(error = %err, "FCP message create failed");
            return internal_error_response();
        }
    };

    let input_message_id = input_message.id.to_string();
    let session_id = resolved.session_id;
    let timeout = Duration::from_secs(context.config.response_timeout_seconds.max(1) as u64);

    let collected = tokio::time::timeout(timeout, async {
        loop {
            let Some(event) = subscription.recv().await else {
                return Err(PublicError::fallback());
            };
            if event.session_id.uuid() != session_id {
                continue;
            }
            let matches_input = event
                .context
                .input_message_id
                .as_ref()
                .map(|id| id.to_string())
                .as_deref()
                == Some(input_message_id.as_str());
            if !matches_input {
                continue;
            }
            match event.event_type.as_str() {
                OUTPUT_MESSAGE_COMPLETED => {
                    let Ok(data) = parse_event_data::<OutputMessageCompletedData>(&event) else {
                        continue;
                    };
                    if matches!(data.message.phase, Some(ExecutionPhase::Commentary)) {
                        continue;
                    }
                    let text = content_parts_to_text(&data.message.content);
                    if text.trim().is_empty() {
                        continue;
                    }
                    return Ok(text);
                }
                TURN_FAILED => {
                    let code = parse_event_data::<TurnFailedData>(&event)
                        .ok()
                        .and_then(|d| d.error_code);
                    return Err(PublicError::from_internal_code(code.as_deref()));
                }
                TURN_CANCELLED if parse_event_data::<TurnCancelledData>(&event).is_ok() => {
                    return Err(PublicError::fallback());
                }
                _ => {}
            }
        }
    })
    .await;

    let mut response = match collected {
        Ok(Ok(text)) => markdown_response(StatusCode::OK, text),
        Ok(Err(public_err)) => turn_error_response(public_err),
        Err(_elapsed) => timeout_response(context.config.response_timeout_seconds),
    };

    // Set the session cookie on every response that resolved a session,
    // including timeouts and provider errors. The session is already
    // recorded; emitting the cookie lets the client retry against the
    // same conversation rather than starting a fresh one.
    if let Some(cookie) =
        build_session_cookie(session_id, context.config.session_expiration_seconds)
        && let Ok(value) = HeaderValue::from_str(&cookie)
    {
        response
            .headers_mut()
            .append(axum::http::header::SET_COOKIE, value);
    }
    response
}

// ----------------------------------------------------------------------------
// Handshake rendering
// ----------------------------------------------------------------------------

fn render_handshake(app: &App, config: &FcpChannelConfig) -> String {
    if let Some(handshake) = config.handshake.as_deref() {
        return handshake.to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!("# {}", app.name));
    if let Some(description) = app.description.as_deref()
        && !description.trim().is_empty()
    {
        lines.push(description.to_string());
    }
    lines.push(String::new());
    lines.push("Send a plain-text message in the body of a `POST` to this URL, or".to_string());
    lines.push(
        "`POST` JSON `{\"message\": \"...\"}` with `Content-Type: application/json`.".to_string(),
    );
    lines.push("Replies are returned as `text/markdown`.".to_string());

    let token_required = !config.anonymous
        || config
            .token
            .as_deref()
            .map(|t| !t.is_empty())
            .unwrap_or(false);
    if token_required {
        lines.push(String::new());
        lines.push(
            "Authentication is required. Send `Authorization: Bearer <token>` or the".to_string(),
        );
        lines.push(
            "`X-Everruns-FCP-Token: <token>` header with the bearer credential issued".to_string(),
        );
        lines.push("to you out of band.".to_string());
    }
    lines.push(String::new());
    if config.session_expiration_seconds > 0 {
        lines.push(format!(
            "Session state is carried by the `{FCP_SESSION_COOKIE}` cookie. Threads expire {}.",
            humanize_seconds(config.session_expiration_seconds)
        ));
    } else {
        lines.push(format!(
            "Session state is carried by the `{FCP_SESSION_COOKIE}` cookie. Threads do not expire."
        ));
    }
    if let Some(limit) = config.rate_limit_per_minute
        && limit > 0
    {
        lines.push(format!(
            "Rate limit: at most {limit} requests per minute per client IP."
        ));
    }
    lines.join("\n")
}

fn humanize_seconds(seconds: u32) -> String {
    if seconds.is_multiple_of(3600) {
        let hours = seconds / 3600;
        if hours == 1 {
            "after 1 hour".to_string()
        } else {
            format!("after {hours} hours")
        }
    } else if seconds.is_multiple_of(60) {
        let minutes = seconds / 60;
        if minutes == 1 {
            "after 1 minute".to_string()
        } else {
            format!("after {minutes} minutes")
        }
    } else {
        format!("after {seconds} seconds")
    }
}

// ----------------------------------------------------------------------------
// Body / session handling
// ----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JsonMessageBody {
    message: Option<String>,
}

fn extract_user_text(headers: &HeaderMap, body: &[u8]) -> Result<String, Box<Response>> {
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let body_text = match std::str::from_utf8(body) {
        Ok(text) => text.to_string(),
        Err(_) => return Err(Box::new(invalid_body_response())),
    };
    if content_type.starts_with("application/json") {
        let parsed: JsonMessageBody = match serde_json::from_str(&body_text) {
            Ok(parsed) => parsed,
            Err(_) => return Err(Box::new(invalid_json_response())),
        };
        return Ok(parsed.message.unwrap_or_default());
    }
    // Per FCP SPEC: "If the body parses as JSON, it MAY be interpreted as
    // such; otherwise it MUST be treated as text." We try JSON opportunistic-
    // ally so the same call works whether the client sets the JSON content
    // type or not, then fall back to raw text.
    if !body_text.is_empty()
        && let Ok(parsed) = serde_json::from_str::<JsonMessageBody>(&body_text)
        && let Some(message) = parsed.message
    {
        return Ok(message);
    }
    Ok(body_text)
}

struct ResolvedSession {
    session_id: Uuid,
}

async fn resolve_session(
    state: &FcpState,
    app: &App,
    config: &FcpChannelConfig,
    cookie_session_id: Option<Uuid>,
) -> Result<ResolvedSession, Response> {
    let routing_tag = format!("fcp:app:{}", app.public_id);
    if let Some(session_id) = cookie_session_id {
        match state.db.get_session(app.org_id, session_id.into()).await {
            Ok(Some(row))
                if row.app_id == Some(app.internal_id) && row.tags.contains(&routing_tag) =>
            {
                if let Some(age) = expired_age_seconds(
                    row.created_at,
                    config.session_expiration_seconds,
                    chrono::Utc::now(),
                ) {
                    tracing::info!(
                        app_id = %app.public_id,
                        session_id = %row.id,
                        age_seconds = age,
                        "Rejecting FCP resume on expired session"
                    );
                    return Err(session_expired_response());
                }
                return Ok(ResolvedSession { session_id });
            }
            Ok(_) => {
                // Stale or unknown cookie — fall through and create a new
                // session rather than stranding the client.
            }
            Err(err) => {
                tracing::error!(error = %err, "FCP session lookup failed");
                return Err(internal_error_response());
            }
        }
    }

    let session = state
        .session_service
        .create_from_app(
            &Caller::internal(app.org_id),
            app.harness_id.uuid(),
            app.agent_id.map(|agent_id| agent_id.uuid()),
            app.agent_id,
            app.internal_id,
            app.owner_principal_id,
            app.resolved_owner_user_id,
            everruns_platform::SessionSource::Fcp,
            CreateSessionRequest {
                source: None,
                workspace_id: None,
                harness_id: Some(app.harness_id),
                harness_name: None,
                agent_id: app.agent_id,
                agent_name: None,
                agent_identity_id: app.agent_identity_id,
                title: Some(format!("FCP session for {}", app.name)),
                goal: None,
                locale: None,
                tags: vec![routing_tag],
                model_id: None,
                capabilities: vec![],
                tools: vec![],
                mcp_servers: Default::default(),
                system_prompt: None,
                initial_files: vec![],
                hints: None,
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                parent_session_id: None,
                forked_from_session_id: None,
                budget_root_session_id: None,
                seed: everruns_core::SessionSeedMode::Fresh,
            },
        )
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "FCP session create failed");
            internal_error_response()
        })?;
    Ok(ResolvedSession {
        session_id: session.id.uuid(),
    })
}

fn parse_session_cookie(headers: &HeaderMap) -> Option<Uuid> {
    let header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for entry in header.split(';') {
        let Some((name, value)) = entry.split_once('=') else {
            continue;
        };
        if name.trim() == FCP_SESSION_COOKIE {
            return Uuid::parse_str(value.trim()).ok();
        }
    }
    None
}

fn build_session_cookie(session_id: Uuid, expiration_seconds: u32) -> Option<String> {
    let mut parts = vec![
        format!("{FCP_SESSION_COOKIE}={session_id}"),
        "Path=/".to_string(),
        "HttpOnly".to_string(),
        "SameSite=Lax".to_string(),
    ];
    if expiration_seconds > 0 {
        parts.push(format!("Max-Age={expiration_seconds}"));
    }
    Some(parts.join("; "))
}

fn expired_age_seconds(
    created_at: chrono::DateTime<chrono::Utc>,
    max_seconds: u32,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<i64> {
    if max_seconds == 0 {
        return None;
    }
    let age = now.signed_duration_since(created_at).num_seconds().max(0);
    (age > max_seconds as i64).then_some(age)
}

fn fcp_message_metadata(app: &App) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    map.insert(
        "_app_id".to_string(),
        Value::String(app.public_id.to_string()),
    );
    map.insert("source".to_string(), Value::String("fcp".to_string()));
    map
}

fn content_parts_to_text(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_event_data<T: for<'de> Deserialize<'de>>(
    event: &everruns_core::Event,
) -> Result<T, serde_json::Error> {
    serde_json::from_value(serde_json::to_value(&event.data)?)
}

fn extract_token(headers: &HeaderMap) -> Option<&str> {
    if let Some(value) = headers.get(FCP_TOKEN_HEADER) {
        return value.to_str().ok();
    }
    let auth = headers.get(AUTHORIZATION)?.to_str().ok()?;
    auth.strip_prefix("Bearer ")
}

// ----------------------------------------------------------------------------
// Response helpers — every body is Markdown with actionable text.
// ----------------------------------------------------------------------------

fn markdown_response(status: StatusCode, body: impl Into<String>) -> Response {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, FCP_CONTENT_TYPE)],
        body.into(),
    )
        .into_response()
}

fn not_found_response() -> Response {
    // Sanitization: this single body covers "no such app", "app exists but
    // is not published", "app has no FCP channel", and "FCP channel is
    // disabled". The caller cannot distinguish them, so the response cannot
    // be used to probe operator state.
    markdown_response(
        StatusCode::NOT_FOUND,
        "No FCP endpoint exists at this URL.\n\nDouble-check the app id in the URL. If the URL was given to you by\nsomeone else, ask them to confirm it is still correct.",
    )
}

fn unauthorized_response() -> Response {
    markdown_response(
        StatusCode::UNAUTHORIZED,
        "Authentication required.\n\nSend `Authorization: Bearer <token>` or the `X-Everruns-FCP-Token: <token>`\nheader with the credential issued to you by this endpoint's operator. Issue\na `GET` request to this same URL for the handshake — it describes how to\nobtain a token.",
    )
}

fn rate_limited_response(limit: u32) -> Response {
    let body = format!(
        "Rate limit exceeded.\n\nThis endpoint accepts at most {limit} requests per minute from your client.\nWait roughly 60 seconds and try again. Repeated bursts at the limit will\nbe rejected before the agent runs."
    );
    let mut response = markdown_response(StatusCode::TOO_MANY_REQUESTS, body);
    response
        .headers_mut()
        .append("retry-after", HeaderValue::from_static("60"));
    response
}

fn turn_error_response(error: PublicError) -> Response {
    let (status, body) = match error.code {
        PublicErrorCode::RateLimited => (
            StatusCode::TOO_MANY_REQUESTS,
            "The model is rate-limited right now.\n\nWait a moment and retry the same request. The agent did not produce a\nreply, so the conversation state is unchanged.".to_string(),
        ),
        PublicErrorCode::ServiceUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "The agent is temporarily unavailable.\n\nRetry the same request in a few seconds. If this persists, the\nendpoint operator's downstream model or service is offline.".to_string(),
        ),
        PublicErrorCode::RequestTooLarge => (
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Your message was too large for the agent.\n\nThe body limit on this endpoint is {MAX_FCP_BODY_BYTES} bytes; the model has its own\nlower context limit. Send a shorter message."
            ),
        ),
        PublicErrorCode::InternalError => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Something went wrong on this endpoint.\n\nThe agent did not produce a reply. Retry the same request in a moment;\nif it keeps failing, contact the endpoint operator with the approximate\ntime of the request.".to_string(),
        ),
    };
    markdown_response(status, body)
}

fn timeout_response(seconds: u32) -> Response {
    markdown_response(
        StatusCode::GATEWAY_TIMEOUT,
        format!(
            "The agent did not reply within {seconds} seconds.\n\nThis endpoint waits up to {seconds} seconds for a response and then gives up.\nRetry the same request in a moment, or shorten the message — long\nturns are more likely to time out."
        ),
    )
}

fn invalid_body_response() -> Response {
    markdown_response(
        StatusCode::BAD_REQUEST,
        "Request body must be valid UTF-8 text.\n\nIssue a `GET` to this URL for the handshake — it describes what shape of\nbody this endpoint accepts.",
    )
}

fn invalid_json_response() -> Response {
    markdown_response(
        StatusCode::BAD_REQUEST,
        "Your `Content-Type` says `application/json` but the body did not parse.\n\nEither set `Content-Type: text/plain` and send the message as the raw\nbody, or send valid JSON of the form `{\"message\": \"...\"}`. Issue a\n`GET` to this URL for the handshake.",
    )
}

fn empty_message_response() -> Response {
    markdown_response(
        StatusCode::BAD_REQUEST,
        "The message was empty.\n\nPut your message in the body of the `POST`, either as plain text or as\n`{\"message\": \"...\"}` with `Content-Type: application/json`. Issue a\n`GET` to this URL for the handshake.",
    )
}

fn session_expired_response() -> Response {
    markdown_response(
        StatusCode::GONE,
        "Your FCP session has expired.\n\nDrop the `fcp_session` cookie and POST again — the next request will\nstart a fresh conversation and return a new cookie.",
    )
}

fn payload_too_large_response() -> Response {
    markdown_response(
        StatusCode::PAYLOAD_TOO_LARGE,
        format!(
            "Body exceeds the FCP body limit of {kib} KiB ({bytes} bytes).\n\nSend a shorter message. If you need to send long content, summarise it\nor break it into multiple `POST`s in the same session.",
            kib = MAX_FCP_BODY_BYTES / 1024,
            bytes = MAX_FCP_BODY_BYTES,
        ),
    )
}

fn internal_error_response() -> Response {
    markdown_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Something went wrong on this endpoint.\n\nRetry the same request in a moment; if it keeps failing, contact the\nendpoint operator with the approximate time of the request.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    fn test_app(name: &str, description: Option<&str>) -> App {
        let json = serde_json::json!({
            "id": "app_01933b5a000070008000000000000001",
            "name": name,
            "description": description,
            "harness_id": "harness_01933b5a000070008000000000000001",
            "owner_principal_id": "principal_01933b5a000070008000000000000001",
            "status": "published",
            "channels": [],
            "published_at": null,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "archived_at": null,
            "deleted_at": null,
        });
        serde_json::from_value(json).expect("test app")
    }

    fn default_config() -> FcpChannelConfig {
        serde_json::from_value(serde_json::json!({})).unwrap()
    }

    #[test]
    fn handshake_uses_override_when_present() {
        let app = test_app("Echo", None);
        let mut config = default_config();
        config.handshake = Some("custom body".to_string());
        assert_eq!(render_handshake(&app, &config), "custom body");
    }

    #[test]
    fn handshake_includes_app_name_and_description() {
        let app = test_app("Flights", Some("Search and book commercial flights"));
        let rendered = render_handshake(&app, &default_config());
        assert!(rendered.contains("# Flights"));
        assert!(rendered.contains("Search and book commercial flights"));
        assert!(rendered.contains("POST"));
        assert!(rendered.contains("application/json"));
        assert!(rendered.contains("fcp_session"));
    }

    #[test]
    fn handshake_advertises_auth_when_token_configured() {
        let app = test_app("Secret", None);
        let mut config = default_config();
        config.token = Some("YExample0".to_string());
        let rendered = render_handshake(&app, &config);
        assert!(rendered.contains("Authorization: Bearer"));
        assert!(rendered.contains("X-Everruns-FCP-Token"));
        assert!(!rendered.contains("YExample0"), "must never leak the token");
    }

    #[test]
    fn handshake_advertises_auth_when_anonymous_false() {
        let app = test_app("Closed", None);
        let mut config = default_config();
        config.anonymous = false;
        let rendered = render_handshake(&app, &config);
        assert!(rendered.contains("Authorization: Bearer"));
    }

    #[test]
    fn handshake_marks_no_expiration() {
        let app = test_app("Forever", None);
        let mut config = default_config();
        config.session_expiration_seconds = 0;
        let rendered = render_handshake(&app, &config);
        assert!(rendered.contains("do not expire"));
    }

    #[test]
    fn handshake_advertises_rate_limit_when_set() {
        let app = test_app("Slow", None);
        let mut config = default_config();
        config.rate_limit_per_minute = Some(30);
        let rendered = render_handshake(&app, &config);
        assert!(rendered.contains("30 requests per minute"));
    }

    #[test]
    fn handshake_marks_anonymous_endpoint() {
        let app = test_app("Open", None);
        let rendered = render_handshake(&app, &default_config());
        assert!(!rendered.contains("Authorization: Bearer"));
    }

    #[test]
    fn humanize_seconds_formats_common_durations() {
        assert_eq!(humanize_seconds(3600), "after 1 hour");
        assert_eq!(humanize_seconds(2 * 3600), "after 2 hours");
        assert_eq!(humanize_seconds(60), "after 1 minute");
        assert_eq!(humanize_seconds(45), "after 45 seconds");
    }

    #[test]
    fn parse_session_cookie_extracts_named_value() {
        let mut headers = HeaderMap::new();
        let uuid = Uuid::now_v7();
        headers.insert(
            axum::http::header::COOKIE,
            format!("other=foo; fcp_session={uuid}; trailing=bar")
                .parse()
                .unwrap(),
        );
        assert_eq!(parse_session_cookie(&headers), Some(uuid));
    }

    #[test]
    fn parse_session_cookie_returns_none_when_absent() {
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::COOKIE, "other=foo".parse().unwrap());
        assert_eq!(parse_session_cookie(&headers), None);
    }

    #[test]
    fn build_session_cookie_includes_max_age_when_set() {
        let id = Uuid::now_v7();
        let cookie = build_session_cookie(id, 3600).unwrap();
        assert!(cookie.contains(&id.to_string()));
        assert!(cookie.contains("Max-Age=3600"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
    }

    #[test]
    fn build_session_cookie_omits_max_age_when_zero() {
        let id = Uuid::now_v7();
        let cookie = build_session_cookie(id, 0).unwrap();
        assert!(!cookie.contains("Max-Age"));
    }

    #[test]
    fn extract_user_text_plain_body_is_returned_verbatim() {
        let headers = HeaderMap::new();
        let body = b"Hello there";
        let text = extract_user_text(&headers, body).unwrap();
        assert_eq!(text, "Hello there");
    }

    #[test]
    fn extract_user_text_json_body_uses_message_field() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        let body = br#"{"message":"book a flight"}"#;
        let text = extract_user_text(&headers, body).unwrap();
        assert_eq!(text, "book a flight");
    }

    #[test]
    fn extract_user_text_unknown_content_type_with_json_body_is_parsed() {
        let headers = HeaderMap::new();
        let body = br#"{"message":"hi"}"#;
        let text = extract_user_text(&headers, body).unwrap();
        assert_eq!(text, "hi");
    }

    #[test]
    fn extract_user_text_invalid_json_with_json_content_type_returns_bad_request() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        let body = b"not json";
        assert!(extract_user_text(&headers, body).is_err());
    }

    #[test]
    fn content_parts_to_text_joins_text_parts_and_drops_others() {
        let parts = vec![ContentPart::text("hello"), ContentPart::text("world")];
        assert_eq!(content_parts_to_text(&parts), "hello\nworld");
    }

    #[test]
    fn extract_token_prefers_dedicated_header() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer fallback".parse().unwrap());
        headers.insert(FCP_TOKEN_HEADER, "primary".parse().unwrap());
        assert_eq!(extract_token(&headers), Some("primary"));
    }

    #[test]
    fn extract_token_falls_back_to_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer fallback".parse().unwrap());
        assert_eq!(extract_token(&headers), Some("fallback"));
    }

    #[test]
    fn expired_age_seconds_within_window_returns_none() {
        let now = chrono::Utc::now();
        let created = now - ChronoDuration::seconds(100);
        assert_eq!(expired_age_seconds(created, 3600, now), None);
    }

    #[test]
    fn expired_age_seconds_outside_window_returns_age() {
        let now = chrono::Utc::now();
        let created = now - ChronoDuration::seconds(7200);
        assert_eq!(expired_age_seconds(created, 3600, now), Some(7200));
    }

    #[test]
    fn expired_age_seconds_zero_disables_expiration() {
        let now = chrono::Utc::now();
        let created = now - ChronoDuration::seconds(100_000);
        assert_eq!(expired_age_seconds(created, 0, now), None);
    }

    #[test]
    fn check_token_anonymous_allows_no_header() {
        let headers = HeaderMap::new();
        assert!(check_token(&headers, &default_config()).is_ok());
    }

    #[test]
    fn check_token_required_rejects_missing() {
        let headers = HeaderMap::new();
        let mut config = default_config();
        config.token = Some("YExample0".to_string());
        assert!(check_token(&headers, &config).is_err());
    }

    #[test]
    fn check_token_required_accepts_matching_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer YExample0".parse().unwrap());
        let mut config = default_config();
        config.token = Some("YExample0".to_string());
        assert!(check_token(&headers, &config).is_ok());
    }

    #[test]
    fn check_token_required_rejects_mismatch() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer wrong".parse().unwrap());
        let mut config = default_config();
        config.token = Some("YExample0".to_string());
        assert!(check_token(&headers, &config).is_err());
    }

    #[test]
    fn check_token_anonymous_false_without_token_rejects() {
        // Misconfigured: operator disabled anonymous but did not configure a
        // token. We refuse the call rather than silently letting it through.
        let headers = HeaderMap::new();
        let mut config = default_config();
        config.anonymous = false;
        assert!(check_token(&headers, &config).is_err());
    }

    fn body_text(response: Response) -> (StatusCode, String) {
        let (parts, body) = response.into_parts();
        let bytes = futures::executor::block_on(async {
            axum::body::to_bytes(body, usize::MAX).await.unwrap()
        });
        (parts.status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[test]
    fn unauthorized_response_body_is_actionable() {
        let (status, body) = body_text(unauthorized_response());
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("Authorization: Bearer"));
        assert!(body.contains("X-Everruns-FCP-Token"));
        assert!(
            body.contains("GET"),
            "must direct caller back at the handshake"
        );
    }

    #[test]
    fn not_found_response_does_not_leak_app_state() {
        let (status, body) = body_text(not_found_response());
        assert_eq!(status, StatusCode::NOT_FOUND);
        let lower = body.to_lowercase();
        // None of these internal states must be readable from a 404 body:
        for forbidden in [
            "draft",
            "archived",
            "unpublished",
            "disabled",
            "permission",
            "auth",
            "channel",
            "config",
        ] {
            assert!(
                !lower.contains(forbidden),
                "leaked internal state via word '{forbidden}': {body}"
            );
        }
    }

    #[test]
    fn rate_limited_response_includes_retry_after_and_limit() {
        let response = rate_limited_response(42);
        let retry_after = response
            .headers()
            .get("retry-after")
            .cloned()
            .expect("retry-after header");
        let (status, body) = body_text(response);
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(retry_after.to_str().unwrap(), "60");
        assert!(body.contains("42"));
    }

    #[test]
    fn turn_error_response_body_is_sanitized() {
        let response = turn_error_response(PublicError::fallback());
        let (status, body) = body_text(response);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let lower = body.to_lowercase();
        // Internal vocabulary must never reach the public body.
        for forbidden in [
            "stack",
            "panic",
            "subscription",
            "openai",
            "anthropic",
            "tokio",
            "postgres",
        ] {
            assert!(
                !lower.contains(forbidden),
                "leaked internal detail via word '{forbidden}': {body}"
            );
        }
    }

    #[test]
    fn timeout_response_mentions_configured_seconds() {
        let (status, body) = body_text(timeout_response(45));
        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert!(body.contains("45 seconds"));
    }

    #[test]
    fn session_expired_response_tells_client_to_drop_cookie() {
        let (status, body) = body_text(session_expired_response());
        assert_eq!(status, StatusCode::GONE);
        assert!(body.contains("fcp_session"));
        assert!(body.to_lowercase().contains("drop"));
    }
}
