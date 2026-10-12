// Shared auth verifier for public agent channels.
//
// Decision: token verification (OIDC/JWKS, introspection, claim requirements,
// shared secrets, mTLS) lives in `everruns_core::channel_auth` so serve checks
// credentials with the same code. The server keeps what needs its storage:
// HTTP Basic passwords are checked against the server's Argon2 hashes, which
// this wrapper installs on every verifier it builds.

use std::ops::Deref;
use std::sync::Arc;

pub use everruns_core::channel_auth::{
    AGENTID_PROVIDER, ChannelAuthError, ChannelAuthPrincipal, LegacyChannelAuth, OIDC_PROVIDER,
    extract_bearer, verify_agentid_claims,
};

use crate::storage::password::verify_password;

/// [`everruns_core::channel_auth::ChannelAuthVerifier`] with the server's
/// password store behind HTTP Basic.
#[derive(Clone, Debug)]
pub struct ChannelAuthVerifier(everruns_core::channel_auth::ChannelAuthVerifier);

impl Default for ChannelAuthVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelAuthVerifier {
    pub fn new() -> Self {
        Self(
            everruns_core::channel_auth::ChannelAuthVerifier::new().with_password_check(Arc::new(
                |password, hash| verify_password(password, hash).map_err(|_| ()),
            )),
        )
    }
}

impl Deref for ChannelAuthVerifier {
    type Target = everruns_core::channel_auth::ChannelAuthVerifier;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::agent_channels::record::{
        ChannelAuthConfig, ChannelAuthMode, ChannelAuthProviderConfig, ChannelAuthRequirements,
    };
    use crate::storage::password::hash_password;
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

    fn basic_auth(hash: String) -> ChannelAuthConfig {
        ChannelAuthConfig {
            mode: ChannelAuthMode::HttpBasic,
            provider: Some(ChannelAuthProviderConfig::HttpBasic {
                username: "example".to_string(),
                password: None,
                password_hash: Some(hash),
                password_configured: false,
            }),
            requirements: ChannelAuthRequirements::default(),
        }
    }

    fn basic_header(password: &str) -> HeaderMap {
        let credentials = BASE64_STANDARD.encode(format!("example:{password}"));
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("basic {credentials}")).unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn basic_auth_verifies_argon2_password_hash() {
        let auth = basic_auth(hash_password("YExample0").unwrap());
        let verifier = ChannelAuthVerifier::new();
        assert!(
            verifier
                .verify(
                    &auth,
                    &basic_header("YExample0"),
                    LegacyChannelAuth::default()
                )
                .await
                .is_ok()
        );
        assert_eq!(
            verifier
                .verify(&auth, &basic_header("wrong"), LegacyChannelAuth::default())
                .await
                .unwrap_err(),
            ChannelAuthError::Unauthorized
        );
    }
}

/// AgentID-shaped ES256 test keys, generated per test so no private key lives
/// in the repo.
#[cfg(test)]
pub(crate) mod test_keys {
    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _};
    use base64::Engine;
    use jsonwebtoken::jwk::JwkSet;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde_json::Value;

    pub(crate) struct Signer {
        encoding: EncodingKey,
        pub(crate) jwks: JwkSet,
    }

    pub(crate) fn signer() -> Signer {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref()).unwrap();
        let point = pair.public_key().as_ref();
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let jwks: JwkSet = serde_json::from_value(serde_json::json!({
            "keys": [{
                "kty": "EC", "crv": "P-256", "kid": "k1", "alg": "ES256", "use": "sig",
                "x": b64(&point[1..33]), "y": b64(&point[33..65]),
            }]
        }))
        .unwrap();
        Signer {
            encoding: EncodingKey::from_ec_der(pkcs8.as_ref()),
            jwks,
        }
    }

    pub(crate) fn token(signer: &Signer, claims: Value) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("k1".to_string());
        encode(&header, &claims, &signer.encoding).unwrap()
    }
}
