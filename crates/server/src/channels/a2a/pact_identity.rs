// PACT personal-agent identity (PACT 1.0 §3): the bearer token on every PACT
// request is a JWT the personal agent signs with its own key, checked against
// the JWKS of a personal agent the endpoint registered.
//
// Design Decisions:
// - The registered issuer is found from the token's own `iss` before the
//   signature is checked, because the issuer decides which keys to verify
//   with. Nothing else is read from an unverified token.
// - Only ES256 and RS256 are accepted (§3.2), so an HMAC token signed with a
//   public key as its secret can never verify.
// - The caller is the pair (issuer, `sub`). Its session tag is a hash of the
//   pair: `sub` is opaque per-user data that never needs to appear in a tag.
// - No `jti` replay tracking (§3.2 "Providers need not track replay"); the
//   short lifetime bound (`exp` at most 300 s after `iat`) is the limit.

use base64::Engine as _;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::api::channel_auth::{ChannelAuthVerifier, extract_bearer};
use crate::domains::agent_channels::record::PactProfileConfig;

/// Clock skew allowed on `iat` and `exp` (§3.2: "at most 30 s").
const CLOCK_SKEW_SECS: i64 = 30;
/// Longest token lifetime accepted, `exp - iat` (§3.2).
const MAX_TOKEN_LIFETIME_SECS: i64 = 300;

/// A user as a personal agent names them: the pair (personal agent, `sub`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PersonalAgentUser {
    pub issuer: String,
    pub subject: String,
}

impl PersonalAgentUser {
    /// Routing tag binding a conversation to this user, so a `contextId` from
    /// one user (or another personal agent) never continues in another's.
    pub fn session_tag(&self) -> String {
        let digest = Sha256::new()
            .chain_update(self.issuer.as_bytes())
            .chain_update([0u8])
            .chain_update(self.subject.as_bytes())
            .finalize();
        format!("a2a_caller:{}", &hex::encode(digest)[..32])
    }
}

/// Verify the request's personal-agent JWT against the endpoint's registered
/// personal agents. Every failure is the same `Err`, answered with one `401`.
/// THREAT[TM-A2A-016].
pub(super) async fn verify(
    verifier: &ChannelAuthVerifier,
    profile: &PactProfileConfig,
    headers: &axum::http::HeaderMap,
) -> Result<PersonalAgentUser, ()> {
    let token = extract_bearer(headers).ok_or(())?;
    let issuer = unverified_issuer(token).ok_or(())?;
    let agent = profile
        .personal_agents
        .iter()
        .find(|agent| agent.enabled && agent.issuer == issuer)
        .ok_or(())?;
    let jwks = match (agent.jwks.as_ref(), agent.jwks_uri.as_deref()) {
        (Some(inline), _) => {
            std::sync::Arc::new(serde_json::from_value::<JwkSet>(inline.clone()).map_err(|_| ())?)
        }
        (None, Some(uri)) => verifier.jwks(uri).await.map_err(|_| ())?,
        (None, None) => return Err(()),
    };
    verify_token(
        token,
        &agent.issuer,
        &profile.audience,
        &jwks,
        chrono::Utc::now().timestamp(),
    )
}

/// The `iss` claim of a token whose signature has not been checked yet. Used
/// only to pick which registered personal agent's keys to verify with.
fn unverified_issuer(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("iss")?.as_str().map(str::to_owned)
}

/// Check signature and claims (§3.2) at time `now` (Unix seconds).
fn verify_token(
    token: &str,
    issuer: &str,
    audience: &str,
    jwks: &JwkSet,
    now: i64,
) -> Result<PersonalAgentUser, ()> {
    let header = decode_header(token).map_err(|_| ())?;
    if !matches!(header.alg, Algorithm::ES256 | Algorithm::RS256) {
        return Err(());
    }
    // `kid` SHOULD name a published key; a token without one is accepted only
    // when the personal agent publishes exactly one key.
    let jwk = match header.kid.as_deref() {
        Some(kid) => jwks
            .keys
            .iter()
            .find(|jwk| jwk.common.key_id.as_deref() == Some(kid)),
        None if jwks.keys.len() == 1 => jwks.keys.first(),
        None => None,
    }
    .ok_or(())?;
    let key = DecodingKey::from_jwk(jwk).map_err(|_| ())?;
    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[audience]);
    validation.set_required_spec_claims(&["exp", "iat", "iss", "aud", "sub"]);
    validation.leeway = CLOCK_SKEW_SECS as u64;
    // `exp` is checked below against `now`, so tests can pin the clock.
    validation.validate_exp = false;
    let claims = decode::<Value>(token, &key, &validation)
        .map_err(|_| ())?
        .claims;
    let iat = claims.get("iat").and_then(Value::as_i64).ok_or(())?;
    let exp = claims.get("exp").and_then(Value::as_i64).ok_or(())?;
    if iat > now + CLOCK_SKEW_SECS
        || exp + CLOCK_SKEW_SECS < now
        || exp - iat > MAX_TOKEN_LIFETIME_SECS
    {
        return Err(());
    }
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|sub| !sub.is_empty())
        .ok_or(())?;
    Ok(PersonalAgentUser {
        issuer: issuer.to_string(),
        subject: subject.to_string(),
    })
}

#[cfg(test)]
pub(crate) mod test_keys {
    //! An ES256 personal-agent key minted per test run, so no private key is
    //! checked in.
    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
    use base64::Engine as _;
    use jsonwebtoken::{EncodingKey, Header};
    use serde_json::{Value, json};

    pub struct PersonalAgentKey {
        pub kid: String,
        encoding: EncodingKey,
        public_jwk: Value,
    }

    impl PersonalAgentKey {
        pub fn generate(kid: &str) -> Self {
            let pkcs8 = EcdsaKeyPair::generate_pkcs8(
                &ECDSA_P256_SHA256_FIXED_SIGNING,
                &SystemRandom::new(),
            )
            .expect("generate P-256 key");
            let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref())
                .expect("parse P-256 key");
            // Uncompressed SEC1 point: 0x04 || x || y.
            let point = pair.public_key().as_ref();
            let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
            Self {
                kid: kid.to_string(),
                encoding: EncodingKey::from_ec_der(pkcs8.as_ref()),
                public_jwk: json!({
                    "kty": "EC",
                    "crv": "P-256",
                    "alg": "ES256",
                    "use": "sig",
                    "kid": kid,
                    "x": b64(&point[1..33]),
                    "y": b64(&point[33..65]),
                }),
            }
        }

        pub fn jwks(&self) -> Value {
            json!({ "keys": [self.public_jwk] })
        }

        pub fn sign(&self, claims: &Value) -> String {
            let mut header = Header::new(jsonwebtoken::Algorithm::ES256);
            header.kid = Some(self.kid.clone());
            jsonwebtoken::encode(&header, claims, &self.encoding).expect("sign token")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_keys::PersonalAgentKey;
    use super::*;
    use serde_json::json;

    const ISSUER: &str = "https://pa.example";
    const AUDIENCE: &str = "everruns-pact";
    const NOW: i64 = 1_800_000_000;

    fn claims(overrides: Value) -> Value {
        let mut claims = json!({
            "iss": ISSUER,
            "aud": AUDIENCE,
            "sub": "user-1",
            "iat": NOW,
            "exp": NOW + 120,
        });
        for (key, value) in overrides.as_object().unwrap() {
            if value.is_null() {
                claims.as_object_mut().unwrap().remove(key);
            } else {
                claims[key] = value.clone();
            }
        }
        claims
    }

    fn check(key: &PersonalAgentKey, token: &str) -> Result<PersonalAgentUser, ()> {
        let jwks: JwkSet = serde_json::from_value(key.jwks()).unwrap();
        verify_token(token, ISSUER, AUDIENCE, &jwks, NOW)
    }

    #[test]
    fn accepts_a_well_formed_token() {
        let key = PersonalAgentKey::generate("k1");
        let user = check(&key, &key.sign(&claims(json!({})))).unwrap();
        assert_eq!(user.issuer, ISSUER);
        assert_eq!(user.subject, "user-1");
        assert_eq!(
            unverified_issuer(&key.sign(&claims(json!({})))).as_deref(),
            Some(ISSUER)
        );
    }

    #[test]
    fn rejects_bad_claims() {
        let key = PersonalAgentKey::generate("k1");
        for overrides in [
            json!({ "aud": "someone-else" }),
            json!({ "iss": "https://other.example" }),
            json!({ "iat": NOW + 31, "exp": NOW + 151 }),
            json!({ "iat": NOW - 200, "exp": NOW - 100 }),
            json!({ "exp": NOW + 301 }),
            json!({ "sub": "" }),
            json!({ "sub": null }),
            json!({ "iat": null }),
        ] {
            let token = key.sign(&claims(overrides.clone()));
            assert!(check(&key, &token).is_err(), "accepted {overrides}");
        }
    }

    #[test]
    fn allows_thirty_seconds_of_clock_skew() {
        let key = PersonalAgentKey::generate("k1");
        let ahead = key.sign(&claims(json!({ "iat": NOW + 30, "exp": NOW + 150 })));
        assert!(check(&key, &ahead).is_ok());
        let just_expired = key.sign(&claims(json!({ "iat": NOW - 140, "exp": NOW - 20 })));
        assert!(check(&key, &just_expired).is_ok());
    }

    #[test]
    fn rejects_a_key_the_agent_did_not_publish() {
        let published = PersonalAgentKey::generate("k1");
        let other = PersonalAgentKey::generate("k1");
        assert!(check(&published, &other.sign(&claims(json!({})))).is_err());
        let unknown_kid = PersonalAgentKey::generate("k2");
        assert!(check(&published, &unknown_kid.sign(&claims(json!({})))).is_err());
    }

    #[test]
    fn rejects_hmac_tokens() {
        let key = PersonalAgentKey::generate("k1");
        let mut header = jsonwebtoken::Header::new(Algorithm::HS256);
        header.kid = Some("k1".into());
        let token = jsonwebtoken::encode(
            &header,
            &claims(json!({})),
            &jsonwebtoken::EncodingKey::from_secret(b"not-an-allowed-platform-key"),
        )
        .unwrap();
        assert!(check(&key, &token).is_err());
    }

    #[test]
    fn session_tag_separates_users_and_agents() {
        let user = |issuer: &str, subject: &str| PersonalAgentUser {
            issuer: issuer.into(),
            subject: subject.into(),
        };
        let tag = user("a", "b").session_tag();
        assert!(tag.starts_with("a2a_caller:"));
        assert_eq!(tag, user("a", "b").session_tag());
        assert_ne!(tag, user("a", "c").session_tag());
        assert_ne!(tag, user("ab", "").session_tag());
    }
}
