//! ES256 keys for a Poppy personal agent: the agent's signing key (client
//! assertions) and each person's DPoP key at a company.
//!
//! Decision: a key is its PKCS#8 bytes. Whoever stores one (the server, a
//! Framework app) keeps those bytes and seals them as it sees fit; this type
//! only signs.

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use base64::Engine as _;
use jsonwebtoken::{EncodingKey, Header};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// A P-256 key that signs ES256 JWTs.
pub struct PoppyKey {
    kid: String,
    encoding: EncodingKey,
    public_jwk: Value,
    pkcs8: Vec<u8>,
}

impl std::fmt::Debug for PoppyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoppyKey")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

impl PoppyKey {
    /// A new random key.
    pub fn generate() -> Result<Self, String> {
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .map_err(|_| "could not generate a P-256 key".to_string())?;
        Self::from_pkcs8(pkcs8.as_ref())
    }

    /// The key stored as `pkcs8`.
    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Self, String> {
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8)
            .map_err(|_| "not a P-256 PKCS#8 key".to_string())?;
        // Uncompressed SEC1 point: 0x04 || x || y.
        let point = pair.public_key().as_ref();
        let (x, y) = (b64(&point[1..33]), b64(&point[33..65]));
        // RFC 7638 thumbprint: the required members in lexicographic order.
        let thumbprint = format!(r#"{{"crv":"P-256","kty":"EC","x":"{x}","y":"{y}"}}"#);
        let kid = b64(&Sha256::digest(thumbprint.as_bytes()));
        Ok(Self {
            public_jwk: json!({
                "kty": "EC", "crv": "P-256", "alg": "ES256", "use": "sig",
                "kid": kid, "x": x, "y": y,
            }),
            kid,
            encoding: EncodingKey::from_ec_der(pkcs8),
            pkcs8: pkcs8.to_vec(),
        })
    }

    /// The private key, to store.
    pub fn pkcs8(&self) -> &[u8] {
        &self.pkcs8
    }

    /// RFC 7638 thumbprint, used as the `kid`.
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// The public key with `kid`, `alg` and `use`, as a JWKS lists it.
    pub fn public_jwk(&self) -> &Value {
        &self.public_jwk
    }

    /// A JWK Set holding this key alone.
    pub fn jwks(&self) -> Value {
        json!({ "keys": [self.public_jwk] })
    }

    /// The public key alone (`kty`, `crv`, `x`, `y`), as a DPoP proof's
    /// header carries it.
    pub fn public_key_jwk(&self) -> Value {
        let jwk = &self.public_jwk;
        json!({ "kty": jwk["kty"], "crv": jwk["crv"], "x": jwk["x"], "y": jwk["y"] })
    }

    /// Sign `claims` with a caller-built header.
    pub fn sign_with_header<T: Serialize>(
        &self,
        header: &Header,
        claims: &T,
    ) -> Result<String, String> {
        jsonwebtoken::encode(header, claims, &self.encoding).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_key_is_the_same_key() {
        let key = PoppyKey::generate().unwrap();
        let again = PoppyKey::from_pkcs8(key.pkcs8()).unwrap();
        assert_eq!(again.kid(), key.kid());
        assert_eq!(key.jwks()["keys"][0]["kid"], key.kid());
        assert!(key.public_key_jwk().get("d").is_none());
        assert!(PoppyKey::from_pkcs8(b"not a key").is_err());
    }
}
