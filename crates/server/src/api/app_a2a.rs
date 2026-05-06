// App A2A (Agent2Agent) ingress — JSON-RPC + API key authenticated invocation.
//
// Design Decision: A2A channels are app-scoped and channel-scoped
// (`POST /v1/apps/{app_id}/a2a/{channel_id}`) so a single app can expose
// multiple agent-to-agent endpoints with independent keys, agent cards, and
// session routing.
//
// Only `message/send` is implemented in this iteration. Streaming, tasks/get,
// and push notifications return JSON-RPC `-32601 Method not found`. See
// `specs/a2a-channel.md`.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::common::ErrorResponse;
use crate::domains::apps::{
    A2aInvocationRequest, hash_a2a_api_key, invoke_a2a_app_channel, queries as app_queries,
};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::middleware::RequestId;
use crate::storage::{EncryptionService, StorageBackend};

const A2A_PROTOCOL_VERSION: &str = "0.3.0";
const A2A_AGENT_VERSION: &str = "0.1";

// THREAT[TM-A2A-005]: Method gating — only `message/send` is supported.
// Allowing arbitrary A2A methods would expose code paths we have not
// audited for prompt injection or task-state forgery.
const METHOD_MESSAGE_SEND: &str = "message/send";

#[derive(Clone)]
pub struct AppA2aState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
}

impl AppA2aState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Arc<dyn everruns_worker::AgentRunner>,
        notifications_enabled: bool,
        event_delivery: crate::event_delivery::EventDelivery,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(
                db.clone(),
                runner,
                notifications_enabled,
                event_delivery,
            )),
            db,
            encryption,
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
        (status = 200, description = "JSON-RPC 2.0 response (success or A2A error envelope)"),
        (status = 401, description = "Missing or invalid API key", body = ErrorResponse),
        (status = 403, description = "App is not published or channel disabled", body = ErrorResponse),
        (status = 404, description = "App or channel not found", body = ErrorResponse),
    ),
    tag = "apps"
)]
pub async fn invoke_a2a(
    State(state): State<AppA2aState>,
    Path((app_id, channel_id)): Path<(String, String)>,
    req_id: Option<axum::Extension<RequestId>>,
    headers: HeaderMap,
    Json(envelope): Json<Value>,
) -> Result<(StatusCode, Json<JsonRpcResponse>), (StatusCode, Json<ErrorResponse>)> {
    // Parse JSON-RPC envelope first so we can return structured errors with the
    // original `id` echoed back.
    let parsed: JsonRpcRequest = match serde_json::from_value(envelope) {
        Ok(parsed) => parsed,
        Err(err) => {
            return Ok((
                StatusCode::BAD_REQUEST,
                rpc_error(Value::Null, -32600, format!("Invalid Request: {err}")),
            ));
        }
    };
    let rpc_id = parsed.id.clone().unwrap_or(Value::Null);

    // Resolve app + channel.
    let app = app_queries::get_by_public_id_unscoped(&state.db, state.encryption.as_ref(), &app_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;

    if app.status != everruns_core::AppStatus::Published {
        return Err(forbidden("App is not published"));
    }

    let channel_id_typed = channel_id
        .parse::<everruns_core::typed_id::AppChannelId>()
        .map_err(|e| bad_request(format!("Invalid channel ID: {e}")))?;
    let channel = app.channel_by_id(&channel_id_typed).ok_or_else(not_found)?;
    if channel.channel_type != everruns_core::ChannelType::A2a {
        return Err(not_found());
    }
    // THREAT[TM-AUTHZ-006]: Anonymous A2A ingress must never reach draft or
    // disabled app channels, and every request must present the per-channel
    // API key before session creation.
    if !channel.enabled {
        return Err(forbidden("A2A channel is disabled"));
    }

    let config = channel
        .a2a_config()
        .ok_or_else(|| bad_request("Invalid A2A channel configuration"))?;

    // THREAT[TM-A2A-001]: API keys are stored as SHA-256 hashes; plaintext is
    // never persisted. We hash the inbound key and compare against the stored
    // hash before doing anything else.
    let provided_key = extract_a2a_api_key(&headers).ok_or_else(unauthorized)?;
    let provided_hash = hash_a2a_api_key(&provided_key);
    if !constant_time_eq(provided_hash.as_bytes(), config.api_key_hash.as_bytes()) {
        return Err(unauthorized());
    }

    // Method gate.
    if parsed.method != METHOD_MESSAGE_SEND {
        return Ok((
            StatusCode::OK,
            rpc_error(
                rpc_id,
                -32601,
                format!(
                    "Method not found: {} (only message/send is supported)",
                    parsed.method
                ),
            ),
        ));
    }

    // Pull text parts. A2A `parts` is an array of objects; we surface
    // `text` parts joined by newline. Anything else is silently ignored at the
    // template level but if there is no text at all we reject.
    let message = parsed.params.get("message").cloned().unwrap_or(Value::Null);
    let parts = message
        .get("parts")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let text = parts
        .iter()
        .filter_map(|part| {
            // Spec uses `kind: "text"`; older drafts used `type: "text"`. Accept both.
            let kind = part.get("kind").or_else(|| part.get("type"));
            if kind.and_then(Value::as_str) == Some("text") {
                part.get("text").and_then(Value::as_str).map(str::to_owned)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        return Ok((
            StatusCode::OK,
            rpc_error(
                rpc_id,
                -32602,
                "Invalid params: message.parts must contain at least one non-empty text part",
            ),
        ));
    }

    let role = message
        .get("role")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let message_id = message
        .get("messageId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let context_id = message
        .get("contextId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let task_id = Uuid::now_v7().to_string();
    let request_id = req_id.map(|axum::Extension(id)| id.0);

    let result = invoke_a2a_app_channel(
        &state.db,
        state.encryption.as_ref(),
        &state.session_service,
        &state.message_service,
        A2aInvocationRequest {
            app_id,
            channel_id,
            params: parsed.params,
            text,
            message_id: message_id.clone(),
            task_id: task_id.clone(),
            context_id: context_id.clone(),
            role,
        },
        request_id,
    )
    .await
    .map_err(|err| match err {
        crate::domains::common::CommandError::BadRequest(msg) => bad_request(msg),
        crate::domains::common::CommandError::Forbidden(msg) => forbidden(msg),
        crate::domains::common::CommandError::NotFound(_) => not_found(),
        crate::domains::common::CommandError::Conflict(msg) => {
            (StatusCode::CONFLICT, Json(ErrorResponse { error: msg }))
        }
        crate::domains::common::CommandError::Unprocessable(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorResponse { error: msg }),
        ),
        crate::domains::common::CommandError::Internal(error) => internal_error(error),
    })?;

    let session_id_string = result.session_id.to_string();
    let task = json!({
        "id": task_id,
        "contextId": session_id_string,
        "status": { "state": "completed" },
        "kind": "task",
    });

    Ok((StatusCode::OK, rpc_success(rpc_id, task)))
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
    Path((app_id, channel_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let app = app_queries::get_by_public_id_unscoped(&state.db, state.encryption.as_ref(), &app_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    // Agent Card is only published when the app is live and the channel is on.
    if app.status != everruns_core::AppStatus::Published {
        return Err(not_found());
    }
    let channel_id_typed = channel_id
        .parse::<everruns_core::typed_id::AppChannelId>()
        .map_err(|_| not_found())?;
    let channel = app.channel_by_id(&channel_id_typed).ok_or_else(not_found)?;
    if channel.channel_type != everruns_core::ChannelType::A2a || !channel.enabled {
        return Err(not_found());
    }
    let config = channel.a2a_config().ok_or_else(not_found)?;

    // Build absolute endpoint URL using the inbound Host header. Agents
    // discovering the card need an absolute URL; if Host is missing we fall
    // back to a relative path.
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("https");
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok());
    let endpoint = match host {
        Some(host) => format!(
            "{scheme}://{host}/v1/apps/{}/a2a/{}",
            app.public_id, channel.public_id
        ),
        None => format!("/v1/apps/{}/a2a/{}", app.public_id, channel.public_id),
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

    let card = json!({
        "name": name,
        "description": description,
        "url": endpoint,
        "protocolVersion": A2A_PROTOCOL_VERSION,
        "version": A2A_AGENT_VERSION,
        "preferredTransport": "JSONRPC",
        "capabilities": {
            "streaming": false,
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
        "securitySchemes": {
            "apiKey": { "type": "http", "scheme": "bearer" }
        },
        "security": [{ "apiKey": [] }],
    });
    Ok(Json(card))
}

fn extract_a2a_api_key(headers: &HeaderMap) -> Option<String> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    auth.strip_prefix("Bearer ").map(ToOwned::to_owned)
}

// THREAT[TM-A2A-002]: Constant-time comparison of the SHA-256 hex digests
// avoids leaking partial-match timing information to a remote attacker.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn bad_request(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
}

fn forbidden(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
}

fn unauthorized() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: "Invalid or missing A2A API key".to_string(),
        }),
    )
}

fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "App channel not found".to_string(),
        }),
    )
}

fn internal_error(error: anyhow::Error) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %error, "Failed to invoke app A2A");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal server error".to_string(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches() {
        assert!(constant_time_eq(b"abcd", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abce"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

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
}
