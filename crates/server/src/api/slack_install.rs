//! One-click Slack install: create the endpoint's app, then finish its OAuth.
//!
//! Replaces three of the four values an operator used to copy out of
//! api.slack.com. `apps.manifest.create` returns the signing secret and the
//! client pair; the OAuth install returns the bot token and workspace id; the
//! channel id was already optional. See
//! `knowledge/integrations/slack-one-click-install.md` for the live PoC that
//! established this, and `everruns_platform::slack_provisioning` for why app
//! creation is a seam rather than something the OSS server does itself.
//!
//! Two routes with deliberately different auth:
//!
//! * `POST /v1/e/{channel_id}/slack/install` is authenticated. It spends a
//!   connected workspace's app configuration token and creates a real app in
//!   that workspace, so an unauthenticated caller could exhaust the
//!   organization's credential and fill its workspace with orphans.
//! * `GET /v1/e/{channel_id}/slack/oauth/callback` cannot be: Slack redirects
//!   the operator's browser to it and carries none of our auth. It is
//!   protected by the single-use `install_state` nonce instead.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
};
use everruns_platform::slack_provisioning::{
    ProvisionedSlackApp, SlackAppProvisioner, SlackProvisioningConnectionStatus,
    SlackProvisioningError, UnavailableSlackAppProvisioner,
};
use everruns_platform::{EndpointTransport, SlackChannelConfig};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::common::ErrorResponse;
use super::slack_events::SlackState;
use crate::auth::{AuthState, ResolvedOrg};
use crate::slack_provisioning::{SlackApiProvisioner, SlackProvisioningSetup};
use crate::storage::OrgSlackConnectionRow;

/// How long a minted `install_state` stays valid.
///
/// The operator is mid-flow with a consent screen in front of them, so this is
/// generous rather than tight; its job is to stop an abandoned install leaving
/// a nonce that is good forever, not to race the human.
const INSTALL_STATE_TTL_MINUTES: i64 = 30;

/// Slack's OAuth v2 endpoints. Separated from `SLACK_API_BASE` so tests can
/// point the exchange at a stub without redirecting every other Slack call.
const SLACK_OAUTH_AUTHORIZE_URL: &str = "https://slack.com/oauth/v2/authorize";

#[derive(Clone)]
pub struct SlackInstallState {
    pub slack: SlackState,
    pub auth: AuthState,
    pub provisioner: Arc<dyn SlackAppProvisioner>,
    pub connection_manager: Option<Arc<SlackApiProvisioner>>,
    /// Where `oauth.v2.access` is posted. Overridden in tests.
    pub slack_api_base: String,
    /// Where the callback sends the operator's browser when it is done.
    pub ui_base_url: String,
}

#[derive(Deserialize)]
pub struct ConnectSlackWorkspaceRequest {
    refresh_token: String,
}

/// A connected workspace as the UI sees it. Never carries a token.
#[derive(Serialize)]
pub struct SlackWorkspace {
    pub id: uuid::Uuid,
    /// `None` only briefly, for a connection made before workspaces were
    /// recorded; the rotation sweep fills it.
    pub team_id: Option<String>,
    pub team_name: Option<String>,
    /// `connected` or `reconnect_required`.
    pub status: &'static str,
    pub connected_at: chrono::DateTime<chrono::Utc>,
}

impl From<OrgSlackConnectionRow> for SlackWorkspace {
    fn from(row: OrgSlackConnectionRow) -> Self {
        Self {
            id: row.id,
            team_id: row.team_id,
            team_name: row.team_name,
            status: if row.state == "reconnect_required" {
                "reconnect_required"
            } else {
                "connected"
            },
            connected_at: row.created_at,
        }
    }
}

/// GET /v1/slack/workspaces — any member: builders choose from this list when
/// putting an agent in Slack, and it holds names, not credentials.
async fn list_workspaces(
    org: ResolvedOrg,
    State(state): State<SlackInstallState>,
) -> Result<Json<Vec<SlackWorkspace>>, (StatusCode, Json<ErrorResponse>)> {
    let Some(manager) = state.connection_manager else {
        return Ok(Json(Vec::new()));
    };
    let rows = manager
        .list_connections(org.org_id)
        .await
        .map_err(connection_error_response)?;
    Ok(Json(rows.into_iter().map(SlackWorkspace::from).collect()))
}

/// POST /v1/slack/workspaces — admin only. Connects whichever workspace the
/// refresh token belongs to; Slack tells us which on the first rotation.
async fn connect_workspace(
    org: ResolvedOrg,
    State(state): State<SlackInstallState>,
    Json(request): Json<ConnectSlackWorkspaceRequest>,
) -> Result<Json<SlackWorkspace>, (StatusCode, Json<ErrorResponse>)> {
    require_org_admin(&org)?;
    let manager = state.connection_manager.ok_or_else(unsupported_response)?;
    let row = manager
        .connect(org.org_id, &request.refresh_token)
        .await
        .map_err(connection_error_response)?;
    Ok(Json(row.into()))
}

async fn test_workspace(
    org: ResolvedOrg,
    State(state): State<SlackInstallState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    require_org_admin(&org)?;
    let manager = state.connection_manager.ok_or_else(unsupported_response)?;
    manager
        .test_connection(org.org_id, id)
        .await
        .map_err(connection_error_response)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn disconnect_workspace(
    org: ResolvedOrg,
    State(state): State<SlackInstallState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    require_org_admin(&org)?;
    let manager = state.connection_manager.ok_or_else(unsupported_response)?;
    if manager
        .clear_connection(org.org_id, id)
        .await
        .map_err(connection_error_response)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("Slack workspace not found").into_response(StatusCode::NOT_FOUND))
    }
}

/// The workspace an install targets, checked against what this organization
/// has connected.
///
/// A named workspace must be one of the organization's — it is stored on the
/// endpoint and put on Slack's consent URL, so it is never taken on trust. With
/// none named, the organization's only workspace, so the consent screen can
/// still be pre-selected; several connected is left to the provisioner, which
/// refuses to guess.
async fn chosen_workspace(
    state: &SlackInstallState,
    org_id: i64,
    requested: Option<String>,
) -> Result<Option<String>, (StatusCode, Json<ErrorResponse>)> {
    let requested = requested.filter(|team| !team.is_empty());
    let Some(manager) = &state.connection_manager else {
        // A custom provisioner owns workspace resolution entirely.
        return Ok(requested);
    };
    let rows = manager
        .list_connections(org_id)
        .await
        .map_err(provisioning_error_response)?;
    pick_workspace(&rows, requested).map_err(provisioning_error_response)
}

fn pick_workspace(
    rows: &[OrgSlackConnectionRow],
    requested: Option<String>,
) -> Result<Option<String>, SlackProvisioningError> {
    match requested {
        Some(team_id)
            if rows
                .iter()
                .any(|row| row.team_id.as_deref() == Some(&team_id)) =>
        {
            Ok(Some(team_id))
        }
        Some(_) => Err(SlackProvisioningError::WorkspaceNotConnected),
        None => Ok(match rows {
            [only] => only.team_id.clone(),
            _ => None,
        }),
    }
}

fn require_org_admin(org: &ResolvedOrg) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if org.role.has_permission(everruns_core::OrgRole::Admin) {
        Ok(())
    } else {
        Err(
            ErrorResponse::new("Organization administrator access required")
                .into_response(StatusCode::FORBIDDEN),
        )
    }
}

fn unsupported_response() -> (StatusCode, Json<ErrorResponse>) {
    ErrorResponse::new("Slack app provisioning is not supported on this deployment")
        .into_response(StatusCode::NOT_IMPLEMENTED)
}

/// Errors from connecting, testing or listing a workspace.
///
/// Any refusal from Slack here is about the token the admin supplied, never
/// about creating an app, and it is their input to fix rather than an upstream
/// outage. A malformed token comes back as `invalid_arguments`, not
/// `invalid_refresh_token`, so the code is reported rather than matched on.
fn connection_error_response(error: SlackProvisioningError) -> (StatusCode, Json<ErrorResponse>) {
    match error {
        SlackProvisioningError::Rejected(code) => ErrorResponse::new(format!(
            "Slack did not accept the configuration refresh token ({code}). Generate a new one and paste its refresh token."
        ))
        .into_response(StatusCode::BAD_REQUEST),
        other => provisioning_error_response(other),
    }
}

impl axum::extract::FromRef<SlackInstallState> for AuthState {
    fn from_ref(state: &SlackInstallState) -> AuthState {
        state.auth.clone()
    }
}

impl SlackInstallState {
    /// `provisioner` is `None` on any deployment that holds no Slack app
    /// configuration token, which stands in the absent one so the route answers
    /// "not configured here" in one shape rather than being absent.
    pub fn new(
        slack: SlackState,
        auth: AuthState,
        ui_base_url: String,
        setup: SlackProvisioningSetup,
    ) -> Self {
        let provisioner = setup
            .provisioner
            .unwrap_or_else(|| Arc::new(UnavailableSlackAppProvisioner));
        Self {
            slack,
            auth,
            provisioner,
            connection_manager: setup.connection_manager,
            slack_api_base: super::slack_events::SLACK_API_BASE.to_string(),
            ui_base_url,
        }
    }
}

pub fn routes(state: SlackInstallState) -> Router {
    Router::new()
        .route("/v1/slack/install", get(install_capability))
        .route(
            "/v1/slack/workspaces",
            get(list_workspaces).post(connect_workspace),
        )
        .route("/v1/slack/workspaces/{id}", delete(disconnect_workspace))
        .route("/v1/slack/workspaces/{id}/test", post(test_workspace))
        .route("/v1/e/{channel_id}/slack/install", post(begin_install))
        .route(
            "/v1/e/{channel_id}/slack/oauth/callback",
            get(finish_install),
        )
        .with_state(state)
}

#[derive(Serialize)]
pub struct SlackInstallCapability {
    pub supported: bool,
    pub connected: bool,
    pub reconnect_required: bool,
    pub can_manage: bool,
}

async fn install_capability(
    org: ResolvedOrg,
    State(state): State<SlackInstallState>,
) -> Result<Json<SlackInstallCapability>, (StatusCode, Json<ErrorResponse>)> {
    let supported = state.provisioner.deployment_supported();
    let status = if supported {
        state
            .provisioner
            .connection_status(org.org_id)
            .await
            .map_err(provisioning_error_response)?
    } else {
        SlackProvisioningConnectionStatus::default()
    };
    Ok(Json(SlackInstallCapability {
        supported,
        connected: status.connected,
        reconnect_required: status.reconnect_required,
        can_manage: state.connection_manager.is_some()
            && org.role.has_permission(everruns_core::OrgRole::Admin),
    }))
}
#[derive(Serialize)]
pub struct BeginInstallResponse {
    /// Send the operator here. Slack shows one consent screen and then
    /// redirects to this endpoint's callback.
    pub authorize_url: String,
}

#[derive(Default, Deserialize)]
pub struct BeginInstallRequest {
    /// The connected workspace (`T…`) to create the agent's app in. May be
    /// omitted while the organization has connected exactly one.
    #[serde(default)]
    team_id: Option<String>,
}

async fn resolve_install_endpoint(
    state: &SlackState,
    channel_id: &str,
) -> Result<
    (
        super::endpoint_ingress::IngressContext,
        super::endpoint_ingress::IngressEndpoint,
    ),
    (StatusCode, Json<ErrorResponse>),
> {
    // Setup precedes publication. Ingress keeps its liveness gate; setup is
    // protected by the caller's organization or the callback's install nonce.
    let endpoint =
        super::endpoint_ingress::resolve_endpoint(&state.db, state.encryption.as_ref(), channel_id)
            .await
            .map_err(|error| {
                tracing::error!(channel_id, %error, "Failed to lookup Slack install endpoint");
                ErrorResponse::new("Internal server error")
                    .into_response(StatusCode::INTERNAL_SERVER_ERROR)
            })?
            .filter(|(_, endpoint)| endpoint.channel_type == EndpointTransport::Slack)
            .ok_or_else(|| {
                ErrorResponse::new("Endpoint not found").into_response(StatusCode::NOT_FOUND)
            })?;
    Ok(endpoint)
}

/// POST /v1/e/{channel_id}/slack/install
async fn begin_install(
    org: ResolvedOrg,
    State(state): State<SlackInstallState>,
    Path(channel_id): Path<String>,
    body: axum::body::Bytes,
) -> Result<Json<BeginInstallResponse>, (StatusCode, Json<ErrorResponse>)> {
    // The body is optional so a single-workspace organization can keep
    // posting nothing; an empty body means "the only workspace".
    let request: BeginInstallRequest = if body.is_empty() {
        BeginInstallRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| {
            ErrorResponse::new("Invalid install request").into_response(StatusCode::BAD_REQUEST)
        })?
    };
    let (app, endpoint) = resolve_install_endpoint(&state.slack, &channel_id).await?;

    // The webhook routes resolve an endpoint by public id alone because Slack
    // is the caller and there is no org to check against. Here there is one,
    // and skipping it would let any authenticated user provision an app onto
    // another org's endpoint.
    if app.org_id != org.org_id {
        return Err(ErrorResponse::new("Endpoint not found").into_response(StatusCode::NOT_FOUND));
    }
    // THREAT[TM-DOS-042]: app creation spends a deployment-wide Slack
    // credential and cannot be rolled back atomically with our database write.
    // Serialize it in PostgreSQL across server instances, then re-read under
    // the lock so concurrent requests reuse the winner instead of creating
    // orphaned apps. The guard stays live through persistence.
    let _install_lock = state
        .slack
        .db
        .lock_slack_install(endpoint.internal_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "Failed to lock Slack app installation");
            ErrorResponse::new("Internal server error")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    let (app, endpoint) = resolve_install_endpoint(&state.slack, &channel_id).await?;

    let mut config = parse_config(&endpoint.channel_config);

    // THREAT[TM-SLACK-008]: pin the workspace before anything is created. An
    // app lives in exactly one workspace, so the consent screen, retries and
    // reaping all have to agree on it, and the choice is checked against this
    // organization's connections rather than taken from the request on trust.
    let team_id = chosen_workspace(&state, org.org_id, request.team_id).await?;

    // Reuse the app the endpoint already has rather than creating a second one
    // per retry: a click that failed after creation but before consent would
    // otherwise orphan one app per attempt. Not across workspaces, though —
    // an app made for one workspace cannot be installed into another, so a
    // changed choice reaps the old app and creates a fresh one.
    let (reusable, stale) = match config.provisioned_app.take() {
        Some(existing)
            if existing.team_id.is_none() || team_id.is_none() || existing.team_id == team_id =>
        {
            (Some(existing), None)
        }
        other => (None, other),
    };
    if let Some(stale) = stale {
        // Reaping is only safe before the app was installed. Once it holds a
        // bot token the agent is live in that workspace, and silently deleting
        // it to satisfy a changed choice would take the agent offline.
        if !config.bot_token.is_empty() {
            return Err(ErrorResponse::new(
                "This agent is already live in another Slack workspace; disconnect it there first",
            )
            .into_response(StatusCode::CONFLICT));
        }
        if let Err(error) = state
            .provisioner
            .delete_app(org.org_id, stale.team_id.as_deref(), &stale.app_id)
            .await
        {
            tracing::warn!(app_id = %stale.app_id, %error, "Could not reap Slack app created for another workspace");
        }
    }
    let mut provisioned = match reusable {
        Some(mut existing) => {
            if existing.team_id.is_none() {
                existing.team_id = team_id.clone();
            }
            existing
        }
        None => {
            let manifest =
                super::slack_events::manifest_yaml_for_endpoint(&state.slack, &app, &endpoint)
                    .await?;
            let created = state
                .provisioner
                .create_app(org.org_id, team_id.as_deref(), &manifest)
                .await
                .map_err(provisioning_error_response)?;
            config.signing_secret = created.signing_secret;
            ProvisionedSlackApp {
                app_id: created.app_id,
                client_id: created.client_id,
                client_secret: created.client_secret,
                install_state: None,
                install_state_issued_at: None,
                team_id: team_id.clone(),
            }
        }
    };

    let install_state = mint_install_state();
    let client_id = provisioned.client_id.clone();
    let team_id_for_consent = provisioned.team_id.clone();
    provisioned.install_state = Some(install_state.clone());
    provisioned.install_state_issued_at = Some(chrono::Utc::now());
    config.provisioned_app = Some(provisioned);

    persist(&state, endpoint.internal_id, &config).await?;
    let redirect_uri = super::slack_events::slack_oauth_redirect_url(
        &state.slack.api_base_url,
        &endpoint.public_id.to_string(),
    );
    let authorize_url = authorize_url(
        &client_id,
        &install_state,
        &redirect_uri,
        team_id_for_consent.as_deref(),
        config.agent_surface_enabled,
    );
    Ok(Json(BeginInstallResponse { authorize_url }))
}

fn authorize_url(
    client_id: &str,
    install_state: &str,
    redirect_uri: &str,
    team_id: Option<&str>,
    agent_surface_enabled: bool,
) -> String {
    let scopes = super::slack_events::slack_bot_scopes(agent_surface_enabled).join(",");
    let mut authorize_url = format!(
        "{SLACK_OAUTH_AUTHORIZE_URL}?client_id={}&state={}&redirect_uri={}&scope={}",
        urlencoding_encode(client_id),
        urlencoding_encode(install_state),
        urlencoding_encode(redirect_uri),
        urlencoding_encode(&scopes),
    );
    // Pre-select the workspace on Slack's consent screen. Without it Slack
    // offers every workspace the operator belongs to, and picking any other
    // fails, since the app exists only in this one.
    if let Some(team_id) = team_id {
        authorize_url.push_str(&format!("&team={}", urlencoding_encode(team_id)));
    }
    authorize_url
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// GET /v1/e/{channel_id}/slack/oauth/callback
///
/// THREAT[TM-SLACK-009]: Slack echoes `state` back here without verifying it, and
/// this route cannot be authenticated because it is reached by a browser
/// redirect. Without a stored single-use nonce an attacker could drive it with
/// an authorization code from their own workspace and bind that workspace to
/// someone else's endpoint. The nonce is compared in constant time, checked for
/// expiry, and cleared before the exchange result is stored, so a replay of the
/// same callback URL finds nothing to match.
async fn finish_install(
    State(state): State<SlackInstallState>,
    Path(channel_id): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let ui_base = state.ui_base_url.trim_end_matches('/');
    let rejected = |reason: &str| {
        tracing::warn!(%channel_id, reason, "Slack install callback rejected");
        Redirect::to(&format!("{ui_base}/agents?slack_install=failed")).into_response()
    };
    let (context, endpoint) = match resolve_install_endpoint(&state.slack, &channel_id).await {
        Ok(endpoint) => endpoint,
        Err(_) => return rejected("endpoint not found"),
    };
    let mut config = parse_config(&endpoint.channel_config);
    let Some(mut provisioned) = config.provisioned_app.take() else {
        return rejected("no provisioned app");
    };
    if let Err(reason) = spend_install_state(
        &mut provisioned,
        query.state.as_deref().unwrap_or_default(),
        chrono::Utc::now(),
    ) {
        return rejected(reason);
    }

    // Endpoint editors are agent-scoped. Only a valid install nonce may expose
    // the owning agent in this unauthenticated callback's return URL.
    let editor_url = match context.agent_id {
        Some(agent_id) => format!(
            "{ui_base}/agents/{agent_id}/endpoints/{}",
            endpoint.public_id
        ),
        None => format!("{ui_base}/agents"),
    };
    let outcome = match finish_install_inner(&state, &endpoint, config, provisioned, query).await {
        Ok(()) => "ok",
        Err(reason) => {
            // The operator sees a generic marker; the detail stays in the log.
            // This page is reached by a redirect an attacker can also trigger.
            tracing::warn!(%channel_id, reason, "Slack install callback rejected");
            "failed"
        }
    };
    Redirect::to(&format!("{editor_url}?slack_install={outcome}")).into_response()
}

async fn finish_install_inner(
    state: &SlackInstallState,
    endpoint: &super::endpoint_ingress::IngressEndpoint,
    mut config: SlackChannelConfig,
    provisioned: ProvisionedSlackApp,
    query: CallbackQuery,
) -> Result<(), &'static str> {
    if query.error.is_some() {
        // Slack reports a declined consent this way; it is not an error of ours.
        return Err("slack reported an error");
    }
    let code = query.code.filter(|c| !c.is_empty()).ok_or("missing code")?;
    let client_id = provisioned.client_id.clone();
    if provisioned.client_secret.is_empty() {
        return Err("no client secret");
    }
    let redirect_uri = super::slack_events::slack_oauth_redirect_url(
        &state.slack.api_base_url,
        &endpoint.public_id.to_string(),
    );
    let exchanged = exchange_code(
        &state.slack_api_base,
        &client_id,
        &provisioned.client_secret,
        &code,
        &redirect_uri,
    )
    .await?;

    // The nonce was taken above, so storing `provisioned` back spends it in the
    // same write that records the result and a replay finds nothing to match.
    config.provisioned_app = Some(provisioned);
    config.bot_token = exchanged.bot_token;
    config.team_id = Some(exchanged.team_id);

    persist(state, endpoint.internal_id, &config)
        .await
        .map_err(|_| "failed to store install result")?;
    Ok(())
}

/// Check the presented nonce against the stored one and spend it.
///
/// Pure and separately tested because it is the whole of this route's
/// protection: it has no auth, so every way in is through here. Takes `now`
/// rather than reading the clock so expiry is testable without sleeping.
///
/// On success the nonce and its timestamp are cleared from `provisioned`, so
/// the caller storing it back is what makes a replay find nothing to match.
fn spend_install_state(
    provisioned: &mut ProvisionedSlackApp,
    presented: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), &'static str> {
    let expected = provisioned
        .install_state
        .take()
        .ok_or("no install in flight")?;
    let issued = provisioned.install_state_issued_at.take();
    if !constant_time_eq(&expected, presented) {
        return Err("state mismatch");
    }
    let issued = issued.ok_or("no issue time")?;
    if now - issued > chrono::Duration::minutes(INSTALL_STATE_TTL_MINUTES) {
        return Err("install state expired");
    }
    Ok(())
}

struct ExchangedInstall {
    bot_token: String,
    team_id: String,
}

async fn exchange_code(
    api_base: &str,
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
) -> Result<ExchangedInstall, &'static str> {
    let response = reqwest::Client::new()
        .post(format!(
            "{}/oauth.v2.access",
            api_base.trim_end_matches('/')
        ))
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await
        .map_err(|_| "oauth exchange unreachable")?;

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "oauth exchange returned no json")?;

    if body.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err("oauth exchange refused");
    }
    let bot_token = body
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or("oauth exchange returned no access token")?
        .to_string();
    let team_id = body
        .get("team")
        .and_then(|team| team.get("id"))
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("oauth exchange returned no team id")?
        .to_string();
    Ok(ExchangedInstall { bot_token, team_id })
}

/// The endpoint's Slack config, or an empty one when it will not parse.
///
/// Matching the manifest route's read: a config we cannot parse is not a 500,
/// because the install is how an operator recovers from exactly that. Built
/// from `{}` rather than `Default` so every field's serde default applies,
/// which is the same shape a never-configured endpoint has.
fn parse_config(raw: &serde_json::Value) -> SlackChannelConfig {
    serde_json::from_value(raw.clone()).unwrap_or_else(|_| {
        serde_json::from_value(serde_json::json!({}))
            .expect("an empty object is a valid SlackChannelConfig")
    })
}

async fn persist(
    state: &SlackInstallState,
    endpoint_internal_id: uuid::Uuid,
    config: &SlackChannelConfig,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let json = serde_json::to_value(config).map_err(|error| {
        tracing::error!(%error, "Failed to serialise Slack channel config");
        ErrorResponse::new("Internal server error").into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;
    crate::domains::agent_endpoints::queries::update_channel_config_unscoped(
        &state.slack.db,
        state.slack.encryption.as_ref(),
        endpoint_internal_id,
        &json,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "Failed to store Slack channel config");
        ErrorResponse::new("Internal server error").into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })
}

fn provisioning_error_response(error: SlackProvisioningError) -> (StatusCode, Json<ErrorResponse>) {
    match error {
        // Not a failure: it is the self-hosted steady state. 501 rather than
        // 500 so the UI can tell "this deployment does not do one-click" from
        // "one-click broke" and fall back to the manual fields.
        SlackProvisioningError::Unavailable => ErrorResponse::new(
            "One-click Slack install is not configured on this deployment; configure the endpoint manually",
        )
        .into_response(StatusCode::NOT_IMPLEMENTED),
        SlackProvisioningError::OrgNotConnected => ErrorResponse::new(
            "Connect Slack for this organization before using one-click install",
        )
        .into_response(StatusCode::CONFLICT),
        SlackProvisioningError::ReconnectRequired => ErrorResponse::new(
            "Reconnect Slack for this organization before using one-click install",
        )
        .into_response(StatusCode::CONFLICT),
        SlackProvisioningError::WorkspaceRequired => {
            ErrorResponse::new("Choose which connected Slack workspace this agent belongs in")
                .into_response(StatusCode::BAD_REQUEST)
        }
        SlackProvisioningError::WorkspaceNotConnected => {
            ErrorResponse::new("That Slack workspace is not connected to this organization")
                .into_response(StatusCode::NOT_FOUND)
        }
        SlackProvisioningError::Rejected(code) => {
            tracing::warn!(slack_error = %code, "Slack app provisioning rejected");
            ErrorResponse::new(format!("Slack rejected the app creation: {code}"))
                .with_code(code)
                .into_response(StatusCode::BAD_GATEWAY)
        }
        SlackProvisioningError::Unreachable(detail) => {
            tracing::warn!(detail, "Slack app creation unreachable");
            ErrorResponse::new("Could not reach Slack to create the app")
                .into_response(StatusCode::BAD_GATEWAY)
        }
    }
}

/// 256 bits of randomness, hex-encoded. Long enough that guessing is not a
/// consideration and the value is URL-safe without escaping.
fn mint_install_state() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.random()).collect();
    hex::encode(bytes)
}

/// Comparison that does not leak how much of the nonce matched.
///
/// Hand-written rather than pulling in `subtle` as a direct dependency for
/// four lines. The early length check leaks only the length, which for a
/// fixed-width hex nonce is not a secret.
fn constant_time_eq(expected: &str, presented: &str) -> bool {
    let (expected, presented) = (expected.as_bytes(), presented.as_bytes());
    if expected.len() != presented.len() {
        return false;
    }
    expected
        .iter()
        .zip(presented)
        .fold(0u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

fn urlencoding_encode(value: &str) -> String {
    super::slack_events::urlencoding_encode(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    fn resolved_org(role: everruns_core::OrgRole) -> ResolvedOrg {
        ResolvedOrg {
            org_id: 41,
            public_id: "org_00000000000000000000000000000029".to_string(),
            name: "Test".to_string(),
            user_id: Some(uuid::Uuid::nil()),
            role,
            is_platform_user: false,
            feature_flags: everruns_platform::FeatureFlags::default(),
        }
    }

    #[test]
    fn slack_connection_mutations_require_org_admin() {
        assert!(require_org_admin(&resolved_org(everruns_core::OrgRole::Admin)).is_ok());
        assert!(require_org_admin(&resolved_org(everruns_core::OrgRole::Owner)).is_ok());
        let (status, _) =
            require_org_admin(&resolved_org(everruns_core::OrgRole::Member)).unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    fn config_with_state(state: Option<&str>, issued_minutes_ago: i64) -> SlackChannelConfig {
        let mut config = parse_config(&serde_json::json!({}));
        config.provisioned_app = Some(ProvisionedSlackApp {
            app_id: "A0123".to_string(),
            client_id: "4567.89".to_string(),
            client_secret: "secret".to_string(),
            install_state: state.map(str::to_string),
            install_state_issued_at: Some(
                chrono::Utc::now() - chrono::Duration::minutes(issued_minutes_ago),
            ),
            team_id: None,
        });
        config
    }

    fn connection_row(team_id: Option<&str>) -> OrgSlackConnectionRow {
        OrgSlackConnectionRow {
            id: uuid::Uuid::now_v7(),
            org_id: 41,
            team_id: team_id.map(str::to_string),
            team_name: None,
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            access_token_expires_at: None,
            state: "connected".to_string(),
            token_generation: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn any_slack_refusal_while_connecting_is_reported_as_the_tokens_fault() {
        for code in ["invalid_refresh_token", "invalid_arguments", "invalid_auth"] {
            let (status, Json(body)) =
                connection_error_response(SlackProvisioningError::Rejected(code.to_string()));
            assert_eq!(status, StatusCode::BAD_REQUEST, "{code}");
            let rendered = serde_json::to_string(&body).unwrap();
            assert!(
                rendered.contains("configuration refresh token"),
                "{rendered}"
            );
            assert!(rendered.contains(code), "{rendered}");
            assert!(!rendered.contains("app creation"), "{rendered}");
        }
    }

    #[test]
    fn a_requested_workspace_must_be_one_the_org_connected() {
        let rows = [connection_row(Some("T1")), connection_row(Some("T2"))];
        assert_eq!(
            pick_workspace(&rows, Some("T2".to_string())).unwrap(),
            Some("T2".to_string())
        );
        // Never stored or put on the consent URL on the caller's word alone.
        assert!(matches!(
            pick_workspace(&rows, Some("T_OTHER_ORG".to_string())),
            Err(SlackProvisioningError::WorkspaceNotConnected)
        ));
    }

    #[test]
    fn with_no_choice_only_a_sole_workspace_is_assumed() {
        assert_eq!(
            pick_workspace(&[connection_row(Some("T1"))], None).unwrap(),
            Some("T1".to_string())
        );
        // Several connected: no guess here; the provisioner refuses later.
        assert_eq!(
            pick_workspace(
                &[connection_row(Some("T1")), connection_row(Some("T2"))],
                None
            )
            .unwrap(),
            None
        );
        assert_eq!(pick_workspace(&[], None).unwrap(), None);
    }

    #[test]
    fn minted_state_is_long_random_hex() {
        let first = mint_install_state();
        let second = mint_install_state();
        assert_eq!(first.len(), 64, "256 bits, hex-encoded");
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()), "{first}");
        assert_ne!(first, second, "a fixed nonce would defeat the check");
    }

    #[test]
    fn state_comparison_rejects_mismatch_and_prefix() {
        let expected = mint_install_state();
        assert!(constant_time_eq(&expected, &expected.clone()));
        assert!(!constant_time_eq(&expected, ""));
        // A prefix must not pass: a length-only check would let one through.
        assert!(!constant_time_eq(&expected, &expected[..32]));
        let mut flipped = expected.clone();
        flipped.replace_range(63..64, if expected.ends_with('a') { "b" } else { "a" });
        assert!(!constant_time_eq(&expected, &flipped));
    }

    #[test]
    fn an_empty_object_parses_as_an_unconfigured_endpoint() {
        let config = parse_config(&serde_json::json!({}));
        assert!(config.provisioned_app.is_none());
    }

    #[test]
    fn an_unparseable_config_does_not_panic() {
        let config = parse_config(&serde_json::json!("not an object"));
        assert!(config.provisioned_app.is_none());
    }

    #[test]
    fn credentials_round_trip_through_the_stored_config() {
        let config = config_with_state(Some("abc"), 0);
        let json = serde_json::to_value(&config).expect("serialises");
        let provisioned = parse_config(&json).provisioned_app.expect("round-trips");
        assert_eq!(provisioned.client_id, "4567.89");
        assert_eq!(provisioned.client_secret, "secret");
        assert_eq!(provisioned.install_state.as_deref(), Some("abc"));
    }

    #[test]
    fn an_expired_install_state_is_outside_the_window() {
        let issued_at = |config: SlackChannelConfig| {
            config
                .provisioned_app
                .and_then(|app| app.install_state_issued_at)
                .expect("issued")
        };
        let window = chrono::Duration::minutes(INSTALL_STATE_TTL_MINUTES);
        assert!(chrono::Utc::now() - issued_at(config_with_state(Some("abc"), 0)) <= window);
        assert!(
            chrono::Utc::now()
                - issued_at(config_with_state(
                    Some("abc"),
                    INSTALL_STATE_TTL_MINUTES + 1
                ))
                > window
        );
    }

    #[tokio::test]
    async fn exchange_rejects_a_body_slack_marked_not_ok() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth.v2.access"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": false, "error": "invalid_code"})),
            )
            .mount(&server)
            .await;
        let result = exchange_code(&server.uri(), "id", "secret", "code", "https://x/cb").await;
        assert_eq!(result.err(), Some("oauth exchange refused"));
    }

    #[tokio::test]
    async fn exchange_requires_both_a_token_and_a_team() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth.v2.access"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"ok": true, "access_token": "xoxb-1", "team": {}}),
            ))
            .mount(&server)
            .await;
        let result = exchange_code(&server.uri(), "id", "secret", "code", "https://x/cb").await;
        assert_eq!(result.err(), Some("oauth exchange returned no team id"));
    }

    #[tokio::test]
    async fn exchange_takes_the_bot_token_and_workspace_from_a_good_response() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth.v2.access"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": true,
                    "access_token": "xoxb-real-token",
                    "team": {"id": "T0123", "name": "Acme"},
                })),
            )
            .mount(&server)
            .await;
        let exchanged = exchange_code(&server.uri(), "id", "secret", "code", "https://x/cb")
            .await
            .expect("exchange succeeds");
        assert_eq!(exchanged.bot_token, "xoxb-real-token");
        assert_eq!(exchanged.team_id, "T0123");
    }

    fn provisioned(state: Option<&str>, issued_minutes_ago: Option<i64>) -> ProvisionedSlackApp {
        ProvisionedSlackApp {
            app_id: "A0123".to_string(),
            client_id: "4567.89".to_string(),
            client_secret: "secret".to_string(),
            install_state: state.map(str::to_string),
            install_state_issued_at: issued_minutes_ago
                .map(|ago| chrono::Utc::now() - chrono::Duration::minutes(ago)),
            team_id: None,
        }
    }

    #[test]
    fn spending_a_matching_nonce_clears_it() {
        let nonce = mint_install_state();
        let mut app = provisioned(Some(&nonce), Some(0));
        assert!(spend_install_state(&mut app, &nonce, chrono::Utc::now()).is_ok());
        assert!(app.install_state.is_none(), "a replay must find nothing");
        assert!(app.install_state_issued_at.is_none());
    }

    #[test]
    fn a_wrong_nonce_is_refused_and_still_burns_the_stored_one() {
        let nonce = mint_install_state();
        let mut app = provisioned(Some(&nonce), Some(0));
        assert_eq!(
            spend_install_state(&mut app, "not-the-nonce", chrono::Utc::now()),
            Err("state mismatch")
        );
        // Taken even on refusal: leaving it set would let an attacker who can
        // trigger the callback keep guessing against the same value.
        assert!(app.install_state.is_none());
    }

    #[test]
    fn an_empty_nonce_cannot_stand_in_for_a_missing_one() {
        let mut app = provisioned(Some(&mint_install_state()), Some(0));
        assert_eq!(
            spend_install_state(&mut app, "", chrono::Utc::now()),
            Err("state mismatch")
        );
    }

    #[test]
    fn a_callback_with_no_install_in_flight_is_refused() {
        let mut app = provisioned(None, None);
        assert_eq!(
            spend_install_state(&mut app, "anything", chrono::Utc::now()),
            Err("no install in flight")
        );
    }

    #[test]
    fn a_nonce_past_its_window_is_refused_even_when_it_matches() {
        let nonce = mint_install_state();
        let mut app = provisioned(Some(&nonce), Some(INSTALL_STATE_TTL_MINUTES + 1));
        assert_eq!(
            spend_install_state(&mut app, &nonce, chrono::Utc::now()),
            Err("install state expired")
        );
    }

    #[test]
    fn a_nonce_inside_its_window_is_accepted() {
        let nonce = mint_install_state();
        let mut app = provisioned(Some(&nonce), Some(INSTALL_STATE_TTL_MINUTES - 1));
        assert!(spend_install_state(&mut app, &nonce, chrono::Utc::now()).is_ok());
    }

    #[test]
    fn consent_requests_manifest_scopes_for_both_slack_surfaces() {
        for agent_surface_enabled in [false, true] {
            let redirect = "https://example.com/api/v1/e/test/slack/oauth/callback";
            let url = url::Url::parse(&authorize_url(
                "client-id",
                "nonce",
                redirect,
                Some("T1"),
                agent_surface_enabled,
            ))
            .unwrap();
            let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
            assert_eq!(params["client_id"], "client-id");
            assert_eq!(params["state"], "nonce");
            assert_eq!(params["redirect_uri"], redirect);
            assert_eq!(params["team"], "T1");
            let scopes: Vec<_> = params["scope"].split(',').collect();
            assert!(scopes.contains(&"chat:write"));
            assert_eq!(scopes.contains(&"assistant:write"), agent_surface_enabled);

            let yaml = super::super::slack_events::build_manifest_yaml(
                "Agent",
                "Agent",
                None,
                "https://example.com/events",
                "https://example.com/actions",
                redirect,
                agent_surface_enabled,
                &[],
            );
            let manifest: serde_json::Value = serde_yaml::from_str(&yaml).unwrap();
            let declared: Vec<_> = manifest["oauth_config"]["scopes"]["bot"]
                .as_array()
                .unwrap()
                .iter()
                .map(|scope| scope.as_str().unwrap())
                .collect();
            assert_eq!(scopes, declared);
        }
    }

    #[test]
    fn an_absent_provisioner_answers_not_implemented_rather_than_error() {
        let (status, _) = provisioning_error_response(SlackProvisioningError::Unavailable);
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        let (status, Json(problem)) =
            provisioning_error_response(SlackProvisioningError::Rejected("ratelimited".into()));
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(problem.code.as_deref(), Some("ratelimited"));
    }

    #[tokio::test]
    async fn concurrent_installs_for_one_endpoint_are_serialized() {
        let db = Arc::new(StorageBackend::in_memory());
        let endpoint_id = uuid::Uuid::now_v7();
        let first = db
            .lock_slack_install(endpoint_id)
            .await
            .expect("first lock");

        let waiting_db = db.clone();
        let waiting = tokio::spawn(async move {
            let _guard = waiting_db
                .lock_slack_install(endpoint_id)
                .await
                .expect("second lock");
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), waiting)
                .await
                .is_err(),
            "a concurrent install must wait for the endpoint lock"
        );

        drop(first);
        db.lock_slack_install(endpoint_id)
            .await
            .expect("lock released when guard drops");
    }
}
