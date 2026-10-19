// Poppy Sessions and Session Tokens (spec 4.2, 4.3, 4.9): the token
// endpoint starts and renews Sessions with the JWT bearer grant, and every
// conversation request is authenticated here.
//
// Design Decisions:
// - The personal agent authenticates at the token and revocation endpoints
//   with `private_key_jwt` (its client assertion), and the Session assertion
//   must be signed by the same agent (`iss` = `client_id`). A client
//   assertion is never accepted as the Session assertion: their `jti`s share
//   one replay scope.
// - Only signed-out Sessions exist until sign-in lands, so the token
//   endpoint takes only the JWT bearer grant, and revocation has no Account
//   Tokens to revoke (RFC 7009 answers `200` for unknown tokens).
// - A Session ends after `SESSION_IDLE_DAYS` without a new token. Ended
//   Sessions answer `invalid_session`, and their tokens `invalid_token`.
// THREAT[TM-POPPY-001, TM-POPPY-003, TM-POPPY-004].

use std::collections::HashMap;

use axum::{
    body::Bytes,
    extract::{OriginalUri, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use base64::Engine as _;
use chrono::Utc;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::client::{self, Audience};
use super::{Channel, Peer, PoppyState, Urls, dpop, error, internal, json_response};
use crate::channels::a2a::pact_keys::ProviderKey;
use crate::storage::poppy::PoppySessionRow;

const JWT_BEARER_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
const CLIENT_ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";
/// `typ` of a Session Token.
const SESSION_TOKEN_TYPE: &str = "poppy-session+jwt";
/// Session Tokens "SHOULD last hours, not days" (spec 4.2).
pub(super) const SESSION_TOKEN_TTL_SECS: i64 = 3600;
/// A Session with no new token for this long has ended.
const SESSION_IDLE_DAYS: i64 = 7;

/// The claims of a Session Token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SessionClaims {
    pub iss: String,
    pub aud: String,
    /// The personal agent's User ID.
    pub sub: String,
    pub client_id: String,
    /// The Poppy Session.
    pub sid: String,
    pub scope: String,
    pub cnf: Confirmation,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
}

/// RFC 9449 §6: the DPoP key the token is bound to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Confirmation {
    pub jkt: String,
}

/// A form body (`application/x-www-form-urlencoded`). A parameter given twice
/// counts as missing (RFC 6749 §3.1).
struct Form(HashMap<String, Option<String>>);

impl Form {
    fn parse(body: &[u8]) -> Self {
        let mut params: HashMap<String, Option<String>> = HashMap::new();
        for (key, value) in url::form_urlencoded::parse(body) {
            params
                .entry(key.into_owned())
                .and_modify(|seen| *seen = None)
                .or_insert(Some(value.into_owned()));
        }
        Self(params)
    }

    fn one(&self, name: &str) -> Option<&str> {
        self.0
            .get(name)?
            .as_deref()
            .filter(|value| !value.is_empty())
    }
}

fn random_id(prefix: &str) -> String {
    let bytes: [u8; 16] = rand::rng().random();
    format!(
        "{prefix}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

fn now() -> i64 {
    Utc::now().timestamp()
}

/// The `401` for a client the channel does not accept (spec 4.1).
fn invalid_client() -> Response {
    error(
        StatusCode::UNAUTHORIZED,
        "invalid_client",
        "The personal agent's metadata or client assertion does not check out, or this channel does not accept it",
    )
}

/// An authenticated personal agent at the token or revocation endpoint.
struct Admitted {
    channel: Channel,
    client: client::Client,
    urls: Urls,
}

/// Resolve the channel, apply its rate limit, and authenticate the client.
async fn admit_client(
    state: &PoppyState,
    channel_id: &str,
    headers: &HeaderMap,
    path: &str,
    peer: Peer,
    form: &Form,
) -> Result<Admitted, Response> {
    let channel = super::channel(state, channel_id).await?;
    super::rate_limit(state, &channel, headers, peer).await?;
    let urls = Urls::from_request(headers, path, channel_id);
    let Some(client_id) = form.one("client_id") else {
        return Err(invalid_client());
    };
    if form.one("client_assertion_type") != Some(CLIENT_ASSERTION_TYPE)
        || !channel.config.admits(client_id)
    {
        return Err(invalid_client());
    }
    let client = client::load(state, client_id)
        .await
        .map_err(|()| invalid_client())?;
    let token_url = urls.token();
    let allowed = [token_url.as_str(), urls.issuer.as_str()];
    let assertion = form
        .one("client_assertion")
        .and_then(|token| {
            client::check_assertion(token, &client, Audience::AnyOf(&allowed), now()).ok()
        })
        .ok_or_else(invalid_client)?;
    if !first_use(state, &channel, &client.client_id, &assertion.jti).await {
        return Err(invalid_client());
    }
    Ok(Admitted {
        channel,
        client,
        urls,
    })
}

/// Record an assertion `jti`; `false` when it was used before.
async fn first_use(state: &PoppyState, channel: &Channel, client_id: &str, jti: &str) -> bool {
    state
        .replay_store
        .try_record(
            &format!("poppy-assertion:{}:{client_id}", channel.channel.public_id),
            jti,
        )
        .await
}

/// `POST {issuer}/oauth/token`.
pub(super) async fn token(
    State(state): State<PoppyState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    peer: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let form = Form::parse(&body);
    let admitted = match admit_client(&state, &channel_id, &headers, uri.path(), peer, &form).await
    {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    let Admitted {
        channel,
        client,
        urls,
    } = admitted;
    if form.one("grant_type") != Some(JWT_BEARER_GRANT_TYPE) {
        return error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "This Company offers only signed-out Sessions: use the JWT bearer grant",
        );
    }
    let invalid_grant = |description| error(StatusCode::BAD_REQUEST, "invalid_grant", description);
    let token_url = urls.token();
    let Some(assertion) = form.one("assertion").and_then(|token| {
        client::check_assertion(token, &client, Audience::Exactly(&token_url), now()).ok()
    }) else {
        return invalid_grant("The assertion is missing, expired or malformed");
    };
    if !first_use(&state, &channel, &client.client_id, &assertion.jti).await {
        return invalid_grant("The assertion was already used");
    }
    if form.one("resource").is_some() {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_target",
            "This Company lists no resource that needs its own token",
        );
    }
    if form.one("scope").is_some() {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "A signed-out Session has no scopes",
        );
    }
    // Spec 4.3: every token request carries a proof, except a Bearer request
    // for an MCP API, and this Company lists none.
    let proof = match proof_for(&state, &channel, &headers, "POST", &token_url, None).await {
        Ok(proof) => proof,
        Err(response) => return response,
    };
    let session = match form.one("session_id") {
        Some(session_id) => match state.db.poppy_session(session_id).await {
            Ok(Some(row))
                if row.channel_id == channel.channel.internal_id
                    && row.client_id == client.client_id
                    && row.user_id == assertion.subject
                    && is_live(&row) =>
            {
                if let Err(err) = state.db.renew_poppy_session(&row.id).await {
                    return internal(err);
                }
                row
            }
            Ok(_) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid_session",
                    "The Session has ended or belongs to another personal agent or User. Start a new Session.",
                );
            }
            Err(err) => return internal(err),
        },
        None => {
            let row = PoppySessionRow {
                id: random_id("ses_"),
                channel_id: channel.channel.internal_id,
                client_id: client.client_id.clone(),
                user_id: assertion.subject.clone(),
                account: None,
                scope: String::new(),
                created_at: Utc::now(),
                renewed_at: Utc::now(),
                ended_at: None,
            };
            if let Err(err) = state.db.create_poppy_session(&row).await {
                return internal(err);
            }
            row
        }
    };
    let key = match ProviderKey::for_channel(
        &state.db,
        state.encryption.as_ref(),
        channel.channel.internal_id,
    )
    .await
    {
        Ok(key) => key,
        Err(err) => return internal(err),
    };
    let issued_at = now();
    let claims = SessionClaims {
        iss: urls.issuer.clone(),
        aud: urls.issuer.clone(),
        sub: session.user_id.clone(),
        client_id: session.client_id.clone(),
        sid: session.id.clone(),
        scope: String::new(),
        cnf: Confirmation { jkt: proof },
        iat: issued_at,
        exp: issued_at + SESSION_TOKEN_TTL_SECS,
        jti: random_id(""),
    };
    let access_token = match key.sign(SESSION_TOKEN_TYPE, &claims) {
        Ok(token) => token,
        Err(err) => return internal(err),
    };
    json_response(
        StatusCode::OK,
        &json!({
            "access_token": access_token,
            "token_type": "DPoP",
            "expires_in": SESSION_TOKEN_TTL_SECS,
            "scope": "",
            "session_id": session.id,
            "signed_in": false,
        }),
    )
}

/// `POST {issuer}/oauth/revoke` (RFC 7009). With no Account Tokens yet,
/// every authenticated request succeeds without effect.
pub(super) async fn revoke(
    State(state): State<PoppyState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    peer: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let form = Form::parse(&body);
    if let Err(response) =
        admit_client(&state, &channel_id, &headers, uri.path(), peer, &form).await
    {
        return response;
    }
    if form.one("token").is_none() {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "token is required",
        );
    }
    json_response(StatusCode::OK, &json!({}))
}

fn is_live(row: &PoppySessionRow) -> bool {
    row.ended_at.is_none()
        && row.renewed_at > Utc::now() - chrono::Duration::days(SESSION_IDLE_DAYS)
}

/// Check the request's DPoP proof and record its `jti`. Returns the key's
/// thumbprint, or the finished error (`400` at the token endpoint, `401`
/// elsewhere, both with `WWW-Authenticate: DPoP`).
async fn proof_for(
    state: &PoppyState,
    channel: &Channel,
    headers: &HeaderMap,
    method: &str,
    url: &str,
    access_token: Option<&str>,
) -> Result<String, Response> {
    let status = match access_token {
        Some(_) => StatusCode::UNAUTHORIZED,
        None => StatusCode::BAD_REQUEST,
    };
    let reject = |description: &str| dpop_error(status, "invalid_dpop_proof", description);
    let mut proofs = headers.get_all("dpop").iter();
    let (Some(proof), None) = (proofs.next(), proofs.next()) else {
        return Err(reject("Send exactly one DPoP proof"));
    };
    let proof = proof.to_str().map_err(|_| reject("Invalid DPoP proof"))?;
    let proof = dpop::check(proof, method, url, access_token, now()).map_err(reject)?;
    if !state
        .replay_store
        .try_record(
            &format!("poppy-dpop:{}", channel.channel.public_id),
            &proof.jti,
        )
        .await
    {
        return Err(reject("The DPoP proof was already used"));
    }
    Ok(proof.jkt)
}

/// An error with the RFC 9449 `WWW-Authenticate: DPoP` challenge.
pub(super) fn dpop_error(status: StatusCode, code: &str, description: &str) -> Response {
    let challenge = format!("DPoP error=\"{code}\", algs=\"ES256 ES384 RS256 PS256 EdDSA\"");
    let mut response = error(status, code, description);
    if let Ok(value) = HeaderValue::from_str(&challenge) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

/// A conversation request whose Session Token and proof checked out.
pub(super) struct Authenticated {
    pub channel: Channel,
    pub session: PoppySessionRow,
}

/// Authenticate a request to a Poppy API (spec 4.3): resolve the channel,
/// then the `DPoP` Session Token and its proof, then the rate limit and the
/// Session itself. Every token failure is `401 invalid_token`.
pub(super) async fn authenticate(
    state: &PoppyState,
    channel_id: &str,
    method: &str,
    path: &str,
    headers: &HeaderMap,
    peer: Peer,
) -> Result<Authenticated, Response> {
    let channel = super::channel(state, channel_id).await?;
    let urls = Urls::from_request(headers, path, channel_id);
    let invalid_token =
        |description: &str| dpop_error(StatusCode::UNAUTHORIZED, "invalid_token", description);
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("dpop"))
        .map(|(_, token)| token.trim())
        .ok_or_else(|| invalid_token("Send a Session Token with the DPoP scheme"))?;
    let key = ProviderKey::for_channel(
        &state.db,
        state.encryption.as_ref(),
        channel.channel.internal_id,
    )
    .await
    .map_err(internal)?;
    let claims: SessionClaims = key
        .verify(SESSION_TOKEN_TYPE, token, Some(&urls.issuer))
        .ok_or_else(|| invalid_token("The Session Token is invalid or expired"))?;
    let url = super::request_url(headers, path);
    let jkt = proof_for(state, &channel, headers, method, &url, Some(token)).await?;
    if jkt != claims.cnf.jkt {
        return Err(dpop_error(
            StatusCode::UNAUTHORIZED,
            "invalid_dpop_proof",
            "The DPoP proof is not from the key the token is bound to",
        ));
    }
    super::rate_limit(state, &channel, headers, peer).await?;
    let session = match state
        .db
        .poppy_session(&claims.sid)
        .await
        .map_err(internal)?
    {
        Some(row)
            if row.channel_id == channel.channel.internal_id
                && row.client_id == claims.client_id
                && row.user_id == claims.sub
                && is_live(&row) =>
        {
            row
        }
        _ => return Err(invalid_token("The Session has ended")),
    };
    // THREAT[TM-POPPY-001]: an agent blocked after its token was issued is
    // refused at once.
    if !channel.config.admits(&session.client_id) {
        return Err(invalid_token(
            "This channel no longer accepts the personal agent",
        ));
    }
    Ok(Authenticated { channel, session })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_form_parameters_count_as_missing() {
        let form = Form::parse(b"grant_type=a&grant_type=b&assertion=x&scope=");
        assert_eq!(form.one("grant_type"), None);
        assert_eq!(form.one("assertion"), Some("x"));
        assert_eq!(form.one("scope"), None);
    }

    #[test]
    fn sessions_end_after_idle_days() {
        let mut row = PoppySessionRow {
            id: "ses_1".into(),
            channel_id: uuid::Uuid::now_v7(),
            client_id: "https://pa.example/agent.json".into(),
            user_id: "usr_1".into(),
            account: None,
            scope: String::new(),
            created_at: Utc::now(),
            renewed_at: Utc::now(),
            ended_at: None,
        };
        assert!(is_live(&row));
        row.renewed_at = Utc::now() - chrono::Duration::days(SESSION_IDLE_DAYS + 1);
        assert!(!is_live(&row));
        row.renewed_at = Utc::now();
        row.ended_at = Some(Utc::now());
        assert!(!is_live(&row));
    }
}
