//! Org tokens at the gateway (cp-dp-protocol.md, "Org tokens"): a JWT the
//! control plane signs (EdDSA) that names the org a caller acts for.
//!
//! Decisions:
//! - Checked against the control plane's JWKS, fetched and cached; an
//!   unknown `kid` refetches (at most every [`REFETCH`]), so a rotated key
//!   is picked up without a restart and no request calls back otherwise.
//! - Off unless configured (`CELL_JWKS_URL`): a test fleet and the step 6
//!   suites run open, as before. Configured, every `/v1/sessions` call
//!   needs a token with the `sessions:write` scope (or `admin`).
//! - The token's org is the session's tenant. A create bills the session to
//!   it (a different `org` or `tenant` in the body is refused), and every
//!   later call on a session checks the session's tenant against it. A
//!   session with no tenant is reachable only with `admin`.
//! - `dp` must name this data plane when one is configured
//!   (`CELL_DATA_PLANE`): a token minted for another data plane is refused.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use parking_lot::Mutex;

pub use everruns_contracts::dataplane::OrgTokenClaims;

/// The scope that may create and drive sessions.
pub const SESSIONS: &str = "sessions:write";
/// The scope that may do anything, including tenantless sessions.
pub const ADMIN: &str = "admin";
/// How often an unknown `kid` may refetch the JWKS.
const REFETCH: Duration = Duration::from_secs(10);
/// How long fetched keys are trusted before they are refetched anyway.
const KEYS_FOR: Duration = Duration::from_secs(300);
/// Clock skew allowed on an org token's expiry, in seconds.
const LEEWAY_SECS: u64 = 5;

type ApiError = (StatusCode, String);

pub struct OrgTokens {
    jwks_url: Option<String>,
    data_plane: Option<String>,
    http: reqwest::Client,
    keys: Mutex<Keys>,
}

#[derive(Default)]
struct Keys {
    by_kid: HashMap<String, DecodingKey>,
    fetched: Option<Instant>,
}

impl OrgTokens {
    /// From `CELL_JWKS_URL` and `CELL_DATA_PLANE`; none when unset.
    pub fn from_env() -> Option<Arc<Self>> {
        let url = std::env::var("CELL_JWKS_URL")
            .ok()
            .filter(|u| !u.is_empty())?;
        let data_plane = std::env::var("CELL_DATA_PLANE")
            .ok()
            .filter(|d| !d.is_empty());
        Some(Arc::new(Self::new(Some(url), data_plane)))
    }

    pub fn new(jwks_url: Option<String>, data_plane: Option<String>) -> Self {
        Self {
            jwks_url,
            data_plane,
            http: reqwest::Client::new(),
            keys: Mutex::default(),
        }
    }

    /// Trust `set` (tests, and a JWKS handed over at start).
    pub fn trust(&self, set: &JwkSet) -> Result<(), String> {
        let mut by_kid = HashMap::new();
        for jwk in &set.keys {
            let Some(kid) = jwk.common.key_id.clone() else {
                continue;
            };
            by_kid.insert(kid, DecodingKey::from_jwk(jwk).map_err(|e| e.to_string())?);
        }
        let mut keys = self.keys.lock();
        keys.by_kid = by_kid;
        keys.fetched = Some(Instant::now());
        Ok(())
    }

    async fn fetch(&self) -> Result<(), String> {
        let Some(url) = &self.jwks_url else {
            return Ok(());
        };
        let set: JwkSet = self
            .http
            .get(url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        self.trust(&set)
    }

    fn key(&self, kid: &str) -> Option<DecodingKey> {
        self.keys.lock().by_kid.get(kid).cloned()
    }

    /// The claims of `token`, checked: signature, expiry, data plane.
    pub async fn verify(&self, token: &str) -> Result<OrgTokenClaims, ApiError> {
        let header = jsonwebtoken::decode_header(token).map_err(unauthorized)?;
        if header.alg != Algorithm::EdDSA {
            return Err(unauthorized("org tokens are EdDSA"));
        }
        let kid = header
            .kid
            .ok_or_else(|| unauthorized("the token names no key"))?;
        let (stale, may_refetch) = {
            let keys = self.keys.lock();
            let age = keys.fetched.map(|at| at.elapsed());
            (
                age.is_none_or(|a| a >= KEYS_FOR),
                age.is_none_or(|a| a >= REFETCH),
            )
        };
        if (stale || (self.key(&kid).is_none() && may_refetch))
            && let Err(error) = self.fetch().await
        {
            tracing::warn!(%error, "the control plane's keys were not fetched");
        }
        let key = self
            .key(&kid)
            .ok_or_else(|| unauthorized("the token's key is not known"))?;
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_aud = false;
        // Decision: 5 s of clock skew, not jsonwebtoken's 60 s or a
        // generous 30: an org token lives minutes (30 s in the expiry
        // drill), and leeway is extra life for a token nobody refreshed.
        validation.leeway = LEEWAY_SECS;
        let claims = jsonwebtoken::decode::<OrgTokenClaims>(token, &key, &validation)
            .map_err(unauthorized)?
            .claims;
        if let Some(dp) = &self.data_plane
            && claims.dp != *dp
        {
            return Err(unauthorized("the token is for another data plane"));
        }
        Ok(claims)
    }
}

impl OrgTokenClaimsExt for OrgTokenClaims {
    fn has(&self, scope: &str) -> bool {
        self.scope.split_whitespace().any(|s| s == scope)
    }

    fn may_drive(&self, tenant: Option<&str>) -> bool {
        if self.has(ADMIN) {
            return true;
        }
        self.has(SESSIONS) && tenant == Some(self.org.as_str())
    }
}

/// What a checked token allows.
pub trait OrgTokenClaimsExt {
    fn has(&self, scope: &str) -> bool;
    /// May this caller act on a session of `tenant`?
    fn may_drive(&self, tenant: Option<&str>) -> bool;
}

fn unauthorized(why: impl std::fmt::Display) -> ApiError {
    (StatusCode::UNAUTHORIZED, format!("org token: {why}"))
}

/// The header a client that also sends its own `Authorization` (a browser
/// with a console session) puts its org token in.
pub const HEADER: &str = "x-everruns-org-token";
/// The cookie a browser keeps its org token in (an `EventSource` sets no
/// headers).
pub const COOKIE: &str = "everruns_org_token";

/// The org token of a request, if any: the `X-Everruns-Org-Token` header,
/// the `everruns_org_token` cookie, or the bearer token, in that order.
pub fn bearer(headers: &axum::http::HeaderMap) -> Option<&str> {
    if let Some(token) = headers.get(HEADER).and_then(|v| v.to_str().ok()) {
        return Some(token.trim());
    }
    if let Some(token) = cookie(headers, COOKIE) {
        return Some(token);
    }
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// The value of cookie `name`, if the request carries it.
pub fn cookie<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, header};

    #[test]
    fn the_org_token_header_wins_then_the_cookie_then_the_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer b"));
        assert_eq!(bearer(&headers), Some("b"));
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; everruns_org_token=c; z=2"),
        );
        assert_eq!(bearer(&headers), Some("c"));
        headers.insert(HEADER, HeaderValue::from_static("h"));
        assert_eq!(bearer(&headers), Some("h"));
    }
}
