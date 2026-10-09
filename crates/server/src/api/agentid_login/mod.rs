//! AgentID consumer sign-in for Public Chat channels.
//!
//! A completed AgentID browser login becomes an end-user virtual user and the
//! existing short runtime session (the `runtime-auth` JWT). Decisions:
//!
//! - An AgentID agent never becomes a management user: no `users` row, no
//!   personal access token, no `/login`, and neither the agent's email nor its
//!   owner's email goes through the management email linker. The account key
//!   is `sub`; `owner_sub` keys the per-owner cap; `owner_email` (only with
//!   the explicit owner-scope opt-in) is a contact.
//! - One deployment-level registered AgentID client, from the environment.
//!   Its secret never reaches logs (`ClientSecret` redacts its `Debug`).
//! - Only a channel whose own auth is the AgentID preset can start a sign-in,
//!   so a runtime session never bypasses a channel's chosen auth (a Google
//!   domain restriction, say).
//! - Sign-in state lives server-side, hashed, single use, ten minutes; neither
//!   the management OAuth state cookie nor `cli_auth_sessions` is reused.
//! - The directory's initiate-login URL carries only `iss` and `login_hint`.
//!   Without a configured default channel there is no org to put an account
//!   in, so nothing is created and the browser is not sent to `/login`.
//! - Independent of `AUTH_MODE`. The runtime JWT keeps its short lifetime;
//!   AgentID has no refresh token, so a new sign-in renews it.
//!
//! See knowledge/integrations/agentid.md.
use crate::api::ag_ui::AgUiState;
use crate::api::channel_auth::verify_agentid_claims;
use crate::api::channel_ingress::{IngressChannel, IngressContext};
use crate::records::{AGENTID_ISSUER, ChannelAuthConfig, ChannelType};
use crate::storage::agentid::{AgentIdAgent, AgentIdLoginState, AgentIdSignIn};
use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::jwk::JwkSet;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Scopes for every sign-in. `profile` carries `owner_sub`, which is required.
const SCOPES: &str = "openid email profile";
/// Added only behind `AGENTID_OWNER_SCOPES`: they put a person's name and
/// address in the token.
const OWNER_SCOPES: &str = "owner_profile owner_email";
const MAX_LOGIN_HINT: usize = 512;
const MAX_DISPLAY_NAME: usize = 255;

/// Redacted in `Debug`, so a logged config never carries the secret.
#[derive(Clone)]
pub(crate) struct ClientSecret(String);

impl std::fmt::Debug for ClientSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientSecret([redacted])")
    }
}

/// The deployment's registered AgentID client.
#[derive(Debug, Clone)]
pub(crate) struct AgentIdLoginConfig {
    pub client_id: String,
    pub client_secret: ClientSecret,
    pub redirect_uri: String,
    pub owner_scopes: bool,
    pub default_channel: Option<String>,
    pub frontend_url: String,
    /// AgentID's base URL; a test points it at a mock.
    pub issuer_base: String,
}

impl AgentIdLoginConfig {
    /// `AGENTID_CLIENT_ID` and `AGENTID_CLIENT_SECRET` turn sign-in on.
    /// `AGENTID_REDIRECT_URI` overrides `{base_url}/v1/agentid/callback`,
    /// `AGENTID_DEFAULT_CHANNEL` names the channel the directory's
    /// initiate-login link signs in to, and `AGENTID_OWNER_SCOPES=true` also
    /// requests the owner's name and email.
    fn from_env(base_url: &str, frontend_url: &str) -> Option<Self> {
        let var = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let client_id = var("AGENTID_CLIENT_ID")?;
        let client_secret = ClientSecret(var("AGENTID_CLIENT_SECRET")?);
        Some(Self {
            redirect_uri: var("AGENTID_REDIRECT_URI").unwrap_or_else(|| {
                format!("{}/v1/agentid/callback", base_url.trim_end_matches('/'))
            }),
            owner_scopes: var("AGENTID_OWNER_SCOPES").is_some_and(|v| v == "true" || v == "1"),
            default_channel: var("AGENTID_DEFAULT_CHANNEL"),
            frontend_url: frontend_url.trim_end_matches('/').to_string(),
            issuer_base: AGENTID_ISSUER.to_string(),
            client_id,
            client_secret,
        })
    }

    fn scope(&self) -> String {
        if self.owner_scopes {
            format!("{SCOPES} {OWNER_SCOPES}")
        } else {
            SCOPES.to_string()
        }
    }

    fn authorize_url(
        &self,
        state: &str,
        challenge: &str,
        nonce: &str,
        hint: Option<&str>,
    ) -> String {
        let mut url = url::Url::parse(&format!("{}/v0/authorize", self.issuer_base))
            .expect("AgentID authorize URL is valid");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("scope", &self.scope())
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
        if let Some(hint) = hint {
            url.query_pairs_mut().append_pair("login_hint", hint);
        }
        url.into()
    }

    fn page_url(&self, channel_id: &str, fragment: &str) -> String {
        format!(
            "{}/public-chat/{}#{fragment}",
            self.frontend_url,
            urlencoding::encode(channel_id)
        )
    }
}

/// Process-wide config; the environment does not change while running.
fn login_config(state: &AgUiState) -> Option<Arc<AgentIdLoginConfig>> {
    static CONFIG: OnceLock<Option<Arc<AgentIdLoginConfig>>> = OnceLock::new();
    let auth = state.runtime_auth.as_ref()?;
    CONFIG
        .get_or_init(|| {
            AgentIdLoginConfig::from_env(&auth.config.base_url, &auth.config.frontend_url)
                .map(Arc::new)
        })
        .clone()
}

/// Whether a Public Chat page should offer "Continue with AgentID".
pub(crate) fn browser_sign_in_available(state: &AgUiState, auth: &ChannelAuthConfig) -> bool {
    auth.is_agentid() && login_config(state).is_some()
}

pub fn routes(state: AgUiState) -> Router {
    Router::new()
        .route("/v1/agentid/initiate-login", get(initiate_login))
        .route("/v1/agentid/callback", get(callback))
        .route(
            "/v1/channels/{channel_id}/public-chat/agentid/login",
            get(start_login),
        )
        .route(
            "/v1/e/{channel_id}/public-chat/agentid/login",
            get(start_login),
        )
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct StartQuery {
    login_hint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InitiateQuery {
    iss: Option<String>,
    login_hint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    iss: Option<String>,
    error: Option<String>,
}

async fn start_login(
    State(state): State<AgUiState>,
    Path(channel_id): Path<String>,
    Query(query): Query<StartQuery>,
) -> Response {
    let Some(config) = login_config(&state) else {
        return text(StatusCode::NOT_FOUND, "AgentID sign-in is not enabled.");
    };
    begin(&state, &config, &channel_id, query.login_hint).await
}

/// The URL listed in the AgentID directory. AgentID appends `iss` and
/// `login_hint`; neither names a channel, so only a deployment default
/// channel can turn this into a sign-in.
async fn initiate_login(
    State(state): State<AgUiState>,
    Query(query): Query<InitiateQuery>,
) -> Response {
    let Some(config) = login_config(&state) else {
        return text(StatusCode::NOT_FOUND, "AgentID sign-in is not enabled.");
    };
    if query.iss.as_deref().map(|iss| iss.trim_end_matches('/')) != Some(AGENTID_ISSUER) {
        return text(StatusCode::BAD_REQUEST, "Unknown sign-in issuer.");
    }
    let Some(channel_id) = config.default_channel.clone() else {
        return text(
            StatusCode::NOT_FOUND,
            "This sign-in link does not name an agent. Open the agent's chat page and choose Continue with AgentID.",
        );
    };
    begin(&state, &config, &channel_id, query.login_hint).await
}

async fn begin(
    state: &AgUiState,
    config: &AgentIdLoginConfig,
    channel_id: &str,
    login_hint: Option<String>,
) -> Response {
    let (context, channel) = match sign_in_channel(state, channel_id).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    let login_hint = login_hint
        .map(|hint| hint.trim().to_string())
        .filter(|hint| !hint.is_empty() && hint.len() <= MAX_LOGIN_HINT);
    let state_value = random_token();
    let verifier = random_token();
    let nonce = random_token();
    let login = AgentIdLoginState {
        org_id: context.org_id,
        channel_id: channel.public_id.to_string(),
        code_verifier: verifier.clone(),
        nonce: nonce.clone(),
        login_hint: login_hint.clone(),
    };
    if let Err(error) = state
        .db
        .create_agentid_login_state(&hash(&state_value), &login)
        .await
    {
        tracing::error!(error = %error, "AgentID sign-in state could not be stored");
        return text(StatusCode::INTERNAL_SERVER_ERROR, "Sign-in is unavailable.");
    }
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    no_store(
        Redirect::to(&config.authorize_url(
            &state_value,
            &challenge,
            &nonce,
            login_hint.as_deref(),
        ))
        .into_response(),
    )
}

async fn callback(State(state): State<AgUiState>, Query(query): Query<CallbackQuery>) -> Response {
    let (Some(config), Some(auth)) = (login_config(&state), state.runtime_auth.clone()) else {
        return text(StatusCode::NOT_FOUND, "AgentID sign-in is not enabled.");
    };
    let Some(state_value) = query.state.as_deref().filter(|s| !s.is_empty()) else {
        return text(StatusCode::BAD_REQUEST, "This sign-in link is not valid.");
    };
    let login = match state
        .db
        .consume_agentid_login_state(&hash(state_value))
        .await
    {
        Ok(Some(login)) => login,
        Ok(None) => {
            return text(
                StatusCode::BAD_REQUEST,
                "This sign-in has expired or was already used. Start again from the chat page.",
            );
        }
        Err(error) => {
            tracing::error!(error = %error, "AgentID sign-in state could not be read");
            return text(StatusCode::INTERNAL_SERVER_ERROR, "Sign-in is unavailable.");
        }
    };
    let fail = |code: &str| {
        no_store(
            Redirect::to(&config.page_url(&login.channel_id, &format!("agentid_error={code}")))
                .into_response(),
        )
    };
    if query.error.is_some() {
        return fail("denied");
    }
    // RFC 9207: the response names its issuer, which defeats mix-up.
    if query.iss.as_deref() != Some(AGENTID_ISSUER) {
        return fail("failed");
    }
    let Some(code) = query.code.as_deref().filter(|c| !c.is_empty()) else {
        return fail("failed");
    };
    // The channel may have changed since the sign-in started.
    let channel = match sign_in_channel(&state, &login.channel_id).await {
        Ok((context, channel)) if context.org_id == login.org_id => channel,
        _ => return fail("unavailable"),
    };
    let jwks = match state.auth_verifier.agentid_jwks().await {
        Ok(jwks) => jwks,
        Err(_) => return fail("failed"),
    };
    let agent = match finish(&config, &http_client(), &jwks, code, &login).await {
        Ok(agent) => agent,
        Err(SignInError::MissingOwner) => return fail("owner_required"),
        Err(SignInError::Rejected) => return fail("failed"),
    };
    let user = match state.db.agentid_sign_in(login.org_id, agent.clone()).await {
        Ok(AgentIdSignIn::SignedIn(user)) => *user,
        Ok(AgentIdSignIn::OwnerCapReached) => return fail("owner_limit"),
        Err(error) => {
            tracing::error!(error = %error, "AgentID sign-in could not resolve its account");
            return fail("failed");
        }
    };
    if user.status != "active" {
        return fail("unavailable");
    }
    let binding = match state
        .db
        .list_virtual_user_bindings(login.org_id, user.id)
        .await
    {
        Ok(bindings) => bindings.into_iter().find(|b| {
            b.provider == crate::api::channel_auth::AGENTID_PROVIDER
                && b.realm == AGENTID_ISSUER
                && b.subject == agent.subject
                && b.status == "active"
        }),
        Err(_) => None,
    };
    let Some(binding) = binding else {
        return fail("unavailable");
    };
    let channel_id = channel.public_id.to_string();
    let token = match crate::auth::jwt::JwtService::new(auth.config.jwt.clone())
        .generate_runtime_token(login.org_id, user.id, channel_id.clone(), binding.id)
    {
        Ok(token) => token,
        Err(_) => return fail("failed"),
    };
    // The fragment never reaches a server; the page moves the token into
    // session storage and clears it from the address bar.
    no_store(
        Redirect::to(&config.page_url(
            &channel_id,
            &format!("agentid_token={token}&expires_in=900"),
        ))
        .into_response(),
    )
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SignInError {
    /// Any failed exchange or check. Deliberately one variant: the browser
    /// learns nothing about which check failed.
    Rejected,
    /// The token carried no `owner_sub`, so the per-owner cap cannot apply.
    MissingOwner,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    id_token: String,
    access_token: String,
}

/// Exchange the code, verify the id_token (signature first, then `nonce`,
/// `actor_type`), and read `/v0/userinfo` for the same subject.
pub(crate) async fn finish(
    config: &AgentIdLoginConfig,
    http: &reqwest::Client,
    jwks: &JwkSet,
    code: &str,
    login: &AgentIdLoginState,
) -> Result<AgentIdAgent, SignInError> {
    let form = serde_urlencoded::to_string([
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("code_verifier", login.code_verifier.as_str()),
        ("client_id", config.client_id.as_str()),
        ("client_secret", config.client_secret.0.as_str()),
    ])
    .map_err(|_| SignInError::Rejected)?;
    let tokens: TokenResponse = http
        .post(format!("{}/v0/token", config.issuer_base))
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::ACCEPT, "application/json")
        .body(form)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|_| SignInError::Rejected)?
        .json()
        .await
        .map_err(|_| SignInError::Rejected)?;
    let requirements = ChannelAuthConfig::agentid_preset(&config.client_id).requirements;
    let claims = verify_agentid_claims(&tokens.id_token, jwks, &requirements)
        .map_err(|_| SignInError::Rejected)?;
    if claims.get("nonce").and_then(Value::as_str) != Some(login.nonce.as_str()) {
        return Err(SignInError::Rejected);
    }
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .ok_or(SignInError::Rejected)?
        .to_string();
    let userinfo: Value = http
        .get(format!("{}/v0/userinfo", config.issuer_base))
        .bearer_auth(&tokens.access_token)
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|_| SignInError::Rejected)?
        .json()
        .await
        .map_err(|_| SignInError::Rejected)?;
    if userinfo.get("sub").and_then(Value::as_str) != Some(subject.as_str()) {
        return Err(SignInError::Rejected);
    }
    let claim = |name: &str| {
        userinfo
            .get(name)
            .or_else(|| claims.get(name))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let owner_sub = claim("owner_sub").ok_or(SignInError::MissingOwner)?;
    // The agent's own name, never the owner's.
    let display_name = claim("name")
        .or_else(|| claim("preferred_username"))
        .or_else(|| claim("email").and_then(|email| email.split('@').next().map(str::to_string)))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "AgentID agent".to_string())
        .chars()
        .take(MAX_DISPLAY_NAME)
        .collect();
    Ok(AgentIdAgent {
        subject,
        owner_sub,
        owner_email: if config.owner_scopes {
            claim("owner_email")
        } else {
            None
        },
        display_name,
    })
}

/// A live Public Chat channel whose own auth is the AgentID preset.
async fn sign_in_channel(
    state: &AgUiState,
    channel_id: &str,
) -> Result<(IngressContext, IngressChannel), Response> {
    let unavailable = || {
        text(
            StatusCode::NOT_FOUND,
            "This chat does not offer AgentID sign-in.",
        )
    };
    if !state.public_chat_enabled {
        return Err(unavailable());
    }
    let (context, channel) = crate::api::channel_ingress::resolve_channel(
        &state.db,
        state.encryption.as_ref(),
        channel_id,
    )
    .await
    .map_err(|_| text(StatusCode::INTERNAL_SERVER_ERROR, "Sign-in is unavailable."))?
    .ok_or_else(unavailable)?;
    if channel.channel_type != ChannelType::PublicChat
        || crate::api::channel_ingress::channel_liveness(&context, &channel).is_err()
    {
        return Err(unavailable());
    }
    let auth = channel
        .auth
        .as_deref()
        .cloned()
        .or_else(|| channel.public_chat_config().and_then(|c| c.auth));
    if !auth.is_some_and(|auth| auth.is_agentid()) {
        return Err(unavailable());
    }
    Ok((context, channel))
}

fn http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default()
        })
        .clone()
}

fn random_token() -> String {
    use rand::RngExt;
    let bytes: [u8; 32] = rand::rng().random();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hash(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}

fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn text(status: StatusCode, message: &'static str) -> Response {
    no_store((status, message).into_response())
}

#[cfg(test)]
mod tests;
