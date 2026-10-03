//! Open-source Sign in with ChatGPT, independent of host UI and persistence.
use super::{ChatGptRegistration, CodexAuth, OpenSourceGrant};
use anyhow::{Context, Result, anyhow, bail};
use rand::RngExt;
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
pub const DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";
pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
pub const SCOPE: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
pub const RESOURCE: &str = "https://api.openai.com/v1";
const CLOCK_SKEW_SECS: i64 = 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
pub fn now_epoch_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
pub fn validate_client_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 512
        || value == DYNAMIC_CLIENT_ID
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        bail!("invalid issued OAuth client ID");
    }
    Ok(())
}
fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(HTTP_TIMEOUT)
        .build()
        .context("build ChatGPT auth client")
}
/// OpenAI's auth endpoints. Tests point these at a local mock.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub issuer: String,
    pub authorize: String,
    pub token: String,
    pub revoke: String,
    pub jwks: String,
}

impl Endpoints {
    /// Production values from
    /// `https://auth.openai.com/.well-known/openid-configuration`.
    pub fn production() -> Self {
        Self {
            issuer: "https://auth.openai.com".to_string(),
            authorize: "https://auth.openai.com/api/accounts/authorize".to_string(),
            token: "https://auth.openai.com/api/accounts/oauth/token".to_string(),
            revoke: "https://auth.openai.com/api/accounts/oauth/revoke".to_string(),
            jwks: "https://auth.openai.com/.well-known/jwks.json".to_string(),
        }
    }
}

/// A fresh `urn:uuid:` host ID (UUIDv4), one of the formats the docs accept.
pub fn new_host_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Inputs for one authorization request.
#[derive(Clone)]
pub struct AuthorizeParams<'a> {
    pub agent_name: &'a str,
    pub redirect_uri: &'a str,
    pub host_id: &'a str,
    /// The saved registration, for a returning sign-in. `None` registers.
    pub registration: Option<&'a ChatGptRegistration>,
    /// The retained ID token of that registration's last sign-in.
    pub id_token_hint: Option<&'a str>,
    /// Ask for consent again, after the plan scope was declined.
    pub force_consent: bool,
    pub state: &'a str,
    pub nonce: &'a str,
    pub code_challenge: &'a str,
}

/// The browser URL for one attempt: a new registration with
/// `dynamic_agent_client` and `agent_name_hint`, or a returning sign-in with
/// the saved client and account hints.
pub fn authorize_url(endpoints: &Endpoints, params: &AuthorizeParams<'_>) -> Result<Url> {
    let mut url = Url::parse(&endpoints.authorize).context("parse authorize endpoint")?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        match params.registration {
            Some(registration) => {
                query.append_pair("client_id", &registration.client_id);
                if let Some(hint) = params.id_token_hint {
                    query.append_pair("id_token_hint", hint);
                }
                if let Some(email) = &registration.email {
                    query.append_pair("login_hint", email);
                }
            }
            None => {
                query.append_pair("client_id", DYNAMIC_CLIENT_ID);
                query.append_pair("agent_name_hint", params.agent_name);
            }
        }
        query
            .append_pair("ext_agent_host_id", params.host_id)
            .append_pair("redirect_uri", params.redirect_uri)
            .append_pair("scope", SCOPE)
            .append_pair("resource", RESOURCE)
            .append_pair("state", params.state)
            .append_pair("nonce", params.nonce)
            .append_pair("code_challenge", params.code_challenge)
            .append_pair("code_challenge_method", "S256");
        if params.force_consent {
            query.append_pair("prompt", "consent");
        }
    }
    Ok(url)
}

/// The client the code exchange uses. A new registration must come back with
/// an issued client; a returning sign-in may omit it, but must not change it.
pub fn issued_client_id(pending: Option<&str>, callback: Option<&str>) -> Result<String> {
    let callback = callback.map(str::trim).filter(|value| !value.is_empty());
    match (pending, callback) {
        (None, None) => bail!("ChatGPT registration did not return a client ID"),
        (None, Some(DYNAMIC_CLIENT_ID)) => {
            bail!("ChatGPT registration returned the registration entrypoint, not a client ID")
        }
        (None, Some(issued)) => {
            validate_client_id(issued)
                .context("ChatGPT registration returned an invalid client ID")?;
            Ok(issued.to_string())
        }
        (Some(saved), Some(returned)) if returned != saved => bail!(
            "ChatGPT sign-in returned client `{returned}`, not the saved `{saved}`; refusing to mix registrations"
        ),
        (Some(saved), _) => {
            validate_client_id(saved)?;
            Ok(saved.to_string())
        }
    }
}

/// The token endpoint's response for this route.
#[derive(Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

impl TokenResponse {
    pub fn scopes(&self) -> Vec<String> {
        split_scopes(self.scope.as_deref())
    }
}

fn split_scopes(scope: Option<&str>) -> Vec<String> {
    scope
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

pub fn grants_plan_usage(scopes: &[String]) -> bool {
    scopes.iter().any(|scope| scope == PLAN_SCOPE)
}

#[derive(Debug)]
pub struct TokenHttpError {
    pub status: u16,
}
impl std::fmt::Display for TokenHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ChatGPT token endpoint failed ({})", self.status)
    }
}
impl std::error::Error for TokenHttpError {}

async fn post_token_form(
    token_url: &str,
    form: &[(&str, &str)],
    what: &str,
) -> Result<TokenResponse> {
    let response = http_client()?
        .post(token_url)
        .form(form)
        .send()
        .await
        .with_context(|| format!("{what} ChatGPT token"))?;
    if !response.status().is_success() {
        return Err(TokenHttpError {
            status: response.status().as_u16(),
        }
        .into());
    }
    response
        .json()
        .await
        .with_context(|| format!("parse ChatGPT {what} response"))
}

/// Exchange an authorization code with the issued client (never
/// `dynamic_agent_client`), the same redirect URI, and the same resource.
pub async fn exchange_code(
    token_url: &str,
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse> {
    validate_client_id(client_id)?;
    post_token_form(
        token_url,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
            ("resource", RESOURCE),
        ],
        "exchange",
    )
    .await
}

/// Refresh with the client saved on the token set. `scope` is omitted so the
/// grant is kept as it is.
pub async fn refresh_with_token_at(
    token_url: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<CodexAuth> {
    validate_client_id(client_id)?;
    let token = post_token_form(
        token_url,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("resource", RESOURCE),
        ],
        "refresh",
    )
    .await?;
    let scopes = token.scopes();
    Ok(CodexAuth {
        expires_at: expires_at(token.expires_in),
        account_id: None,
        email: None,
        client_id: Some(client_id.to_string()),
        open_source: Some(OpenSourceGrant {
            id_token: token.id_token,
            scopes,
            subject: None,
        }),
        access_token: token.access_token,
        refresh_token: token.refresh_token,
    })
}

/// End the renewable session at sign-out. An empty `200` is success, also for
/// a token that was already invalid.
pub async fn revoke_refresh_token(client_id: &str, refresh_token: &str) -> Result<()> {
    revoke_refresh_token_at(&Endpoints::production().revoke, client_id, refresh_token).await
}

pub async fn revoke_refresh_token_at(
    revoke_url: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<()> {
    let response = http_client()?
        .post(revoke_url)
        .form(&[
            ("token", refresh_token),
            ("token_type_hint", "refresh_token"),
            ("client_id", client_id),
        ])
        .send()
        .await
        .context("revoke ChatGPT session")?;
    if response.status().is_success() {
        return Ok(());
    }
    let status = response.status();
    bail!("ChatGPT session revocation failed ({status})")
}

fn expires_at(expires_in: Option<i64>) -> Option<i64> {
    expires_in
        .filter(|seconds| *seconds > 0)
        .map(|seconds| now_epoch_millis().saturating_add(seconds.saturating_mul(1000)))
}

// ---------------------------------------------------------------------------
// ID-token validation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Jwk {
    #[serde(default)]
    pub kid: Option<String>,
    pub kty: String,
    #[serde(default, rename = "use")]
    pub key_use: Option<String>,
    #[serde(default)]
    pub alg: Option<String>,
    #[serde(default)]
    pub n: Option<String>,
    #[serde(default)]
    pub e: Option<String>,
}

pub async fn fetch_jwks(jwks_url: &str) -> Result<Jwks> {
    let response = http_client()?
        .get(jwks_url)
        .send()
        .await
        .context("fetch OpenAI signing keys")?;
    if !response.status().is_success() {
        bail!(
            "fetching OpenAI signing keys failed ({})",
            response.status()
        );
    }
    response.json().await.context("parse OpenAI signing keys")
}

/// The identity a validated ID token carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub subject: String,
    pub email: Option<String>,
}

/// Verify an ID token: RS256 signature against `jwks`, then issuer, audience
/// (the issued client), expiry, and the nonce of this attempt.
// THREAT[TM-AUTH-033]: Accept identity only after cryptographic and attempt-bound validation.
pub fn validate_id_token(
    id_token: &str,
    jwks: &Jwks,
    issuer: &str,
    client_id: &str,
    nonce: &str,
    now_secs: i64,
) -> Result<Identity> {
    let mut parts = id_token.split('.');
    let (Some(header_b64), Some(payload_b64), Some(_signature_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        bail!("ID token is not a JWS compact token");
    };
    let decode = |segment: &str| {
        base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, segment)
            .context("ID token segment is not base64url")
    };
    let header: Value = serde_json::from_slice(&decode(header_b64)?).context("ID token header")?;
    if header.get("alg").and_then(Value::as_str) != Some("RS256") {
        bail!("ID token is not signed with RS256");
    }
    let kid = header.get("kid").and_then(Value::as_str);
    let mut keys = jwks
        .keys
        .iter()
        .filter(|key| key.kty == "RSA")
        .filter(|key| key.key_use.as_deref().is_none_or(|v| v == "sig"))
        .filter(|key| key.alg.as_deref().is_none_or(|v| v == "RS256"))
        .filter(|key| kid.is_none() || key.kid.as_deref() == kid);
    let key = keys
        .next()
        .ok_or_else(|| anyhow!("no OpenAI signing key matches the ID token"))?;
    anyhow::ensure!(keys.next().is_none(), "ambiguous OpenAI signing key");
    let (Some(n), Some(e)) = (key.n.as_deref(), key.e.as_deref()) else {
        bail!("OpenAI signing key is missing its modulus or exponent");
    };
    let key = jsonwebtoken::DecodingKey::from_rsa_components(n, e)
        .context("OpenAI signing key is invalid")?;
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[client_id]);
    validation.leeway = CLOCK_SKEW_SECS as u64;
    // The caller's clock is used below as well, allowing deterministic tests.
    validation.validate_exp = false;
    jsonwebtoken::decode::<Value>(id_token, &key, &validation)
        .context("ID token signature does not verify")?;

    let claims: Value = serde_json::from_slice(&decode(payload_b64)?).context("ID token claims")?;
    if claims.get("iss").and_then(Value::as_str) != Some(issuer) {
        bail!("ID token issuer is not {issuer}");
    }
    let audience_ok = match claims.get("aud") {
        Some(Value::String(aud)) => aud == client_id,
        Some(Value::Array(auds)) => auds.iter().any(|aud| aud.as_str() == Some(client_id)),
        _ => false,
    };
    if claims
        .get("azp")
        .and_then(Value::as_str)
        .is_some_and(|party| party != client_id)
    {
        bail!("ID token authorized party does not match the issuing client");
    }
    if claims
        .get("nbf")
        .and_then(Value::as_i64)
        .is_some_and(|nbf| nbf > now_secs.saturating_add(CLOCK_SKEW_SECS))
    {
        bail!("ID token is not yet valid");
    }
    if !audience_ok {
        bail!("ID token was not issued to client {client_id}");
    }
    let exp = claims
        .get("exp")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("ID token has no expiry"))?;
    if exp.saturating_add(CLOCK_SKEW_SECS) < now_secs {
        bail!("ID token has expired");
    }
    if claims.get("nonce").and_then(Value::as_str) != Some(nonce) {
        bail!("ID token nonce does not match this sign-in");
    }
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|sub| !sub.is_empty())
        .ok_or_else(|| anyhow!("ID token has no subject"))?
        .to_string();
    let email = claims
        .get("email")
        .and_then(Value::as_str)
        .or_else(|| {
            claims
                .get("https://api.openai.com/profile")
                .and_then(|profile| profile.get("email"))
                .and_then(Value::as_str)
        })
        .filter(|email| !email.is_empty())
        .map(str::to_string);
    Ok(Identity { subject, email })
}
