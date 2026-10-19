//! Browser-bound, single-use consent for the deployment's manager app.

use super::install::{
    CallbackQuery, SlackInstallState, provisioning_error_response, require_org_admin,
};
use crate::api::common::ErrorResponse;
use crate::auth::ResolvedOrg;
use crate::channels::slack::provisioning::MANAGER_CONNECT_STATE_TTL;
use crate::domains::agent_channels::record::slack_provisioning::SlackProvisioningError;
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::Engine;
use serde::Serialize;

#[derive(Serialize)]
pub struct BeginManagerConnectResponse {
    /// Send the admin here: Slack's consent screen for the manager app.
    pub authorize_url: String,
}

/// POST /v1/slack/workspaces/oauth — admin only. Starts connecting a workspace
/// by Slack consent through this deployment's manager app.
pub(super) async fn begin_manager_connect(
    org: ResolvedOrg,
    State(state): State<SlackInstallState>,
    jar: CookieJar,
) -> Result<(CookieJar, Json<BeginManagerConnectResponse>), (StatusCode, Json<ErrorResponse>)> {
    require_org_admin(&org)?;
    let manager = state
        .connection_manager
        .filter(|manager| manager.manager_app_configured())
        .ok_or_else(|| {
            ErrorResponse::new("Connecting Slack by consent is not configured on this deployment")
                .into_response(StatusCode::NOT_IMPLEMENTED)
        })?;
    let browser_token =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let (authorize_url, transaction_id) = manager
        .manager_authorize_url(
            org.org_id,
            &manager_redirect_uri(&state.slack.api_base_url),
            &browser_token,
        )
        .await
        .map_err(provisioning_error_response)?;
    Ok((
        jar.add(manager_connect_cookie(transaction_id, browser_token)),
        Json(BeginManagerConnectResponse { authorize_url }),
    ))
}

// One cookie per transaction preserves simultaneous tabs and organizations.
// __Host- prevents sibling subdomains from injecting a parent-domain binding;
// Lax permits Slack's top-level GET redirect while keeping cross-site POSTs out.
fn manager_connect_cookie_name(transaction_id: uuid::Uuid) -> String {
    format!("__Host-slack_manager_{}", transaction_id.simple())
}

fn manager_connect_cookie(transaction_id: uuid::Uuid, token: String) -> Cookie<'static> {
    Cookie::build((manager_connect_cookie_name(transaction_id), token))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::seconds(
            MANAGER_CONNECT_STATE_TTL.num_seconds(),
        ))
        .build()
}

/// Where Slack returns the admin after the manager app consent. Register it as
/// the manager app's redirect URL.
pub(super) fn manager_redirect_uri(api_base_url: &str) -> String {
    format!(
        "{}/v1/slack/workspaces/oauth/callback",
        api_base_url.trim_end_matches('/')
    )
}

/// GET /v1/slack/workspaces/oauth/callback
///
/// THREAT[TM-SLACK-014]: reached by Slack's browser redirect, so it carries
/// none of our auth. Validate the sealed state and its initiating-browser
/// cookie, then atomically consume the ledger entry before resolving the org,
/// reflecting a provider error, or exchanging a code. Invalid callbacks expose
/// neither organization information nor provider errors.
pub(super) async fn finish_manager_connect(
    State(state): State<SlackInstallState>,
    Query(query): Query<CallbackQuery>,
    jar: CookieJar,
) -> Response {
    let settings = format!("{}/settings/slack", state.ui_base_url.trim_end_matches('/'));
    let failed = |jar: CookieJar, code: Option<&str>| {
        let mut url = format!("{settings}?slack_connect=failed");
        if let Some(code) = code.and_then(slack_error_code) {
            url.push_str(&format!("&slack_error={code}"));
        }
        (jar, Redirect::to(&url)).into_response()
    };
    let Some(manager) = state
        .connection_manager
        .as_ref()
        .filter(|manager| manager.manager_app_configured())
    else {
        return failed(jar, None);
    };
    let Some(pending) = query
        .state
        .as_deref()
        .and_then(|sealed| manager.open_manager_state(sealed))
    else {
        tracing::warn!("Slack manager connect callback rejected: invalid state");
        return failed(jar, None);
    };
    let cookie_name = manager_connect_cookie_name(pending.transaction_id);
    let Some(cookie) = jar.get(&cookie_name) else {
        return failed(jar, None);
    };
    match manager
        .consume_manager_state(&pending, cookie.value())
        .await
    {
        Ok(true) => {}
        Ok(false) => return failed(jar, None),
        Err(error) => {
            tracing::warn!(%error, "Slack manager connect transaction unavailable");
            return failed(jar, None);
        }
    }
    let org_id = pending.org_id;
    let jar = jar.remove(manager_connect_cookie(
        pending.transaction_id,
        String::new(),
    ));
    if let Some(error) = query.error.as_deref() {
        // `access_denied` when the admin cancels the consent screen.
        return failed(jar, Some(error));
    }
    let Some(code) = query.code.as_deref().filter(|code| !code.is_empty()) else {
        return failed(jar, None);
    };
    match manager
        .connect_with_manager_code(
            org_id,
            code,
            &manager_redirect_uri(&state.slack.api_base_url),
        )
        .await
    {
        Ok(_) => (jar, Redirect::to(&format!("{settings}?slack_connect=ok"))).into_response(),
        Err(SlackProvisioningError::Rejected(code)) => {
            tracing::warn!(org_id, slack_error = %code, "Slack manager connect refused");
            failed(jar, Some(&code))
        }
        Err(error) => {
            tracing::warn!(org_id, %error, "Slack manager connect failed");
            failed(jar, None)
        }
    }
}

/// A Slack error code safe to put in a URL: Slack's codes are short
/// `snake_case` words, and anything else is dropped rather than reflected.
pub(super) fn slack_error_code(code: &str) -> Option<&str> {
    (!code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'))
    .then_some(code)
}
