//! Optional forward proxy for tenant egress.
//!
//! Decisions:
//! - Only tenant traffic (capability, integration, MCP and `other` kinds) goes
//!   through the proxy. Host-owned transports (LLM providers, system email,
//!   the utility LLM) keep the deployment's own address, so a tenant whose
//!   traffic gets the proxy's addresses blocklisted cannot take the platform's
//!   own calls down with it.
//! - Each proxied request carries a short-lived signed token naming its org and
//!   session, so the proxy can meter and limit per org without trusting the
//!   caller: `v1.<org>.<session>.<exp>.<hmac>` where `-` stands for a missing
//!   id and the HMAC-SHA256 covers everything before the last dot. The scope
//!   comes from `ScopedEgressService`, never from the tool.
//! - Misconfiguration fails closed: a proxy URL with a missing or short secret
//!   refuses tenant requests rather than letting them leave directly.
//! - With a proxy, the proxy connects to the target, so DNS pinning moves
//!   there: the proxy must re-resolve and refuse non-public addresses. The
//!   in-process SSRF check still runs first.
//! - This file owns the token format on both sides: [`sign_proxy_token`]
//!   mints and [`verify_proxy_token`] checks, so a format change cannot reach
//!   one side only. The proxy itself lives in `everruns-server`
//!   (`everruns-egress-proxy`).
//! - Verification is strict: anything that is not exactly the shape is
//!   malformed, never a best-effort guess, because the org in the token is
//!   what every proxy limit is keyed by. The signature is compared in
//!   constant time and checked before expiry. A token whose expiry is more
//!   than [`MAX_PROXY_TOKEN_FUTURE_SECS`] ahead is refused even when signed:
//!   tokens are minted for five minutes, so a far-future one can only come
//!   from a bug or a leaked secret, and the cap bounds how long a captured
//!   token can be replayed.

use crate::{EgressError, EgressRequestKind, EgressResult, EgressScope};
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Forward proxy URL for tenant egress, e.g. `http://egress-proxy:4750`.
pub const EGRESS_PROXY_URL_ENV: &str = "EVERRUNS_EGRESS_PROXY_URL";
/// Secret shared with the proxy for signing request tokens (32+ bytes).
pub const EGRESS_PROXY_SECRET_ENV: &str = "EVERRUNS_EGRESS_PROXY_SECRET";

/// Shortest proxy secret accepted on either side, in bytes.
pub const MIN_PROXY_SECRET_LEN: usize = 32;
const MIN_SECRET_LEN: usize = MIN_PROXY_SECRET_LEN;
const TOKEN_TTL: Duration = Duration::from_secs(300);
/// How far ahead of now a token's expiry may be. Covers clock skew between
/// hosts on top of the five-minute lifetime.
pub const MAX_PROXY_TOKEN_FUTURE_SECS: i64 = 3_600;

/// The tenant egress proxy and the secret its tokens are signed with.
#[derive(Clone)]
pub struct TenantEgressProxy {
    url: reqwest::Url,
    secret: Vec<u8>,
}

impl std::fmt::Debug for TenantEgressProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TenantEgressProxy")
            .field("url", &self.url.as_str())
            .finish_non_exhaustive()
    }
}

/// How tenant requests leave this process.
#[derive(Clone, Debug, Default)]
pub(crate) enum ProxyRoute {
    /// Directly from this process.
    #[default]
    Direct,
    Proxy(TenantEgressProxy),
    /// A proxy was configured but cannot be used; tenant requests fail.
    Misconfigured(String),
}

impl ProxyRoute {
    pub(crate) fn from_env() -> Self {
        let url = std::env::var(EGRESS_PROXY_URL_ENV).unwrap_or_default();
        if url.trim().is_empty() {
            return Self::Direct;
        }
        let secret = std::env::var(EGRESS_PROXY_SECRET_ENV).unwrap_or_default();
        match TenantEgressProxy::new(url.trim(), secret.into_bytes()) {
            Ok(proxy) => Self::Proxy(proxy),
            Err(error) => {
                tracing::error!(error = %error, "tenant egress proxy misconfigured; refusing tenant egress");
                Self::Misconfigured(error)
            }
        }
    }

    /// The proxy a request of this kind must use, if any.
    pub(crate) fn for_kind(
        &self,
        kind: &EgressRequestKind,
    ) -> EgressResult<Option<&TenantEgressProxy>> {
        if !is_tenant_kind(kind) {
            return Ok(None);
        }
        match self {
            Self::Direct => Ok(None),
            Self::Proxy(proxy) => Ok(Some(proxy)),
            Self::Misconfigured(error) => Err(EgressError::Transport(format!(
                "tenant egress proxy misconfigured: {error}"
            ))),
        }
    }
}

/// Whether requests of this kind are tenant traffic.
fn is_tenant_kind(kind: &EgressRequestKind) -> bool {
    match kind {
        EgressRequestKind::Capability
        | EgressRequestKind::Integration
        | EgressRequestKind::Mcp
        | EgressRequestKind::Other(_) => true,
        EgressRequestKind::Provider
        | EgressRequestKind::SystemEmail
        | EgressRequestKind::UtilityLlm => false,
    }
}

impl TenantEgressProxy {
    /// A proxy at `url` (http or https) signing tokens with `secret`.
    pub fn new(url: &str, secret: Vec<u8>) -> Result<Self, String> {
        let url =
            reqwest::Url::parse(url).map_err(|error| format!("invalid proxy URL: {error}"))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err("proxy URL must be http(s)://host:port".to_string());
        }
        if secret.len() < MIN_SECRET_LEN {
            return Err(format!(
                "{EGRESS_PROXY_SECRET_ENV} must be at least {MIN_SECRET_LEN} bytes"
            ));
        }
        Ok(Self { url, secret })
    }

    /// The signed token for one request's scope, valid for five minutes.
    pub fn token(&self, scope: Option<&EgressScope>) -> String {
        let exp = SystemTime::now()
            .checked_add(TOKEN_TTL)
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |at| at.as_secs());
        self.token_at(scope, exp)
    }

    fn token_at(&self, scope: Option<&EgressScope>, exp: u64) -> String {
        let org = scope
            .and_then(|scope| scope.org_id.as_ref())
            .map_or_else(|| "-".to_string(), ToString::to_string);
        let session = scope
            .and_then(|scope| scope.session_id.as_ref())
            .map_or_else(|| "-".to_string(), ToString::to_string);
        sign_proxy_token(&self.secret, &org, &session, exp as i64)
    }

    /// A reqwest proxy for one request, authenticated with its token.
    pub(crate) fn reqwest_proxy(
        &self,
        scope: Option<&EgressScope>,
    ) -> EgressResult<reqwest::Proxy> {
        let auth = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.token(scope)))
            .map_err(|error| EgressError::invalid(format!("proxy token: {error}")))?;
        Ok(reqwest::Proxy::all(self.url.clone())
            .map_err(|error| EgressError::invalid(format!("proxy URL: {error}")))?
            .custom_http_auth(auth))
    }
}

/// Sign `v1.<org>.<session>.<exp>` with `secret`. `-` stands for a missing
/// org or session.
pub fn sign_proxy_token(secret: &[u8], org: &str, session: &str, exp: i64) -> String {
    let claims = format!("v1.{org}.{session}.{exp}");
    format!(
        "{claims}.{}",
        to_hex(&hmac_sha256(secret, claims.as_bytes()))
    )
}

/// Who a proxied request belongs to, as its token says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyClaims {
    /// `None` for unscoped (`-`) traffic.
    pub org: Option<String>,
    /// `None` when the request had no session.
    pub session: Option<String>,
    /// Expiry, unix seconds.
    pub exp: i64,
}

/// Why a proxy token was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProxyTokenError {
    /// No `Proxy-Authorization` header.
    #[error("missing proxy credentials")]
    Missing,
    /// Not exactly `Bearer v1.<org>.<session>.<exp>.<sig>`.
    #[error("malformed proxy token")]
    Malformed,
    /// Signed with another secret, or altered.
    #[error("bad proxy token signature")]
    BadSignature,
    /// Past its expiry.
    #[error("expired proxy token")]
    Expired,
    /// Expiry further ahead than [`MAX_PROXY_TOKEN_FUTURE_SECS`].
    #[error("proxy token expiry too far in the future")]
    TooFarAhead,
}

/// Verify the value of a `Proxy-Authorization` header (`Bearer <token>`).
pub fn verify_proxy_authorization(
    header: Option<&str>,
    secret: &[u8],
    now_unix: i64,
) -> Result<ProxyClaims, ProxyTokenError> {
    let header = header.ok_or(ProxyTokenError::Missing)?.trim();
    let (scheme, token) = header
        .split_once(' ')
        .ok_or(ProxyTokenError::Malformed)?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(ProxyTokenError::Malformed);
    }
    verify_proxy_token(token.trim(), secret, now_unix)
}

/// Verify a bare proxy token.
pub fn verify_proxy_token(
    token: &str,
    secret: &[u8],
    now_unix: i64,
) -> Result<ProxyClaims, ProxyTokenError> {
    let parts: Vec<&str> = token.split('.').collect();
    let [version, org, session, exp, sig] = parts.as_slice() else {
        return Err(ProxyTokenError::Malformed);
    };
    if *version != "v1" || !valid_token_org(org) || !valid_token_session(session) {
        return Err(ProxyTokenError::Malformed);
    }
    if exp.is_empty() || exp.len() > 12 || !exp.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ProxyTokenError::Malformed);
    }
    let exp: i64 = exp.parse().map_err(|_| ProxyTokenError::Malformed)?;
    if sig.len() != 64 || !sig.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(ProxyTokenError::Malformed);
    }

    // Signature first: nothing about expiry handling is revealed for a token
    // the caller could not have minted.
    let claims = &token[..token.len() - 65]; // ".<64 hex>"
    let expected = to_hex(&hmac_sha256(secret, claims.as_bytes()));
    if !constant_time_eq(expected.as_bytes(), sig.as_bytes()) {
        return Err(ProxyTokenError::BadSignature);
    }

    if exp <= now_unix {
        return Err(ProxyTokenError::Expired);
    }
    if exp - now_unix > MAX_PROXY_TOKEN_FUTURE_SECS {
        return Err(ProxyTokenError::TooFarAhead);
    }
    Ok(ProxyClaims {
        org: (*org != "-").then(|| org.to_string()),
        session: (*session != "-").then(|| session.to_string()),
        exp,
    })
}

/// `org_` followed by 32 lowercase hex, or `-`.
fn valid_token_org(org: &str) -> bool {
    org == "-"
        || org.strip_prefix("org_").is_some_and(|hex| {
            hex.len() == 32 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
}

/// A short public session id, or `-`. Only its alphabet matters: it must not
/// smuggle a separator into the token, logs, or usage keys.
fn valid_token_session(session: &str) -> bool {
    session == "-"
        || (!session.is_empty()
            && session.len() <= 64
            && session
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
}

/// Constant-time byte comparison, for proxy signatures and shared secrets.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// HMAC-SHA256 (RFC 2104) over `sha2`, which core already carries, so the
/// token needs no extra crypto dependency in core.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| block.map(|b| b ^ byte);
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(message)
        .finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";

    fn proxy() -> TenantEgressProxy {
        TenantEgressProxy::new("http://egress-proxy:4750", SECRET.to_vec()).unwrap()
    }

    #[test]
    fn token_names_scope_and_signs_the_claims() {
        let scope = EgressScope {
            org_id: Some("org_0000000000000000000000000000002a".parse().unwrap()),
            session_id: None,
        };
        let token = proxy().token_at(Some(&scope), 1_900_000_000);
        let (claims, sig) = token.rsplit_once('.').unwrap();
        assert_eq!(
            claims,
            "v1.org_0000000000000000000000000000002a.-.1900000000"
        );
        assert_eq!(sig, to_hex(&hmac_sha256(SECRET, claims.as_bytes())));
        assert_eq!(proxy().token_at(None, 7).split('.').nth(1), Some("-"));
    }

    #[test]
    fn hmac_matches_rfc_4231_vectors() {
        // Test case 2 (short key) and 6 (key longer than a block).
        assert_eq!(
            to_hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            to_hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    const ORG: &str = "org_0123456789abcdef0123456789abcdef";
    const NOW: i64 = 1_800_000_000;

    #[test]
    fn a_minted_token_verifies_with_its_scope() {
        let scope = EgressScope {
            org_id: Some(ORG.parse().unwrap()),
            session_id: None,
        };
        let token = proxy().token(Some(&scope));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let claims =
            verify_proxy_authorization(Some(&format!("Bearer {token}")), SECRET, now).unwrap();
        assert_eq!(claims.org.as_deref(), Some(ORG));
        assert_eq!(claims.session, None);
        assert!(claims.exp > now && claims.exp <= now + 300);

        let unscoped = verify_proxy_token(&proxy().token(None), SECRET, now).unwrap();
        assert_eq!((unscoped.org, unscoped.session), (None, None));
    }

    #[test]
    fn expired_far_future_and_forged_tokens_are_refused() {
        let token = sign_proxy_token(SECRET, ORG, "session_01", NOW + 300);
        let claims = verify_proxy_token(&token, SECRET, NOW).unwrap();
        assert_eq!(claims.session.as_deref(), Some("session_01"));
        for exp in [NOW - 1, NOW] {
            let token = sign_proxy_token(SECRET, ORG, "-", exp);
            assert_eq!(
                verify_proxy_token(&token, SECRET, NOW),
                Err(ProxyTokenError::Expired)
            );
        }
        let token = sign_proxy_token(SECRET, ORG, "-", NOW + MAX_PROXY_TOKEN_FUTURE_SECS + 1);
        assert_eq!(
            verify_proxy_token(&token, SECRET, NOW),
            Err(ProxyTokenError::TooFarAhead)
        );
        let other = sign_proxy_token(b"another-secret-another-secret-xx", ORG, "-", NOW + 300);
        assert_eq!(
            verify_proxy_token(&other, SECRET, NOW),
            Err(ProxyTokenError::BadSignature)
        );
        // Re-pointing a valid token at another org breaks the signature.
        let good = sign_proxy_token(SECRET, ORG, "-", NOW + 300);
        let forged = good.replace(ORG, "org_ffffffffffffffffffffffffffffffff");
        assert_eq!(
            verify_proxy_token(&forged, SECRET, NOW),
            Err(ProxyTokenError::BadSignature)
        );
    }

    #[test]
    fn malformed_tokens_and_headers_are_refused() {
        let good = sign_proxy_token(SECRET, ORG, "-", NOW + 300);
        for bad in [
            "",
            "v1",
            &good.replace("v1.", "v2."),
            &good.to_uppercase(),
            &format!("{good}.extra"),
            &good.replace(ORG, "org_short"),
            &good.replace(ORG, "../../etc"),
        ] {
            assert_eq!(
                verify_proxy_token(bad, SECRET, NOW).err(),
                Some(ProxyTokenError::Malformed),
                "{bad:?} should be malformed"
            );
        }
        assert_eq!(
            verify_proxy_authorization(None, SECRET, NOW),
            Err(ProxyTokenError::Missing)
        );
        for header in [format!("Basic {good}"), good.clone()] {
            assert_eq!(
                verify_proxy_authorization(Some(&header), SECRET, NOW),
                Err(ProxyTokenError::Malformed)
            );
        }
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn short_secrets_and_bad_urls_are_rejected() {
        assert!(TenantEgressProxy::new("http://p:1", b"short".to_vec()).is_err());
        assert!(TenantEgressProxy::new("ftp://p:1", SECRET.to_vec()).is_err());
        assert!(TenantEgressProxy::new("not a url", SECRET.to_vec()).is_err());
    }

    #[test]
    fn only_tenant_kinds_use_the_proxy_and_misconfiguration_fails_closed() {
        let route = ProxyRoute::Proxy(proxy());
        assert!(route.for_kind(&EgressRequestKind::Mcp).unwrap().is_some());
        assert!(
            route
                .for_kind(&EgressRequestKind::Other("plugin".into()))
                .unwrap()
                .is_some()
        );
        assert!(
            route
                .for_kind(&EgressRequestKind::Provider)
                .unwrap()
                .is_none()
        );
        assert!(
            route
                .for_kind(&EgressRequestKind::SystemEmail)
                .unwrap()
                .is_none()
        );

        let broken = ProxyRoute::Misconfigured("no secret".into());
        assert!(broken.for_kind(&EgressRequestKind::Capability).is_err());
        assert!(
            broken
                .for_kind(&EgressRequestKind::UtilityLlm)
                .unwrap()
                .is_none()
        );
        assert!(
            ProxyRoute::Direct
                .for_kind(&EgressRequestKind::Mcp)
                .unwrap()
                .is_none()
        );
    }
}
