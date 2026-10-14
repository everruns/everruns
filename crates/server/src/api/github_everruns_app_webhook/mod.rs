// The shared Everruns GitHub App's webhook: `POST /v1/github/everruns-app/webhook`.
//
// The deployment's shared App (`GITHUB_APP_*`) is configured with this URL and
// `GITHUB_APP_WEBHOOK_SECRET`. A verified delivery names its installation;
// the org that links that installation (D3 in
// knowledge/integrations/github-model.md) is found by it, and the delivery
// fans out to the GitHub triggers of that org's agents that chose it.
//
// THREAT[TM-GITHUB-005]: the route is public and unauthenticated by our auth.
// The HMAC over the raw body is the only gate, so it runs first, compares in
// constant time, and fails closed when no secret is configured. Routing uses
// only the signed payload's `installation.id`, and an installation reaches
// only the one org whose admin linked it (a unique index enforces one org).
// THREAT[TM-GHAPP-007]: replays are recorded duplicates through the shared
// pipeline's `X-GitHub-Delivery` dedupe, as for agent Apps.
//
// Decision: a delivery for an installation no org links is accepted and
// dropped (202), so GitHub does not retry or disable the hook for
// installations people made but never linked.

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};

use super::channel_webhooks::ChannelWebhookState;
use super::github_webhooks::{
    ApiError, GitHubWebhookResponse, MAX_DELIVERY_ID_LEN, MAX_EVENT_NAME_LEN, error, header,
    signature_valid, tally,
};
use crate::auth::oauth::format_installation_permissions;
use crate::domains::agent_triggers::github::{
    GitHubDelivery, GitHubDeliveryRecipients, dispatch_github_delivery,
};
use crate::middleware::RequestId;

/// What the route needs of the deployment's shared App.
#[derive(Debug, Clone)]
pub struct EverrunsAppWebhook {
    /// `GITHUB_APP_WEBHOOK_SECRET`; without it every delivery is refused.
    pub secret: Option<String>,
    /// `GITHUB_APP_SLUG`, for dropping the bot's own events.
    pub app_slug: String,
}

impl From<&crate::auth::config::GitHubConnectionConfig> for EverrunsAppWebhook {
    fn from(config: &crate::auth::config::GitHubConnectionConfig) -> Self {
        Self {
            secret: config.webhook_secret.clone(),
            app_slug: config.app_slug.clone(),
        }
    }
}

fn accepted() -> (StatusCode, Json<GitHubWebhookResponse>) {
    (StatusCode::ACCEPTED, Json(GitHubWebhookResponse::default()))
}

pub(super) async fn receive(
    State(state): State<ChannelWebhookState>,
    req_id: Option<axum::Extension<RequestId>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<GitHubWebhookResponse>), ApiError> {
    let app = state.everruns_app.as_ref();
    let verified = app
        .and_then(|app| app.secret.as_deref())
        .is_some_and(|secret| {
            signature_valid(
                secret.as_bytes(),
                &body,
                header(&headers, "x-hub-signature-256"),
            )
        });
    if !verified {
        return Err(error(StatusCode::UNAUTHORIZED, "Invalid signature"));
    }
    let app_slug = app.map(|app| app.app_slug.as_str());

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
    if event == "ping" {
        return Ok((StatusCode::OK, Json(GitHubWebhookResponse::default())));
    }

    let Some(installation_id) = payload.pointer("/installation/id").and_then(|v| v.as_i64()) else {
        return Ok(accepted());
    };
    let internal = |e: anyhow::Error| {
        tracing::error!(error = %e, "Shared GitHub App delivery failed");
        error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
    };
    let Some(link) = state
        .db
        .find_everruns_app_link(installation_id)
        .await
        .map_err(internal)?
    else {
        return Ok(accepted());
    };

    let action = payload.get("action").and_then(|a| a.as_str());
    match (event.as_str(), action) {
        // Uninstalled on GitHub: forget the link and every agent's choice of
        // it, so nothing keeps minting for an installation that is gone.
        ("installation", Some("deleted")) => {
            let removed = state
                .db
                .unlink_everruns_app(link.org_id, link.row.id)
                .await
                .map_err(internal)?;
            tracing::info!(
                org_id = link.org_id,
                installation_id,
                agents = removed.unwrap_or(0),
                "Shared GitHub App uninstalled; org link removed"
            );
            return Ok(accepted());
        }
        ("installation", Some("new_permissions_accepted")) => {
            if let Some(permissions) = payload.pointer("/installation/permissions") {
                let scopes = format_installation_permissions(permissions);
                state
                    .db
                    .update_everruns_app_scopes(
                        link.org_id,
                        installation_id,
                        Some(scopes.as_str()).filter(|s| !s.is_empty()),
                    )
                    .await
                    .map_err(internal)?;
            }
            return Ok(accepted());
        }
        ("installation", _) => return Ok(accepted()),
        _ => {}
    }

    let recipients =
        GitHubDeliveryRecipients::everruns_app(&state.db, link.org_id, installation_id, app_slug)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Shared GitHub App routing failed");
                error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
            })?;
    let results = dispatch_github_delivery(
        &state.db,
        &state.session_service,
        &state.message_service,
        &recipients,
        &GitHubDelivery {
            event,
            delivery_id,
            payload,
        },
        req_id.map(|axum::Extension(id)| id.0),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Shared GitHub App delivery routing failed");
        error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
    })?;
    Ok((StatusCode::ACCEPTED, Json(tally(results))))
}

#[cfg(test)]
mod tests;
