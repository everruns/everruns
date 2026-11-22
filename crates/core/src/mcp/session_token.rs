//! Session tokens: how a data plane vouches for a session to an injected
//! capability's server.
//!
//! Decisions:
//! - EdDSA (Ed25519), the key type of the data plane's org tokens, so a
//!   data plane verifies and publishes both with one JWKS shape.
//! - One token per call, minted just before the request and never stored.
//!   The TTL ([`SESSION_TOKEN_TTL_SECS`]) only has to cover one request and
//!   clock skew.
//! - The audience is the exact server URL, and verification requires it, the
//!   `typ` header and the `cap` claim the server serves. A token for one
//!   server cannot be replayed at another.
//! - The signer and the verifier never share a process in production: the
//!   data plane signs, the server verifies against the data plane's published
//!   JWKS. Both live here so every host and test uses the same code.

use crate::McpServerAuthMode;
use crate::mcp::auth::{McpAuthProvider, McpAuthRequest, McpCredential};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use everruns_contracts::runtime::capabilities::injected::{
    SESSION_TOKEN_TTL_SECS, SESSION_TOKEN_TYP, SessionActor, SessionTokenClaims,
    injected_capability_id,
};
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Allowed clock skew between the data plane and the server, in seconds.
const LEEWAY_SECS: u64 = 30;

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// What a host asks a minter for: a token for one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTokenRequest {
    pub session_id: uuid::Uuid,
    /// The injected capability id (`injected:{name}`).
    pub capability_id: String,
    /// The server URL the call goes to.
    pub audience: String,
}

/// Mints session tokens. The data plane implements it; a host that runs
/// sessions for it asks through this trait instead of holding the key.
#[async_trait]
pub trait SessionTokenMinter: Send + Sync {
    async fn mint(&self, request: &SessionTokenRequest) -> Result<String>;
}

/// An Ed25519 signing key and the JWKS that verifies it.
pub struct SessionTokenSigner {
    issuer: String,
    kid: String,
    encoding: EncodingKey,
    public_jwk: Value,
    pkcs8: Vec<u8>,
}

/// PKCS#8 v1 (RFC 8410) wrapping of a 32-byte Ed25519 seed: the DER header
/// before the seed. The only private key form `jsonwebtoken` signs EdDSA with.
const PKCS8_ED25519_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

impl SessionTokenSigner {
    /// A fresh key.
    pub fn generate(issuer: impl Into<String>) -> Result<Self> {
        let seed: [u8; 32] = rand::random();
        let mut pkcs8 = PKCS8_ED25519_PREFIX.to_vec();
        pkcs8.extend_from_slice(&seed);
        Self::from_pkcs8(issuer, &pkcs8)
    }

    /// A stored key, as [`Self::pkcs8`] returned it.
    pub fn from_pkcs8(issuer: impl Into<String>, pkcs8: &[u8]) -> Result<Self> {
        let seed: [u8; 32] = pkcs8
            .strip_prefix(PKCS8_ED25519_PREFIX.as_slice())
            .and_then(|seed| seed.try_into().ok())
            .ok_or_else(|| anyhow!("not an Ed25519 PKCS#8 key"))?;
        let public = SigningKey::from_bytes(&seed).verifying_key();
        let x = b64(public.as_bytes());
        // RFC 7638 thumbprint: required members in lexicographic order.
        let thumbprint = format!(r#"{{"crv":"Ed25519","kty":"OKP","x":"{x}"}}"#);
        let kid = b64(&Sha256::digest(thumbprint.as_bytes()));
        Ok(Self {
            issuer: issuer.into(),
            public_jwk: json!({
                "kty": "OKP", "crv": "Ed25519", "alg": "EdDSA", "use": "sig",
                "kid": kid, "x": x,
            }),
            kid,
            encoding: EncodingKey::from_ed_der(pkcs8),
            pkcs8: pkcs8.to_vec(),
        })
    }

    /// The private key, for the host to store encrypted.
    pub fn pkcs8(&self) -> &[u8] {
        &self.pkcs8
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The JWKS to publish at the data plane's `/.well-known/jwks.json`.
    pub fn jwks(&self) -> Value {
        json!({ "keys": [self.public_jwk] })
    }

    /// Sign a token for one call.
    pub fn sign(&self, request: &SessionTokenRequest, group: Option<String>) -> Result<String> {
        self.sign_as(request, group, None)
    }

    /// Sign a token for one call of the turn `actor` opened.
    pub fn sign_as(
        &self,
        request: &SessionTokenRequest,
        group: Option<String>,
        actor: Option<SessionActor>,
    ) -> Result<String> {
        let now = now_secs();
        let claims = SessionTokenClaims {
            iss: self.issuer.clone(),
            aud: request.audience.clone(),
            sub: request.session_id.to_string(),
            cap: request.capability_id.clone(),
            grp: group,
            act: actor,
            iat: now,
            exp: now + SESSION_TOKEN_TTL_SECS,
            jti: uuid::Uuid::now_v7().to_string(),
        };
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(self.kid.clone());
        header.typ = Some(SESSION_TOKEN_TYP.to_string());
        Ok(jsonwebtoken::encode(&header, &claims, &self.encoding)?)
    }
}

/// Looks up the group label value of a session (its organization in an
/// Everruns deployment).
pub type SessionGroupLookup = Arc<dyn Fn(uuid::Uuid) -> Option<String> + Send + Sync>;

/// A minter over a key the host holds itself.
pub struct LocalSessionTokenMinter {
    signer: Arc<SessionTokenSigner>,
    group_of: SessionGroupLookup,
}

impl LocalSessionTokenMinter {
    pub fn new(signer: Arc<SessionTokenSigner>, group_of: SessionGroupLookup) -> Self {
        Self { signer, group_of }
    }
}

#[async_trait]
impl SessionTokenMinter for LocalSessionTokenMinter {
    async fn mint(&self, request: &SessionTokenRequest) -> Result<String> {
        self.signer
            .sign(request, (self.group_of)(request.session_id))
    }
}

/// Verifies session tokens against a data plane's JWKS.
#[derive(Clone)]
pub struct SessionTokenVerifier {
    keys: Vec<(Option<String>, DecodingKey)>,
    issuers: Vec<String>,
}

impl SessionTokenVerifier {
    /// Trust the EdDSA keys in `jwks`, from any of `issuers`.
    pub fn from_jwks(jwks: &Value, issuers: Vec<String>) -> Result<Self> {
        let set: JwkSet = serde_json::from_value(jwks.clone()).context("invalid JWKS")?;
        let mut verifier = Self {
            keys: Vec::new(),
            issuers,
        };
        for jwk in &set.keys {
            verifier.add_jwk(jwk)?;
        }
        Ok(verifier)
    }

    /// Add more trusted keys, for an install with several data planes.
    pub fn extend(&mut self, other: SessionTokenVerifier) {
        self.keys.extend(other.keys);
        for issuer in other.issuers {
            if !self.issuers.contains(&issuer) {
                self.issuers.push(issuer);
            }
        }
    }

    fn add_jwk(&mut self, jwk: &Jwk) -> Result<()> {
        self.keys
            .push((jwk.common.key_id.clone(), DecodingKey::from_jwk(jwk)?));
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Verify a token minted for `audience` (this server's URL) and
    /// `capability_id`. Every failure is an error with no detail worth
    /// returning to the caller.
    pub fn verify(
        &self,
        token: &str,
        audience: &str,
        capability_id: &str,
    ) -> Result<SessionTokenClaims> {
        let header = jsonwebtoken::decode_header(token).context("malformed token")?;
        if header.alg != Algorithm::EdDSA || header.typ.as_deref() != Some(SESSION_TOKEN_TYP) {
            bail!("not a session token");
        }
        let candidates: Vec<&DecodingKey> = match header.kid.as_deref() {
            Some(kid) => self
                .keys
                .iter()
                .filter(|(key_id, _)| key_id.as_deref() == Some(kid))
                .map(|(_, key)| key)
                .collect(),
            None => self.keys.iter().map(|(_, key)| key).collect(),
        };
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.leeway = LEEWAY_SECS;
        validation.set_audience(&[audience]);
        validation.set_issuer(&self.issuers);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        for key in candidates {
            if let Ok(data) = jsonwebtoken::decode::<SessionTokenClaims>(token, key, &validation) {
                if data.claims.cap != capability_id {
                    bail!("token is for another capability");
                }
                if data.claims.iat > now_secs() + LEEWAY_SECS {
                    bail!("token issued in the future");
                }
                return Ok(data.claims);
            }
        }
        bail!("invalid session token")
    }
}

/// An auth provider that mints a session token for injected capabilities'
/// servers ([`McpServerAuthMode::SessionToken`]) and asks `inner` for every
/// other server.
pub struct SessionTokenAuthProvider {
    inner: Arc<dyn McpAuthProvider>,
    minter: Arc<dyn SessionTokenMinter>,
}

impl SessionTokenAuthProvider {
    pub fn new(inner: Arc<dyn McpAuthProvider>, minter: Arc<dyn SessionTokenMinter>) -> Self {
        Self { inner, minter }
    }
}

#[async_trait]
impl McpAuthProvider for SessionTokenAuthProvider {
    async fn authorization(&self, request: &McpAuthRequest<'_>) -> Result<Option<McpCredential>> {
        if request.auth_mode != McpServerAuthMode::SessionToken {
            return self.inner.authorization(request).await;
        }
        // Fail closed: a session-token server never gets a request without
        // one, and never outside a session.
        let session_id = request.session_id.ok_or_else(|| {
            anyhow!(
                "injected capability '{}' needs a session",
                request.server_name
            )
        })?;
        let audience = request
            .server_url
            .ok_or_else(|| anyhow!("injected capability '{}' has no URL", request.server_name))?;
        let token = self
            .minter
            .mint(&SessionTokenRequest {
                session_id,
                // Injected servers are named after their capability.
                capability_id: injected_capability_id(request.server_name),
                audience: audience.to_string(),
            })
            .await?;
        Ok(Some(McpCredential::bearer(token)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::auth::StaticAuthProvider;

    const AUD: &str = "https://cp.example/v1/injected/knowledge/mcp";
    const CAP: &str = "injected:knowledge";

    fn request() -> SessionTokenRequest {
        SessionTokenRequest {
            session_id: uuid::Uuid::now_v7(),
            capability_id: CAP.into(),
            audience: AUD.into(),
        }
    }

    fn verifier_for(signer: &SessionTokenSigner) -> SessionTokenVerifier {
        SessionTokenVerifier::from_jwks(&signer.jwks(), vec![signer.issuer().to_string()]).unwrap()
    }

    #[test]
    fn signs_and_verifies() {
        let signer = SessionTokenSigner::generate("https://dp.example").unwrap();
        let request = request();
        let token = signer.sign(&request, Some("org_1".into())).unwrap();
        let claims = verifier_for(&signer).verify(&token, AUD, CAP).unwrap();
        assert_eq!(claims.sub, request.session_id.to_string());
        assert_eq!(claims.grp.as_deref(), Some("org_1"));
        assert_eq!(claims.cap, CAP);
        assert!(claims.exp - claims.iat == SESSION_TOKEN_TTL_SECS);
    }

    #[test]
    fn rejects_other_audience_capability_issuer_and_key() {
        let signer = SessionTokenSigner::generate("https://dp.example").unwrap();
        let token = signer.sign(&request(), None).unwrap();
        let verifier = verifier_for(&signer);
        assert!(
            verifier
                .verify(&token, "https://evil.example/mcp", CAP)
                .is_err()
        );
        assert!(verifier.verify(&token, AUD, "injected:poppy").is_err());

        let other_issuer =
            SessionTokenVerifier::from_jwks(&signer.jwks(), vec!["https://other".into()]).unwrap();
        assert!(other_issuer.verify(&token, AUD, CAP).is_err());

        let stranger = SessionTokenSigner::generate("https://dp.example").unwrap();
        assert!(verifier_for(&stranger).verify(&token, AUD, CAP).is_err());

        assert!(verifier.verify("not.a.token", AUD, CAP).is_err());
    }

    #[test]
    fn rejects_tokens_with_another_typ() {
        let signer = SessionTokenSigner::generate("https://dp.example").unwrap();
        let claims = json!({"iss": "https://dp.example", "aud": AUD, "sub": "s", "cap": CAP,
            "iat": now_secs(), "exp": now_secs() + 60, "jti": "j"});
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(signer.kid.clone());
        header.typ = Some("JWT".into());
        let token = jsonwebtoken::encode(&header, &claims, &signer.encoding).unwrap();
        assert!(verifier_for(&signer).verify(&token, AUD, CAP).is_err());
    }

    #[test]
    fn rejects_expired_tokens() {
        let signer = SessionTokenSigner::generate("https://dp.example").unwrap();
        let past = now_secs() - 3600;
        let claims = SessionTokenClaims {
            iss: "https://dp.example".into(),
            aud: AUD.into(),
            sub: "s".into(),
            cap: CAP.into(),
            grp: None,
            act: None,
            iat: past,
            exp: past + 60,
            jti: "j".into(),
        };
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(signer.kid.clone());
        header.typ = Some(SESSION_TOKEN_TYP.into());
        let token = jsonwebtoken::encode(&header, &claims, &signer.encoding).unwrap();
        assert!(verifier_for(&signer).verify(&token, AUD, CAP).is_err());
    }

    #[test]
    fn stored_key_round_trips() {
        let signer = SessionTokenSigner::generate("https://dp.example").unwrap();
        let reloaded =
            SessionTokenSigner::from_pkcs8("https://dp.example", signer.pkcs8()).unwrap();
        assert_eq!(reloaded.jwks(), signer.jwks());
        let token = reloaded.sign(&request(), None).unwrap();
        assert!(verifier_for(&signer).verify(&token, AUD, CAP).is_ok());
    }

    #[tokio::test]
    async fn provider_mints_only_for_session_token_servers() {
        let signer = Arc::new(SessionTokenSigner::generate("https://dp.example").unwrap());
        let minter = Arc::new(LocalSessionTokenMinter::new(
            signer.clone(),
            Arc::new(|_| Some("org_1".to_string())),
        ));
        let provider = SessionTokenAuthProvider::new(
            Arc::new(StaticAuthProvider::new().with_bearer("docs", "static")),
            minter,
        );
        let session = uuid::Uuid::now_v7();

        let docs = provider
            .authorization(&McpAuthRequest {
                server_url: Some("https://docs.example/mcp"),
                server_name: "docs",
                auth_mode: McpServerAuthMode::ApiKey,
                oauth_provider_id: None,
                session_id: Some(session),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(docs.authorization.as_deref(), Some("Bearer static"));

        let injected = provider
            .authorization(&McpAuthRequest {
                server_url: Some(AUD),
                server_name: "knowledge",
                auth_mode: McpServerAuthMode::SessionToken,
                oauth_provider_id: None,
                session_id: Some(session),
            })
            .await
            .unwrap()
            .unwrap();
        let token = injected
            .authorization
            .unwrap()
            .strip_prefix("Bearer ")
            .unwrap()
            .to_string();
        let claims = verifier_for(&signer).verify(&token, AUD, CAP).unwrap();
        assert_eq!(claims.sub, session.to_string());
        assert_eq!(claims.grp.as_deref(), Some("org_1"));

        let no_session = provider
            .authorization(&McpAuthRequest {
                server_url: Some(AUD),
                server_name: "knowledge",
                auth_mode: McpServerAuthMode::SessionToken,
                oauth_provider_id: None,
                session_id: None,
            })
            .await;
        assert!(no_session.is_err());
    }
}
