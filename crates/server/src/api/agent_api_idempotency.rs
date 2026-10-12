// Safe retries on the Agent Execution API: `Idempotency-Key` on
// `POST …/sessions` and `POST …/sessions/{id}/messages`.
//
// Decisions (see knowledge/integrations/agent-execution-api.md):
// - Same store and contract as the command API (`command_dispatch`): a key is
//   remembered for a day, a retry with the same request replays the first
//   response with `Idempotent-Replayed: true`, a different request under the
//   same key is 422, and a retry while the first is still running is 409.
// - Keys belong to the caller (`ApiCaller::idempotency_owner`), so two
//   applications or two end users never collide on one key.
// - Only a success is remembered. A failure releases the key, so the client
//   can fix the request and retry with the same key.

use axum::{
    Json,
    body::{Bytes, to_bytes},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::channel_api::ChannelApiState;
use super::command_dispatch::{
    IDEMPOTENCY_KEY_TTL, IDEMPOTENCY_LOCK_TTL, IDEMPOTENT_REPLAYED_HEADER, idempotency_key,
    open_replay, request_fingerprint, seal_replay,
};
use crate::domains::agent_channels::api_sessions::ApiCaller;
use crate::domains::common::CommandError;
use crate::storage::command_idempotency::{
    ClaimIdempotencyKey, IdempotencyClaim, IdempotencyKeyScope,
};

/// Largest response remembered for replay.
const MAX_REPLAY_BYTES: usize = 1024 * 1024;

/// A success as stored for replay.
#[derive(Serialize, Deserialize)]
struct StoredReply {
    status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    location: Option<String>,
    body: Value,
}

/// One retry-safe request: what it does (`operation`), to what (`target`),
/// and its body.
pub(crate) struct Retryable<'a> {
    pub org_id: i64,
    pub caller: &'a ApiCaller,
    pub operation: &'static str,
    pub target: String,
    pub body: &'a Bytes,
}

/// Run `handle` once per `Idempotency-Key`, or right away when the request
/// sent none.
pub(crate) async fn run_once(
    state: &ChannelApiState,
    headers: &HeaderMap,
    request: Retryable<'_>,
    handle: impl Future<Output = Response>,
) -> Response {
    let key = match idempotency_key(headers) {
        Ok(Some(key)) => key,
        Ok(None) => return handle.await,
        Err(err) => return failure(err),
    };
    let scope = IdempotencyKeyScope {
        org_id: request.org_id,
        principal_id: request.caller.idempotency_owner(),
        key,
    };
    let body: Value = serde_json::from_slice(request.body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(request.body).into_owned()));
    let fingerprint = request_fingerprint(
        request.operation,
        &serde_json::json!({ "target": request.target, "body": body }),
    );
    let now = chrono::Utc::now();
    let claim = state
        .db
        .claim_command_idempotency_key(&ClaimIdempotencyKey {
            scope: scope.clone(),
            command: request.operation.to_string(),
            fingerprint: fingerprint.clone(),
            locked_until: now + IDEMPOTENCY_LOCK_TTL,
            expires_at: now + IDEMPOTENCY_KEY_TTL,
        })
        .await;
    let stored = match claim {
        Ok(IdempotencyClaim::Claimed) => None,
        Ok(IdempotencyClaim::Existing(stored)) => Some(stored),
        Err(err) => return failure(CommandError::internal(err)),
    };
    if let Some(stored) = stored {
        if stored.fingerprint != fingerprint {
            return failure(
                CommandError::unprocessable(
                    "Idempotency-Key was already used for a different request; use a new key for a new request",
                )
                .with_code("idempotency_key_reused"),
            );
        }
        return match stored.response {
            Some(sealed) => match open_replay::<StoredReply>(state.encryption.as_ref(), &sealed) {
                Ok(reply) => replay(reply),
                Err(err) => failure(err),
            },
            None => failure(
                CommandError::conflict(
                    "A request with this Idempotency-Key is still running; retry later",
                )
                .with_code("idempotency_key_in_progress"),
            ),
        };
    }

    let response = handle.await;
    if !response.status().is_success() {
        release(state, &scope).await;
        return response;
    }
    let (parts, body) = response.into_parts();
    let bytes = match to_bytes(body, MAX_REPLAY_BYTES).await {
        Ok(bytes) => bytes,
        Err(err) => {
            release(state, &scope).await;
            return failure(CommandError::internal(anyhow::anyhow!(
                "agent API response too large to remember: {err}"
            )));
        }
    };
    let reply = StoredReply {
        status: parts.status.as_u16(),
        location: parts
            .headers
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    };
    // The work already happened: failing to remember it must not turn its
    // success into an error.
    let remembered = match seal_replay(state.encryption.as_ref(), &reply) {
        Ok(sealed) => {
            state
                .db
                .complete_command_idempotency_key(&scope, &sealed)
                .await
        }
        Err(err) => Err(err),
    };
    if let Err(err) = remembered {
        tracing::warn!(error = %err, "failed to store idempotent agent API response");
    }
    Response::from_parts(parts, axum::body::Body::from(bytes))
}

fn replay(reply: StoredReply) -> Response {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::OK);
    let mut response = (status, Json(reply.body)).into_response();
    let headers = response.headers_mut();
    headers.insert(IDEMPOTENT_REPLAYED_HEADER, HeaderValue::from_static("true"));
    if let Some(location) = reply
        .location
        .and_then(|value| HeaderValue::from_str(&value).ok())
    {
        headers.insert(header::LOCATION, location);
    }
    response
}

async fn release(state: &ChannelApiState, scope: &IdempotencyKeyScope) {
    if let Err(err) = state.db.release_command_idempotency_key(scope).await {
        tracing::warn!(error = %err, "failed to release idempotency key");
    }
}

fn failure(err: CommandError) -> Response {
    <(StatusCode, Json<_>)>::from(err).into_response()
}
