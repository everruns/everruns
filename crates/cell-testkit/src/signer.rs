//! A control plane's org token signing key and its JWKS, for tests of the
//! gateway's org token check (`everruns_cell::org_token`).

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use base64::Engine as _;
use everruns_cell::org_token::OrgTokenClaims;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, EncodingKey, Header};

/// An Ed25519 signing key named `kid`.
pub struct Signer {
    pkcs8: Vec<u8>,
    public: Vec<u8>,
    pub kid: String,
}

impl Signer {
    pub fn new(kid: &str) -> anyhow::Result<Self> {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Self {
            public: pair.public_key().as_ref().to_vec(),
            pkcs8: pkcs8.as_ref().to_vec(),
            kid: kid.into(),
        })
    }

    /// The JWKS the control plane serves at `/.well-known/jwks.json`.
    pub fn jwks(&self) -> anyhow::Result<JwkSet> {
        let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&self.public);
        Ok(serde_json::from_value(serde_json::json!({
            "keys": [{ "kty": "OKP", "crv": "Ed25519", "x": x, "kid": self.kid, "alg": "EdDSA", "use": "sig" }]
        }))?)
    }

    /// A token for `org` on data plane `dp` with `scope`, valid `ttl_secs`
    /// (negative: already expired).
    pub fn token(&self, org: &str, dp: &str, scope: &str, ttl_secs: i64) -> anyhow::Result<String> {
        let now = chrono::Utc::now().timestamp();
        let claims = OrgTokenClaims {
            sub: "user-1".into(),
            org: org.into(),
            dp: dp.into(),
            scope: scope.into(),
            iat: now,
            exp: now + ttl_secs,
            jti: uuid::Uuid::new_v4().to_string(),
        };
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(self.kid.clone());
        Ok(jsonwebtoken::encode(
            &header,
            &claims,
            &EncodingKey::from_ed_der(&self.pkcs8),
        )?)
    }
}
