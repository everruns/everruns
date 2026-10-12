//! AgentID-shaped ES256 test keys, generated per test so no private key lives
//! in the repo.

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
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref()).unwrap();
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
