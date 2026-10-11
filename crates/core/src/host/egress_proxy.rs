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

use crate::{EgressError, EgressRequestKind, EgressResult, EgressScope};
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Forward proxy URL for tenant egress, e.g. `http://egress-proxy:4750`.
pub const EGRESS_PROXY_URL_ENV: &str = "EVERRUNS_EGRESS_PROXY_URL";
/// Secret shared with the proxy for signing request tokens (32+ bytes).
pub const EGRESS_PROXY_SECRET_ENV: &str = "EVERRUNS_EGRESS_PROXY_SECRET";

const MIN_SECRET_LEN: usize = 32;
const TOKEN_TTL: Duration = Duration::from_secs(300);

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
        let claims = format!("v1.{org}.{session}.{exp}");
        format!(
            "{claims}.{}",
            to_hex(&hmac_sha256(&self.secret, claims.as_bytes()))
        )
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
