// GitHub App webhook ingress: `POST /v1/github/apps/{app_row_id}/webhook`.
//
// Every per-agent GitHub App (see `crate::github_apps`) is created with this
// URL, carrying its own row id. GitHub signs each delivery with the App's
// webhook secret (`X-Hub-Signature-256`); a verified delivery fans out to the
// GitHub triggers of the agents that share the App's identity
// (`domains::agent_triggers::github`).
//
// THREAT[TM-GHAPP-006]: the route is public and unauthenticated by our auth.
// Signature verification is the only gate, so it runs before any parsing, uses
// a constant-time comparison, and fails closed when the App has no secret.
// THREAT[TM-GHAPP-007]: dedupe on `X-GitHub-Delivery` in the shared pipeline
// makes a replayed (still validly signed) delivery a recorded duplicate.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use hmac::{Hmac, KeyInit, Mac};
use serde::Serialize;
use sha2::Sha256;
use uuid::Uuid;

use super::channel_webhooks::ChannelWebhookState;
use super::common::ErrorResponse;
use crate::domains::agent_triggers::events::TriggerEventOutcome;
use crate::domains::agent_triggers::github::{GitHubDelivery, dispatch_github_delivery};
use crate::middleware::RequestId;
use crate::security::constant_time_eq;

const MAX_DELIVERY_ID_LEN: usize = 128;
const MAX_EVENT_NAME_LEN: usize = 64;

pub fn routes(state: ChannelWebhookState) -> Router {
    Router::new()
        .route("/v1/github/apps/{app_row_id}/webhook", post(receive))
        .with_state(state)
}

/// What a delivery did, per outcome.
#[derive(Debug, Default, Serialize, PartialEq)]
pub struct GitHubWebhookResponse {
    pub dispatched: usize,
    pub filtered: usize,
    pub duplicate: usize,
    pub failed: usize,
}

type ApiError = (StatusCode, Json<ErrorResponse>);

fn error(status: StatusCode, message: &str) -> ApiError {
    ErrorResponse::new(message.to_string()).into_response(status)
}

/// Verify `sha256=<hex>` over the raw body.
fn signature_valid(secret: &[u8], body: &[u8], header: Option<&str>) -> bool {
    let Some(presented) = header
        .and_then(|value| value.strip_prefix("sha256="))
        .and_then(|hex_sig| hex::decode(hex_sig).ok())
    else {
        return false;
    };
    if secret.is_empty() {
        return false;
    }
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        return false;
    };
    mac.update(body);
    constant_time_eq(&mac.finalize().into_bytes(), &presented)
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

async fn receive(
    State(state): State<ChannelWebhookState>,
    Path(app_row_id): Path<Uuid>,
    req_id: Option<axum::Extension<RequestId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<GitHubWebhookResponse>), ApiError> {
    let not_found = || error(StatusCode::NOT_FOUND, "Unknown GitHub App");
    let app = state
        .db
        .get_github_app_unscoped(app_row_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to load GitHub App");
            error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        })?
        .ok_or_else(not_found)?;

    let secret = app
        .webhook_secret_encrypted
        .as_deref()
        .zip(state.encryption.as_deref())
        .and_then(|(sealed, encryption)| encryption.decrypt(sealed).ok());
    let verified = secret.as_deref().is_some_and(|secret| {
        signature_valid(secret, &body, header(&headers, "x-hub-signature-256"))
    });
    if !verified {
        return Err(error(StatusCode::UNAUTHORIZED, "Invalid signature"));
    }

    let event = header(&headers, "x-github-event")
        .filter(|event| !event.is_empty() && event.len() <= MAX_EVENT_NAME_LEN)
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "Missing X-GitHub-Event"))?
        .to_string();
    let delivery_id = header(&headers, "x-github-delivery")
        .filter(|id| !id.is_empty() && id.len() <= MAX_DELIVERY_ID_LEN)
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "Missing X-GitHub-Delivery"))?
        .to_string();
    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Body is not JSON"))?;

    match event.as_str() {
        "ping" => return Ok((StatusCode::OK, Json(GitHubWebhookResponse::default()))),
        "installation" if payload.get("action").and_then(|a| a.as_str()) == Some("deleted") => {
            forget_installation(&state, &app, &payload).await;
            return Ok((StatusCode::ACCEPTED, Json(GitHubWebhookResponse::default())));
        }
        _ => {}
    }

    let results = dispatch_github_delivery(
        &state.db,
        &state.session_service,
        &state.message_service,
        &app,
        &GitHubDelivery {
            event,
            delivery_id,
            payload,
        },
        req_id.map(|axum::Extension(id)| id.0),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "GitHub delivery routing failed");
        error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
    })?;

    let mut response = GitHubWebhookResponse::default();
    for result in results {
        match result.outcome {
            Ok(TriggerEventOutcome::Dispatched(_)) => response.dispatched += 1,
            Ok(TriggerEventOutcome::Filtered) => response.filtered += 1,
            Ok(TriggerEventOutcome::Duplicate) => response.duplicate += 1,
            Err(_) => response.failed += 1,
        }
    }
    Ok((StatusCode::ACCEPTED, Json(response)))
}

/// The user uninstalled the App on GitHub: drop the identity's connection so
/// nothing keeps minting tokens for an installation that is gone.
async fn forget_installation(
    state: &ChannelWebhookState,
    app: &crate::storage::github_app_rows::GitHubAppRow,
    payload: &serde_json::Value,
) {
    let installation_id = payload.pointer("/installation/id").and_then(|v| v.as_i64());
    let Ok(Some(connection)) = state
        .db
        .get_virtual_user_connection(app.virtual_user_id, "github")
        .await
    else {
        return;
    };
    if connection.installation_id.is_some()
        && connection.installation_id == installation_id
        && let Err(e) = state
            .db
            .delete_virtual_user_connection(app.virtual_user_id, "github")
            .await
    {
        tracing::warn!(error = %e, "Failed to drop uninstalled GitHub connection");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &[u8], body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn signature_must_match_body_and_secret() {
        let body = br#"{"action":"opened"}"#;
        let good = sign(b"s3cret", body);
        assert!(signature_valid(b"s3cret", body, Some(&good)));
        assert!(!signature_valid(b"other", body, Some(&good)));
        assert!(!signature_valid(b"s3cret", b"{}", Some(&good)));
        assert!(!signature_valid(b"s3cret", body, None));
        assert!(!signature_valid(b"s3cret", body, Some("sha1=abc")));
        assert!(!signature_valid(b"s3cret", body, Some("sha256=zz")));
        // An empty secret would key an HMAC anyone can compute.
        assert!(!signature_valid(b"", body, Some(&sign(b"", body))));
    }
}
