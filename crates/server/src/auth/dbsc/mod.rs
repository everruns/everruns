//! Device Bound Session Credentials (W3C WebAppSec DBSC, shipped in Chrome)
//! for browser sign-ins.
//!
//! Decisions:
//! - The refresh-token row is the DBSC session. Binding stores the browser's
//!   public key on it and removes the 30-day `refresh_token` cookie from the
//!   browser. The bound credential is the existing `access_token` cookie: the
//!   browser renews it through `/v1/auth/dbsc/refresh` with a proof signed by
//!   the device key before it lets a request go out without one. A copied
//!   access token is good until it expires (the access-token lifetime, 15
//!   minutes by default); a copied refresh token is refused once bound.
//! - `/v1/auth/refresh` never accepts a bound row, so the refresh token taken
//!   before binding cannot be exchanged after it.
//! - The registration challenge is derived from the refresh token the response
//!   just issued, so login paths need no extra storage; registration also
//!   requires that token's cookie, and a row binds once. Refresh challenges are
//!   random, single-use and stored on the row.
//! - Registration is offered by `offer_registration`, a response layer that
//!   sees every new `refresh_token` cookie, so every sign-in path (password,
//!   register, OAuth callback, a wrapper's own OIDC callback) is covered. It
//!   writes a path relative to the request, so a proxy that serves the API
//!   under a prefix (the UI's `/api`) still resolves it.
//! - A small `dbsc_session` cookie names the bound row so sign-out can revoke
//!   it: the browser no longer holds the refresh token. The identifier alone
//!   cannot refresh anything.
//! - Browsers without DBSC never call the endpoints and keep today's cookies.
//!   `none` is never offered. Proofs must be typed `dbsc+jwt` (spec) or `jwt`
//!   (earlier Chrome builds); the challenge in `jti` is what binds a proof to
//!   this server.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header::SET_COOKIE},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation,
    jwk::{AlgorithmParameters, EllipticCurve, Jwk},
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::builtin::BuiltinAuthBackend;
use super::jwt::hash_token;

pub const REGISTER_PATH: &str = "/v1/auth/dbsc/start";
pub const REFRESH_PATH: &str = "/v1/auth/dbsc/refresh";
/// Names the bound refresh-token row, for sign-out.
pub const SESSION_COOKIE: &str = "dbsc_session";
/// The bound credential. Its attributes match the login cookie.
const BOUND_COOKIE: &str = "access_token";
const BOUND_COOKIE_ATTRIBUTES: &str = "Path=/; HttpOnly; Secure; SameSite=Lax";
/// A challenge handed out with one refresh is answered at the next, one
/// access-token lifetime later.
const CHALLENGE_TTL_SECS: i64 = 3600;
const MIN_RSA_MODULUS_BYTES: usize = 256;

pub const REGISTRATION_HEADER: &str = "secure-session-registration";
pub const CHALLENGE_HEADER: &str = "secure-session-challenge";
pub const RESPONSE_HEADER: &str = "secure-session-response";
pub const SESSION_ID_HEADER: &str = "sec-secure-session-id";

/// Challenge for the registration proof of a freshly issued refresh token.
pub fn registration_challenge(refresh_token: &str) -> String {
    hash_token(&format!("everruns dbsc registration\0{refresh_token}"))
}

/// `Secure-Session-Registration` value, its path relative to `request_path`.
pub fn registration_header(request_path: &str, challenge: &str) -> String {
    format!(
        "(ES256 RS256);path=\"{}\";challenge=\"{challenge}\"",
        relative_path(request_path, REGISTER_PATH)
    )
}

/// `Secure-Session-Challenge` value for a session.
pub fn challenge_header(challenge: &str, session_id: &str) -> String {
    format!("\"{challenge}\";id=\"{session_id}\"")
}

/// A path reference from the directory of `from` to `to`, both absolute.
/// Resolved by the browser against the URL it used, so any prefix a proxy
/// adds in front of both is preserved.
pub fn relative_path(from: &str, to: &str) -> String {
    let from_dir: Vec<&str> = from
        .rsplit_once('/')
        .map_or("", |(dir, _)| dir)
        .split('/')
        .collect();
    let target: Vec<&str> = to.split('/').collect();
    let common = from_dir
        .iter()
        .zip(&target)
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts = vec![".."; from_dir.len() - common];
    parts.extend(&target[common..]);
    parts.join("/")
}

/// Session instructions returned by registration and refresh. Paths are
/// relative to the registration URL, which shares the refresh URL's directory.
pub fn session_instructions(session_id: &str) -> Value {
    json!({
        "session_identifier": session_id,
        "refresh_url": relative_path(REGISTER_PATH, REFRESH_PATH),
        "scope": {
            "include_site": false,
            // Static UI assets never need the credential.
            "scope_specification": [{"type": "exclude", "path": "/_next/static"}],
        },
        "credentials": [
            {"type": "cookie", "name": BOUND_COOKIE, "attributes": BOUND_COOKIE_ATTRIBUTES},
        ],
        "allowed_refresh_initiators": [],
    })
}

/// Parse an RFC 8941 sf-string item, ignoring parameters.
pub fn sf_string(header: &str) -> Option<String> {
    let mut chars = header.trim().strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                c @ ('"' | '\\') => out.push(c),
                _ => return None,
            },
            ' '..='~' => out.push(c),
            _ => return None,
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofAlg {
    Es256,
    Rs256,
}

impl ProofAlg {
    fn algorithm(self) -> Algorithm {
        match self {
            Self::Es256 => Algorithm::ES256,
            Self::Rs256 => Algorithm::RS256,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Es256 => "ES256",
            Self::Rs256 => "RS256",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ES256" => Some(Self::Es256),
            "RS256" => Some(Self::Rs256),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProofError(&'static str);

/// The proof's device key, reduced to its public parameters and checked
/// against the algorithm.
fn device_key(alg: ProofAlg, jwk: &Jwk) -> Result<Jwk, ProofError> {
    match (&jwk.algorithm, alg) {
        (AlgorithmParameters::EllipticCurve(p), ProofAlg::Es256)
            if p.curve == EllipticCurve::P256 => {}
        (AlgorithmParameters::RSA(p), ProofAlg::Rs256) => {
            let n = base64::Engine::decode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                p.n.as_bytes(),
            )
            .map_err(|_| ProofError("bad RSA key"))?;
            let significant = n.iter().skip_while(|b| **b == 0).count();
            if significant < MIN_RSA_MODULUS_BYTES {
                return Err(ProofError("RSA key too small"));
            }
        }
        _ => return Err(ProofError("key does not match algorithm")),
    }
    Ok(Jwk {
        common: Default::default(),
        algorithm: jwk.algorithm.clone(),
    })
}

#[derive(serde::Deserialize)]
struct ProofClaims {
    jti: String,
}

/// Verify a proof's signature with `key`; returns its `jti`.
fn verify(token: &str, alg: ProofAlg, key: &Jwk) -> Result<String, ProofError> {
    let decoding = DecodingKey::from_jwk(key).map_err(|_| ProofError("bad key"))?;
    let mut validation = Validation::new(alg.algorithm());
    validation.validate_exp = false;
    validation.validate_aud = false;
    validation.required_spec_claims.clear();
    let data = jsonwebtoken::decode::<ProofClaims>(token, &decoding, &validation)
        .map_err(|_| ProofError("bad signature"))?;
    if data.claims.jti.is_empty() {
        return Err(ProofError("proof missing jti"));
    }
    Ok(data.claims.jti)
}

fn header(token: &str) -> Result<jsonwebtoken::Header, ProofError> {
    let header = jsonwebtoken::decode_header(token).map_err(|_| ProofError("malformed proof"))?;
    let typ = header.typ.as_deref().unwrap_or_default();
    if !typ.eq_ignore_ascii_case("dbsc+jwt") && !typ.eq_ignore_ascii_case("jwt") {
        return Err(ProofError("proof type mismatch"));
    }
    Ok(header)
}

fn proof_alg(header: &jsonwebtoken::Header) -> Result<ProofAlg, ProofError> {
    match header.alg {
        Algorithm::ES256 => Ok(ProofAlg::Es256),
        Algorithm::RS256 => Ok(ProofAlg::Rs256),
        _ => Err(ProofError("unsupported algorithm")),
    }
}

/// Verify a registration proof: signed by the key it carries, over
/// `challenge`. Returns the algorithm and the public key to store.
pub fn verify_registration(token: &str, challenge: &str) -> Result<(ProofAlg, Jwk), ProofError> {
    let header = header(token)?;
    let alg = proof_alg(&header)?;
    let key = device_key(
        alg,
        header.jwk.as_ref().ok_or(ProofError("proof missing key"))?,
    )?;
    let jti = verify(token, alg, &key)?;
    if !crate::security::constant_time_eq(jti.as_bytes(), challenge.as_bytes()) {
        return Err(ProofError("challenge mismatch"));
    }
    Ok((alg, key))
}

/// Verify a refresh proof against the stored key. Returns the challenge it
/// answers; the caller consumes it atomically.
pub fn verify_refresh(token: &str, alg: ProofAlg, key: &Jwk) -> Result<String, ProofError> {
    let header = header(token)?;
    if proof_alg(&header)? != alg {
        return Err(ProofError("algorithm mismatch"));
    }
    if header.jwk.is_some() {
        return Err(ProofError("refresh proof must not carry a key"));
    }
    verify(token, alg, key)
}

/// The value of a newly issued, non-empty `refresh_token` cookie, if `headers`
/// set one.
fn issued_refresh_token(headers: &HeaderMap) -> Option<String> {
    headers.get_all(SET_COOKIE).iter().find_map(|value| {
        let cookie = Cookie::parse(value.to_str().ok()?).ok()?;
        let live = cookie.max_age().is_none_or(|age| age.is_positive());
        (cookie.name() == "refresh_token" && !cookie.value().is_empty() && live)
            .then(|| cookie.value().to_string())
    })
}

/// Response layer offering DBSC registration with every new browser session.
/// Mount it on any router whose responses issue the `refresh_token` cookie.
pub async fn offer_registration(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    if path != REGISTER_PATH
        && let Some(token) = issued_refresh_token(response.headers())
        && let Ok(value) =
            HeaderValue::from_str(&registration_header(&path, &registration_challenge(&token)))
    {
        response.headers_mut().insert(REGISTRATION_HEADER, value);
    }
    response
}

fn sf_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(sf_string)
}

fn status(code: StatusCode) -> Response {
    code.into_response()
}

fn challenge_response(challenge: &str, session_id: &str) -> Response {
    let mut response = status(StatusCode::FORBIDDEN);
    if let Ok(value) = HeaderValue::from_str(&challenge_header(challenge, session_id)) {
        response.headers_mut().insert(CHALLENGE_HEADER, value);
    }
    response
}

fn new_challenge() -> String {
    use rand::RngExt;
    let bytes: [u8; 32] = rand::rng().random();
    hex::encode(bytes)
}

fn challenge_expiry() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() + chrono::Duration::seconds(CHALLENGE_TTL_SECS)
}

/// A fresh access-token cookie for `user_id`, or None if the user is gone.
async fn access_cookie(state: &BuiltinAuthBackend, user_id: Uuid) -> Option<Cookie<'static>> {
    let user = state.db.get_user(user_id).await.ok()??;
    let roles: Vec<String> = serde_json::from_value(user.roles).unwrap_or_default();
    let token = state
        .jwt_service
        .generate_access_token(user.id, &user.email, &user.name, &roles)
        .ok()?;
    Some(
        Cookie::build((BOUND_COOKIE, token))
            .path("/")
            .http_only(true)
            .secure(true)
            .same_site(SameSite::Lax)
            .max_age(time::Duration::seconds(
                state.jwt_service.access_token_lifetime_secs(),
            ))
            .build(),
    )
}

fn instructions(
    jar: CookieJar,
    access: Cookie<'static>,
    session_id: &str,
    challenge: &str,
) -> Response {
    let mut response = (
        jar.add(access),
        axum::Json(session_instructions(session_id)),
    )
        .into_response();
    // Handing out the next challenge now saves the browser a 403 round trip.
    if let Ok(value) = HeaderValue::from_str(&challenge_header(challenge, session_id)) {
        response.headers_mut().insert(CHALLENGE_HEADER, value);
    }
    response
}

/// Add the DBSC endpoints to the auth router and wrap it so every new browser
/// session is offered binding and sign-out revokes a bound session. Refresh
/// shares the classic refresh limiter; registration is single-use per token.
pub fn mount(auth_routes: axum::Router, state: BuiltinAuthBackend) -> axum::Router {
    use axum::routing::post;
    let endpoints = axum::Router::new()
        .route(REGISTER_PATH, post(register))
        .merge(
            axum::Router::new()
                .route(REFRESH_PATH, post(refresh))
                .route_layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    super::routes::rate_limit_refresh,
                )),
        )
        .with_state(state.clone());
    auth_routes
        .merge(endpoints)
        .layer(axum::middleware::from_fn_with_state(
            state,
            revoke_on_logout,
        ))
        .layer(axum::middleware::from_fn(offer_registration))
}

/// Sign-out for a device-bound browser, which holds no refresh token: the
/// DBSC session cookie names the row. Revoking it ends the browser's DBSC
/// session at its next refresh. Best-effort, like the rest of logout.
async fn revoke_on_logout(
    State(state): State<BuiltinAuthBackend>,
    jar: CookieJar,
    request: Request,
    next: Next,
) -> Response {
    let logout = request.uri().path() == "/v1/auth/logout";
    if logout
        && let Some(cookie) = jar.get(SESSION_COOKIE)
        && let Err(e) = state
            .db
            .delete_refresh_token_by_dbsc_id(cookie.value())
            .await
    {
        tracing::warn!(error = %e, "failed to revoke device-bound session on logout");
    }
    let response = next.run(request).await;
    if logout {
        return (
            jar.remove(Cookie::build(SESSION_COOKIE).path("/")),
            response,
        )
            .into_response();
    }
    response
}

/// POST /v1/auth/dbsc/start: bind the request's new refresh token to the
/// browser's device key.
pub async fn register(
    State(state): State<BuiltinAuthBackend>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let (Some(proof), Some(refresh)) = (
        sf_header(&headers, RESPONSE_HEADER),
        jar.get("refresh_token").map(|c| c.value().to_string()),
    ) else {
        return status(StatusCode::BAD_REQUEST);
    };
    if state.jwt_service.validate_refresh_token(&refresh).is_err() {
        return status(StatusCode::BAD_REQUEST);
    }
    let Ok((alg, key)) = verify_registration(&proof, &registration_challenge(&refresh)) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let (session_id, challenge) = (new_challenge(), new_challenge());
    let key_json = serde_json::to_value(&key).unwrap_or(Value::Null);
    let bound = state
        .db
        .bind_refresh_token(
            &hash_token(&refresh),
            &session_id,
            alg.as_str(),
            &key_json,
            &challenge,
            challenge_expiry(),
        )
        .await;
    let user_id = match bound {
        Ok(Some(user_id)) => user_id,
        // Expired, signed out, or already bound: one key per session.
        Ok(None) => return status(StatusCode::BAD_REQUEST),
        Err(error) => {
            tracing::error!(%error, "dbsc registration storage error");
            return status(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let Some(access) = access_cookie(&state, user_id).await else {
        return status(StatusCode::BAD_REQUEST);
    };
    let jar = jar.remove(Cookie::build("refresh_token").path("/")).add(
        Cookie::build((SESSION_COOKIE, session_id.clone()))
            .path("/")
            .http_only(true)
            .secure(true)
            .same_site(SameSite::Strict)
            .max_age(time::Duration::seconds(
                state.jwt_service.refresh_token_lifetime_secs(),
            ))
            .build(),
    );
    instructions(jar, access, &session_id, &challenge)
}

/// POST /v1/auth/dbsc/refresh: exchange a proof from the bound key for a new
/// access token.
pub async fn refresh(
    State(state): State<BuiltinAuthBackend>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let Some(session_id) = sf_header(&headers, SESSION_ID_HEADER) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let session = match state.db.bound_refresh_token(&session_id).await {
        Ok(Some(session)) => session,
        // Signed out, revoked or expired: tell the browser to drop it.
        Ok(None) => return axum::Json(json!({"continue": false})).into_response(),
        Err(error) => {
            tracing::error!(%error, "dbsc refresh storage error");
            return status(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let reissue = |state: BuiltinAuthBackend, session_id: String| async move {
        match state
            .db
            .dbsc_refresh_challenge(&session_id, &new_challenge(), challenge_expiry())
            .await
        {
            Ok(Some(challenge)) => challenge_response(&challenge, &session_id),
            Ok(None) => axum::Json(json!({"continue": false})).into_response(),
            Err(error) => {
                tracing::error!(%error, "dbsc challenge storage error");
                status(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    };
    let Some(proof) = sf_header(&headers, RESPONSE_HEADER) else {
        return reissue(state, session_id).await;
    };
    let (Some(alg), Ok(key)) = (
        ProofAlg::parse(&session.dbsc_alg),
        serde_json::from_value::<Jwk>(session.dbsc_public_jwk.clone()),
    ) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let Ok(answered) = verify_refresh(&proof, alg, &key) else {
        // Not signed by this session's key: a copied cookie's owner, not the device.
        return status(StatusCode::BAD_REQUEST);
    };
    let next = new_challenge();
    match state
        .db
        .consume_dbsc_challenge(&session_id, &answered, &next, challenge_expiry())
        .await
    {
        Ok(true) => {}
        // Valid key, stale or reused challenge: ask again.
        Ok(false) => return reissue(state, session_id).await,
        Err(error) => {
            tracing::error!(%error, "dbsc refresh storage error");
            return status(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
    let Some(access) = access_cookie(&state, session.user_id).await else {
        return axum::Json(json!({"continue": false})).into_response();
    };
    instructions(jar, access, &session_id, &next)
}

#[cfg(test)]
mod tests;
