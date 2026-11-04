//! Web Bot Auth: platform signing of outbound agent requests.
//!
//! Everruns proves to the sites its agents visit that a request came from
//! this deployment. Requests carry RFC 9421 HTTP Message Signatures with the
//! Web Bot Auth profile (`draft-ietf-webbotauth-httpsig-protocol`), and the
//! server publishes the matching public keys as a signed key directory at
//! `/.well-known/http-message-signatures-directory`.
//!
//! Decision: one signer for every caller, owned by the egress boundary.
//!   `web_fetch` and the Bashkit shell used to sign inside fetchkit and
//!   Bashkit, two copies that drifted apart and both encoded signatures as
//!   base64url, which RFC 8941 byte sequences do not allow. Callers now ask
//!   for signing with `EgressSigning::PlatformDefault` and the egress
//!   service signs (knowledge/operations/egress.md, "Signing").
//! Decision: `Signature-Agent` is sent as the legacy sf-string form
//!   (`"https://app.example.com"`), the deployment's origin. The working-group
//!   draft asks signers for the dictionary form (`sig1="https://…"`), but
//!   Cloudflare, the largest verifier, still rejects it (checked 2026-11-04).
//!   The draft lets verifiers accept the string form, so this verifies on
//!   both. Switch to the dictionary form once Cloudflare accepts it.
//! Decision: covered components are `@method`, `@authority`, `@path` and
//!   `signature-agent`. `@authority` alone is the minimum; the draft
//!   recommends narrowing with `@method` and `@path` so a captured signature
//!   cannot be replayed against other resources on the same site. The query
//!   is left out on purpose, as intermediaries may reorder it.
//! Decision: requests to plain `http://` URLs are not signed. A signature
//!   on an unencrypted channel can be lifted and replayed by anyone on the
//!   path (draft section 6.1).
//! Decision: a caller that already set `Signature` or `Signature-Input` keeps
//!   its own signature; Everruns does not overwrite a user's RFC 9421
//!   signature for an API that expects one.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Signature label used in `Signature` and `Signature-Input`.
const REQUEST_LABEL: &str = "sig1";
const REQUEST_TAG: &str = "web-bot-auth";
const DIRECTORY_TAG: &str = "http-message-signatures-directory";
/// Media type the draft requires for a directory at the well-known URI.
pub const DIRECTORY_MEDIA_TYPE: &str = "application/http-message-signatures-directory+json";
/// The well-known path the key directory is served at.
pub const DIRECTORY_PATH: &str = "/.well-known/http-message-signatures-directory";
const DEFAULT_REQUEST_VALIDITY: Duration = Duration::from_secs(300);
/// Directory proofs are long-lived on purpose: a short `expires` on them
/// buys no revocation speed and drops keys to unverified at expiry (draft
/// appendix C.8). Removing a key from the directory is what revokes it.
const DIRECTORY_SIGNATURE_VALIDITY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long verifiers may cache the directory.
pub const DIRECTORY_MAX_AGE_SECS: u64 = 24 * 60 * 60;

/// Configuration errors for [`WebBotAuth`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WebBotAuthError {
    #[error("signing key seed must be base64url-encoded 32 bytes")]
    InvalidSeed,
    #[error("signature agent must be an absolute https:// URL")]
    InvalidSignatureAgent,
    #[error("request URL cannot be signed: {0}")]
    UnsignableUrl(&'static str),
    #[error("system clock is before the Unix epoch")]
    Clock,
}

/// Header names and values to add to a signed request or directory response.
pub type SignatureHeaders = Vec<(&'static str, String)>;

/// A signed key directory response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedDirectory {
    /// JWKS body, exactly the bytes the `Content-Digest` covers.
    pub body: String,
    /// `Content-Digest`, `Signature-Input` and `Signature`.
    pub headers: SignatureHeaders,
}

/// Platform Web Bot Auth signer and key directory.
pub struct WebBotAuth {
    /// The key that signs requests.
    signing: SigningKey,
    /// Keys published alongside the signing key during a rotation.
    published: Vec<SigningKey>,
    /// `Signature-Agent` value, already a serialized sf-string.
    signature_agent: String,
    validity: Duration,
}

impl std::fmt::Debug for WebBotAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebBotAuth")
            .field("keyid", &self.keyid())
            .field("signature_agent", &self.signature_agent)
            .finish_non_exhaustive()
    }
}

/// Parameters that vary per signature; fixed in tests.
struct RequestParams<'a> {
    created: u64,
    expires: u64,
    nonce: &'a str,
}

impl WebBotAuth {
    /// Build a signer from a base64url seed and the `Signature-Agent` URL.
    pub fn new(seed: &str, signature_agent: &str) -> Result<Self, WebBotAuthError> {
        Ok(Self {
            signing: signing_key_from_seed(seed)?,
            published: Vec::new(),
            signature_agent: serialize_signature_agent(signature_agent)?,
            validity: DEFAULT_REQUEST_VALIDITY,
        })
    }

    /// Also publish these keys in the directory without signing with them.
    ///
    /// Used to rotate: publish the next key, wait for verifier caches to
    /// refresh, switch the signing seed, and keep the old key published for
    /// another cache lifetime before dropping it.
    pub fn with_published_seeds<'a>(
        mut self,
        seeds: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, WebBotAuthError> {
        for seed in seeds {
            let key = signing_key_from_seed(seed)?;
            if key.verifying_key() != self.signing.verifying_key()
                && !self
                    .published
                    .iter()
                    .any(|known| known.verifying_key() == key.verifying_key())
            {
                self.published.push(key);
            }
        }
        Ok(self)
    }

    /// Override the request signature lifetime (default five minutes).
    pub fn with_validity(mut self, validity: Duration) -> Self {
        self.validity = validity;
        self
    }

    /// Process-wide signer from the environment, read once.
    ///
    /// - `BOT_AUTH_SIGNING_KEY_SEED`: base64url 32-byte Ed25519 seed; unset
    ///   disables signing and leaves the directory empty.
    /// - `BOT_AUTH_SIGNATURE_AGENT_URL`: https origin whose
    ///   `/.well-known/http-message-signatures-directory` serves the keys;
    ///   defaults to `PUBLIC_APP_URL`. Signing stays off without one, since
    ///   every signed request must say where its key lives.
    /// - `BOT_AUTH_PUBLISHED_KEY_SEEDS`: comma-separated seeds published but
    ///   not used to sign, for rotation.
    /// - `BOT_AUTH_VALIDITY_SECS`: request signature lifetime.
    pub fn from_env() -> Option<Arc<Self>> {
        static SIGNER: OnceLock<Option<Arc<WebBotAuth>>> = OnceLock::new();
        SIGNER
            .get_or_init(|| Self::from_vars(|name| std::env::var(name).ok()).map(Arc::new))
            .clone()
    }

    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let seed = var("BOT_AUTH_SIGNING_KEY_SEED").filter(|seed| !seed.trim().is_empty())?;
        let Some(agent) = var("BOT_AUTH_SIGNATURE_AGENT_URL")
            .or_else(|| var("PUBLIC_APP_URL"))
            .filter(|url| !url.trim().is_empty())
        else {
            tracing::warn!(
                "BOT_AUTH_SIGNING_KEY_SEED is set but neither BOT_AUTH_SIGNATURE_AGENT_URL nor \
                 PUBLIC_APP_URL is; Web Bot Auth signing disabled"
            );
            return None;
        };
        let published = var("BOT_AUTH_PUBLISHED_KEY_SEEDS").unwrap_or_default();
        let built = Self::new(seed.trim(), agent.trim()).and_then(|signer| {
            signer.with_published_seeds(
                published
                    .split(',')
                    .map(str::trim)
                    .filter(|seed| !seed.is_empty()),
            )
        });
        let mut signer = match built {
            Ok(signer) => signer,
            Err(error) => {
                tracing::warn!(%error, "invalid Web Bot Auth configuration; signing disabled");
                return None;
            }
        };
        if let Some(secs) = var("BOT_AUTH_VALIDITY_SECS").and_then(|s| s.trim().parse().ok()) {
            signer = signer.with_validity(Duration::from_secs(secs));
        }
        tracing::info!(keyid = %signer.keyid(), "Web Bot Auth request signing enabled");
        Some(signer)
    }

    /// JWK thumbprint of the signing key, the `keyid` in every signature.
    pub fn keyid(&self) -> String {
        jwk_thumbprint(&self.signing)
    }

    /// Headers that sign a request: `Signature-Agent`, `Signature-Input`,
    /// `Signature`.
    pub fn sign_request(
        &self,
        method: &str,
        url: &str,
    ) -> Result<SignatureHeaders, WebBotAuthError> {
        let created = unix_now()?;
        let nonce = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        self.sign_request_with(
            method,
            url,
            &RequestParams {
                created,
                expires: created + self.validity.as_secs(),
                nonce: &nonce,
            },
        )
    }

    fn sign_request_with(
        &self,
        method: &str,
        url: &str,
        params: &RequestParams<'_>,
    ) -> Result<SignatureHeaders, WebBotAuthError> {
        let url =
            url::Url::parse(url).map_err(|_| WebBotAuthError::UnsignableUrl("invalid URL"))?;
        if url.scheme() != "https" {
            return Err(WebBotAuthError::UnsignableUrl(
                "only https requests are signed",
            ));
        }
        let authority = authority(&url)?;
        let path = if url.path().is_empty() {
            "/"
        } else {
            url.path()
        };
        let method = method.to_ascii_uppercase();
        let signature_params = format!(
            "(\"@method\" \"@authority\" \"@path\" \"signature-agent\");created={};keyid=\"{}\";\
             alg=\"ed25519\";expires={};nonce=\"{}\";tag=\"{REQUEST_TAG}\"",
            params.created,
            self.keyid(),
            params.expires,
            params.nonce,
        );
        let base = format!(
            "\"@method\": {method}\n\"@authority\": {authority}\n\"@path\": {path}\n\
             \"signature-agent\": {}\n\"@signature-params\": {signature_params}",
            self.signature_agent
        );
        Ok(vec![
            ("signature-agent", self.signature_agent.clone()),
            (
                "signature-input",
                format!("{REQUEST_LABEL}={signature_params}"),
            ),
            (
                "signature",
                format!("{REQUEST_LABEL}=:{}:", self.sign(&self.signing, &base)),
            ),
        ])
    }

    /// The key directory, signed for a request made to `authority`.
    ///
    /// One signature per published key over `@authority;req` and
    /// `content-digest` proves possession of every key and stops the key set
    /// from being re-served under another host (draft appendix B.1).
    pub fn directory(&self, authority: &str) -> Result<SignedDirectory, WebBotAuthError> {
        let created = unix_now()?;
        Ok(self.directory_with(
            authority,
            created,
            created + DIRECTORY_SIGNATURE_VALIDITY.as_secs(),
        ))
    }

    fn directory_with(&self, authority: &str, created: u64, expires: u64) -> SignedDirectory {
        let keys: Vec<&SigningKey> = std::iter::once(&self.signing)
            .chain(self.published.iter())
            .collect();
        let body = directory_body(&keys);
        let digest = format!(
            "sha-256=:{}:",
            STANDARD.encode(Sha256::digest(body.as_bytes()))
        );
        let authority = authority.to_ascii_lowercase();
        let mut inputs = Vec::new();
        let mut signatures = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            let label = format!("binding{index}");
            let params = format!(
                "(\"@authority\";req \"content-digest\");created={created};expires={expires};\
                 keyid=\"{}\";tag=\"{DIRECTORY_TAG}\"",
                jwk_thumbprint(key)
            );
            let base = format!(
                "\"@authority\";req: {authority}\n\"content-digest\": {digest}\n\
                 \"@signature-params\": {params}"
            );
            signatures.push(format!("{label}=:{}:", self.sign(key, &base)));
            inputs.push(format!("{label}={params}"));
        }
        SignedDirectory {
            body,
            headers: vec![
                ("content-digest", digest),
                ("signature-input", inputs.join(", ")),
                ("signature", signatures.join(", ")),
            ],
        }
    }

    /// An empty directory, served when signing is not configured.
    pub fn empty_directory_body() -> &'static str {
        "{\"keys\":[]}"
    }

    fn sign(&self, key: &SigningKey, base: &str) -> String {
        // RFC 8941 byte sequences are standard base64 with padding.
        STANDARD.encode(key.sign(base.as_bytes()).to_bytes())
    }
}

fn signing_key_from_seed(seed: &str) -> Result<SigningKey, WebBotAuthError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(seed.trim_end_matches('='))
        .map_err(|_| WebBotAuthError::InvalidSeed)?;
    let seed: [u8; 32] = bytes.try_into().map_err(|_| WebBotAuthError::InvalidSeed)?;
    Ok(SigningKey::from_bytes(&seed))
}

/// Serialize the agent URL as an sf-string, origin only.
fn serialize_signature_agent(url: &str) -> Result<String, WebBotAuthError> {
    let parsed = url::Url::parse(url).map_err(|_| WebBotAuthError::InvalidSignatureAgent)?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(WebBotAuthError::InvalidSignatureAgent);
    }
    let origin = parsed.origin().ascii_serialization();
    // An origin holds no `"` or `\`, so it needs no sf-string escaping.
    Ok(format!("\"{origin}\""))
}

/// RFC 9421 section 2.2.3: lowercase host, port only when not the default.
fn authority(url: &url::Url) -> Result<String, WebBotAuthError> {
    let host = url
        .host_str()
        .ok_or(WebBotAuthError::UnsignableUrl("URL has no host"))?
        .to_ascii_lowercase();
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

fn public_x(key: &SigningKey) -> String {
    URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes())
}

/// RFC 7638 / RFC 8037 appendix A.3 thumbprint of an Ed25519 key.
fn jwk_thumbprint(key: &SigningKey) -> String {
    let canonical = format!(
        "{{\"crv\":\"Ed25519\",\"kty\":\"OKP\",\"x\":\"{}\"}}",
        public_x(key)
    );
    URL_SAFE_NO_PAD.encode(Sha256::digest(canonical.as_bytes()))
}

fn directory_body(keys: &[&SigningKey]) -> String {
    let entries: Vec<String> = keys
        .iter()
        .map(|key| {
            format!(
                "{{\"kty\":\"OKP\",\"crv\":\"Ed25519\",\"kid\":\"{}\",\"x\":\"{}\",\"use\":\"sig\"}}",
                jwk_thumbprint(key),
                public_x(key)
            )
        })
        .collect();
    format!("{{\"keys\":[{}]}}", entries.join(","))
}

fn unix_now() -> Result<u64, WebBotAuthError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|_| WebBotAuthError::Clock)
}

#[cfg(test)]
#[path = "web_bot_auth_tests.rs"]
mod tests;
