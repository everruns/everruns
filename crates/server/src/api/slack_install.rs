//! One-click Slack install: create the Slack channel's app, then finish its OAuth.
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
//! * `POST /v1/e/{channel_id}/slack/install` is authenticated. It spends the
//!   deployment's app configuration token and creates a real app in the
//!   deployment's Slack account, so an unauthenticated caller could exhaust a
//!   company credential and fill that account with orphans.
//! * `GET /v1/e/{channel_id}/slack/oauth/callback` cannot be: Slack redirects
//!   the operator's browser to it and carries none of our auth. It is
//!   protected by the single-use `install_state` nonce instead.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use everruns_platform::slack_provisioning::{
    ProvisionedSlackApp, SlackAppProvisioner, SlackProvisioningError,
    UnavailableSlackAppProvisioner,
};
use everruns_platform::{ChannelType, SlackChannelConfig};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::common::ErrorResponse;
use super::slack_events::{SlackState, SlackTarget};
use crate::auth::{AuthState, ResolvedOrg};

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
    /// Where `oauth.v2.access` is posted. Overridden in tests.
    pub slack_api_base: String,
    /// Where the callback sends the operator's browser when it is done.
    pub ui_base_url: String,
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
        provisioner: Option<Arc<dyn SlackAppProvisioner>>,
    ) -> Self {
        Self {
            slack,
            auth,
            provisioner: provisioner.unwrap_or_else(|| Arc::new(UnavailableSlackAppProvisioner)),
            slack_api_base: super::slack_events::SLACK_API_BASE.to_string(),
            ui_base_url,
        }
    }
}

pub fn routes(state: SlackInstallState) -> Router {
    Router::new()
        .route("/v1/e/{channel_id}/slack/install", post(begin_install))
        .route(
            "/v1/e/{channel_id}/slack/oauth/callback",
            get(finish_install),
        )
        .with_state(state)
}

#[derive(Serialize)]
pub struct BeginInstallResponse {
    /// Send the operator here. Slack shows one consent screen and then
    /// redirects to this channel's callback.
    pub authorize_url: String,
}

/// POST /v1/e/{channel_id}/slack/install
async fn begin_install(
    org: ResolvedOrg,
    State(state): State<SlackInstallState>,
    Path(channel_id): Path<String>,
) -> Result<Json<BeginInstallResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (app, slack_channel) = super::slack_events::resolve_slack_channel(
        &state.slack,
        SlackTarget::Channel(channel_id.clone()),
    )
    .await?;

    // The webhook routes resolve a channel by public id alone because Slack
    // is the caller and there is no org to check against. Here there is one,
    // and skipping it would let any authenticated user provision an app onto
    // another org's channel.
    if app.org_id != org.org_id {
        return Err(ErrorResponse::new("Channel not found").into_response(StatusCode::NOT_FOUND));
    }
    if slack_channel.channel_type != ChannelType::Slack {
        return Err(ErrorResponse::new("Channel is not a Slack channel")
            .into_response(StatusCode::BAD_REQUEST));
    }

    let mut config = parse_config(&slack_channel.channel_config);

    // Reuse the app the channel already has rather than creating a second one
    // per retry: a click that failed after creation but before consent would
    // otherwise orphan one app per attempt.
    let mut provisioned = match config.provisioned_app.take() {
        Some(existing) => existing,
        None => {
            let manifest =
                super::slack_events::manifest_yaml_for_channel(&state.slack, &app, &slack_channel)
                    .await?;
            let created = state
                .provisioner
                .create_app(&manifest)
                .await
                .map_err(provisioning_error_response)?;
            config.signing_secret = created.signing_secret;
            ProvisionedSlackApp {
                app_id: created.app_id,
                client_id: created.client_id,
                client_secret: created.client_secret,
                install_state: None,
                install_state_issued_at: None,
            }
        }
    };

    let install_state = mint_install_state();
    let client_id = provisioned.client_id.clone();
    provisioned.install_state = Some(install_state.clone());
    provisioned.install_state_issued_at = Some(chrono::Utc::now());
    config.provisioned_app = Some(provisioned);

    persist(&state, slack_channel.internal_id, &config).await?;
    let redirect_uri = super::slack_events::slack_oauth_redirect_url(
        &state.slack.api_base_url,
        &slack_channel.public_id.to_string(),
    );
    Ok(Json(BeginInstallResponse {
        authorize_url: format!(
            "{SLACK_OAUTH_AUTHORIZE_URL}?client_id={}&state={}&redirect_uri={}",
            urlencoding_encode(&client_id),
            urlencoding_encode(&install_state),
            urlencoding_encode(&redirect_uri),
        ),
    }))
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// GET /v1/e/{channel_id}/slack/oauth/callback
///
/// THREAT[TM-API-018]: Slack echoes `state` back here without verifying it, and
/// this route cannot be authenticated because it is reached by a browser
/// redirect. Without a stored single-use nonce an attacker could drive it with
/// an authorization code from their own workspace and bind that workspace to
/// someone else's channel. The nonce is compared in constant time, checked for
/// expiry, and cleared before the exchange result is stored, so a replay of the
/// same callback URL finds nothing to match.
async fn finish_install(
    State(state): State<SlackInstallState>,
    Path(channel_id): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    match finish_install_inner(&state, &channel_id, query).await {
        Ok(()) => Redirect::to(&format!(
            "{}/endpoints/{channel_id}?slack_install=ok",
            state.ui_base_url.trim_end_matches('/')
        ))
        .into_response(),
        Err(reason) => {
            // The operator sees a generic marker; the detail stays in the log.
            // This page is reached by a redirect an attacker can also trigger.
            tracing::warn!(%channel_id, reason, "Slack install callback rejected");
            Redirect::to(&format!(
                "{}/endpoints/{channel_id}?slack_install=failed",
                state.ui_base_url.trim_end_matches('/')
            ))
            .into_response()
        }
    }
}

async fn finish_install_inner(
    state: &SlackInstallState,
    channel_id: &str,
    query: CallbackQuery,
) -> Result<(), &'static str> {
    if query.error.is_some() {
        // Slack reports a declined consent this way; it is not an error of ours.
        return Err("slack reported an error");
    }
    let code = query.code.filter(|c| !c.is_empty()).ok_or("missing code")?;
    let presented = query.state.unwrap_or_default();

    let (_, slack_channel) = super::slack_events::resolve_slack_channel(
        &state.slack,
        SlackTarget::Channel(channel_id.to_string()),
    )
    .await
    .map_err(|_| "channel not found")?;

    let mut config = parse_config(&slack_channel.channel_config);
    let mut provisioned = config.provisioned_app.take().ok_or("no provisioned app")?;
    spend_install_state(&mut provisioned, &presented, chrono::Utc::now())?;
    let client_id = provisioned.client_id.clone();
    if provisioned.client_secret.is_empty() {
        return Err("no client secret");
    }
    let redirect_uri = super::slack_events::slack_oauth_redirect_url(
        &state.slack.api_base_url,
        &slack_channel.public_id.to_string(),
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

    persist(state, slack_channel.internal_id, &config)
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

/// The channel's Slack config, or an empty one when it will not parse.
///
/// Matching the manifest route's read: a config we cannot parse is not a 500,
/// because the install is how an operator recovers from exactly that. Built
/// from `{}` rather than `Default` so every field's serde default applies,
/// which is the same shape a never-configured channel has.
fn parse_config(raw: &serde_json::Value) -> SlackChannelConfig {
    serde_json::from_value(raw.clone()).unwrap_or_else(|_| {
        serde_json::from_value(serde_json::json!({}))
            .expect("an empty object is a valid SlackChannelConfig")
    })
}

async fn persist(
    state: &SlackInstallState,
    channel_internal_id: uuid::Uuid,
    config: &SlackChannelConfig,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let json = serde_json::to_value(config).map_err(|error| {
        tracing::error!(%error, "Failed to serialise Slack channel config");
        ErrorResponse::new("Internal server error").into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;
    crate::domains::apps::queries::update_channel_config_unscoped(
        &state.slack.db,
        state.slack.encryption.as_ref(),
        channel_internal_id,
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
            "One-click Slack install is not configured on this deployment; configure the channel manually",
        )
        .into_response(StatusCode::NOT_IMPLEMENTED),
        SlackProvisioningError::Rejected(code) => {
            ErrorResponse::new(format!("Slack rejected the app creation: {code}"))
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
        });
        config
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
    fn an_empty_object_parses_as_an_unconfigured_channel() {
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
    fn an_absent_provisioner_answers_not_implemented_rather_than_error() {
        let (status, _) = provisioning_error_response(SlackProvisioningError::Unavailable);
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        let (status, _) =
            provisioning_error_response(SlackProvisioningError::Rejected("ratelimited".into()));
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }
}
