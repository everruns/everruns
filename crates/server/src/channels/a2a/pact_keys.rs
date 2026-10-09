// The ES256 key a PACT endpoint signs with (PACT 1.0 §5.4, §5.6): delegation
// tokens, consent-page sessions and receipts. One key per endpoint, generated
// on first use and published at the endpoint's `jwks_uri`.
//
// Design Decisions:
// - Per endpoint, not server-wide: a company's API trusts only its own
//   endpoint's key, so a token minted for one company never verifies at
//   another's.
// - Every token carries a `typ` naming what it is (`at+jwt`, the consent
//   session type), and verification requires it, so a consent session can
//   never be replayed as a delegation token or the other way round.
// - The private key is encrypted with the server's secrets key when one is
//   configured, the same as channel config.

use std::sync::Arc;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use base64::Engine as _;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, jwk::Jwk};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::storage::StorageBackend;
use crate::storage::encryption::EncryptionService;
use crate::storage::pact_delegation::PactSigningKeyRow;

pub(super) struct ProviderKey {
    kid: String,
    encoding: EncodingKey,
    decoding: DecodingKey,
    public_jwk: Value,
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

impl ProviderKey {
    /// The endpoint's key, generated and stored on first use.
    pub async fn for_channel(
        db: &StorageBackend,
        encryption: Option<&Arc<EncryptionService>>,
        channel_id: Uuid,
    ) -> anyhow::Result<Self> {
        let row = match db.pact_signing_key(channel_id).await? {
            Some(row) => row,
            None => {
                let row = Self::generate_row(encryption)?;
                db.insert_pact_signing_key(channel_id, &row).await?
            }
        };
        Self::from_row(&row, encryption)
    }

    fn generate_row(
        encryption: Option<&Arc<EncryptionService>>,
    ) -> anyhow::Result<PactSigningKeyRow> {
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .map_err(|_| anyhow::anyhow!("could not generate a P-256 key"))?;
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref())
            .map_err(|_| anyhow::anyhow!("could not parse the generated P-256 key"))?;
        // Uncompressed SEC1 point: 0x04 || x || y.
        let point = pair.public_key().as_ref();
        let (x, y) = (b64(&point[1..33]), b64(&point[33..65]));
        // RFC 7638 thumbprint: the required members in lexicographic order.
        let thumbprint = format!(r#"{{"crv":"P-256","kty":"EC","x":"{x}","y":"{y}"}}"#);
        let kid = b64(&Sha256::digest(thumbprint.as_bytes()));
        let (private_key, encrypted) = match encryption {
            Some(encryption) => (encryption.encrypt(pkcs8.as_ref())?, true),
            None => (pkcs8.as_ref().to_vec(), false),
        };
        Ok(PactSigningKeyRow {
            public_jwk: json!({
                "kty": "EC", "crv": "P-256", "alg": "ES256", "use": "sig",
                "kid": kid, "x": x, "y": y,
            }),
            kid,
            private_key,
            encrypted,
        })
    }

    fn from_row(
        row: &PactSigningKeyRow,
        encryption: Option<&Arc<EncryptionService>>,
    ) -> anyhow::Result<Self> {
        let pkcs8 = match (row.encrypted, encryption) {
            (false, _) => row.private_key.clone(),
            (true, Some(encryption)) => encryption.decrypt(&row.private_key)?,
            (true, None) => anyhow::bail!("PACT signing key is encrypted but encryption is off"),
        };
        let jwk: Jwk = serde_json::from_value(row.public_jwk.clone())?;
        Ok(Self {
            kid: row.kid.clone(),
            encoding: EncodingKey::from_ec_der(&pkcs8),
            decoding: DecodingKey::from_jwk(&jwk)?,
            public_jwk: row.public_jwk.clone(),
        })
    }

    pub fn jwks(&self) -> Value {
        json!({ "keys": [self.public_jwk] })
    }

    /// Sign `claims` as a compact JWS with header `typ`.
    pub fn sign<T: Serialize>(&self, typ: &str, claims: &T) -> anyhow::Result<String> {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.kid.clone());
        header.typ = Some(typ.to_string());
        Ok(jsonwebtoken::encode(&header, claims, &self.encoding)?)
    }

    /// Verify a token this key signed: signature, `typ`, `exp`, and the
    /// audience when one is given. Every failure is `None`.
    pub fn verify<T: DeserializeOwned>(
        &self,
        typ: &str,
        token: &str,
        audience: Option<&str>,
    ) -> Option<T> {
        let header = jsonwebtoken::decode_header(token).ok()?;
        if header.alg != Algorithm::ES256 || header.typ.as_deref() != Some(typ) {
            return None;
        }
        let mut validation = Validation::new(Algorithm::ES256);
        validation.leeway = 0;
        validation.set_required_spec_claims(&["exp"]);
        match audience {
            Some(audience) => validation.set_audience(&[audience]),
            None => validation.validate_aud = false,
        }
        jsonwebtoken::decode::<T>(token, &self.decoding, &validation)
            .ok()
            .map(|data| data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
    struct Claims {
        aud: String,
        exp: i64,
        sub: String,
    }

    fn key() -> ProviderKey {
        ProviderKey::from_row(&ProviderKey::generate_row(None).unwrap(), None).unwrap()
    }

    fn claims(aud: &str, exp_in: i64) -> Claims {
        Claims {
            aud: aud.into(),
            exp: chrono::Utc::now().timestamp() + exp_in,
            sub: "jane".into(),
        }
    }

    #[test]
    fn round_trips_and_publishes_its_kid() {
        let key = key();
        let token = key
            .sign("at+jwt", &claims("https://a.example", 60))
            .unwrap();
        let back: Claims = key
            .verify("at+jwt", &token, Some("https://a.example"))
            .unwrap();
        assert_eq!(back.sub, "jane");
        assert_eq!(key.jwks()["keys"][0]["kid"], key.kid);
        assert_eq!(
            jsonwebtoken::decode_header(&token).unwrap().kid,
            Some(key.kid)
        );
    }

    #[test]
    fn rejects_wrong_typ_audience_expiry_and_key() {
        let key = key();
        let token = key
            .sign("at+jwt", &claims("https://a.example", 60))
            .unwrap();
        assert!(
            key.verify::<Claims>("pact-consent+jwt", &token, None)
                .is_none()
        );
        assert!(
            key.verify::<Claims>("at+jwt", &token, Some("https://b.example"))
                .is_none()
        );
        let expired = key
            .sign("at+jwt", &claims("https://a.example", -5))
            .unwrap();
        assert!(key.verify::<Claims>("at+jwt", &expired, None).is_none());
        let other = self::key();
        assert!(other.verify::<Claims>("at+jwt", &token, None).is_none());
    }
}
