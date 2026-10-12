// Personal agent identity (Poppy spec 4.1, 4.2): a `client_id` is the HTTPS
// URL of the agent's client metadata document, and the agent proves itself
// with JWTs signed by a key from that document's `jwks_uri`.
//
// Design Decisions:
// - The document must name itself (`client_id` equals the URL it came from),
//   and its `jwks_uri` must be HTTPS on the same host, so a document cannot
//   borrow another party's keys.
// - Asymmetric algorithms only. A JWT whose `typ` names another Poppy token
//   (`poppy-browser+jwt`, `dpop+jwt`) is never accepted as an assertion.
// - `jti` is required and single use within the replay window, which is
//   longer than the longest lifetime accepted (`exp - iat` at most 300 s).
// THREAT[TM-POPPY-001, TM-POPPY-002].

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde_json::Value;

use super::PoppyState;

/// Clock skew allowed on `iat` and `exp`.
const CLOCK_SKEW_SECS: i64 = 30;
/// Longest assertion lifetime accepted, `exp - iat`.
const MAX_LIFETIME_SECS: i64 = 300;
/// Longest `sub` (User ID) or `jti` accepted.
const MAX_CLAIM_LEN: usize = 256;

/// A personal agent whose metadata document checked out.
#[derive(Debug, Clone)]
pub(super) struct Client {
    pub client_id: String,
    pub name: Option<String>,
    pub jwks: JwkSet,
}

/// Fetch and check the client metadata document and keys of `client_id`.
pub(super) async fn load(state: &PoppyState, client_id: &str) -> Result<Client, ()> {
    let document = state
        .verifier
        .public_document(client_id)
        .await
        .map_err(|_| ())?;
    let (jwks_uri, name) = check_document(client_id, &document)?;
    let keys = state
        .verifier
        .public_document(&jwks_uri)
        .await
        .map_err(|_| ())?;
    let jwks: JwkSet = serde_json::from_value((*keys).clone()).map_err(|_| ())?;
    Ok(Client {
        client_id: client_id.to_string(),
        name,
        jwks,
    })
}

/// Pure: the `jwks_uri` and display name of a valid document for
/// `client_id`.
fn check_document(client_id: &str, document: &Value) -> Result<(String, Option<String>), ()> {
    let client_url = url::Url::parse(client_id).map_err(|_| ())?;
    if client_url.scheme() != "https" || client_url.fragment().is_some() {
        return Err(());
    }
    if document.get("client_id").and_then(Value::as_str) != Some(client_id) {
        return Err(());
    }
    if let Some(method) = document.get("token_endpoint_auth_method")
        && method.as_str() != Some("private_key_jwt")
    {
        return Err(());
    }
    let jwks_uri = document.get("jwks_uri").and_then(Value::as_str).ok_or(())?;
    let jwks_url = url::Url::parse(jwks_uri).map_err(|_| ())?;
    if jwks_url.scheme() != "https" || jwks_url.host_str() != client_url.host_str() {
        return Err(());
    }
    let name = document
        .get("client_name")
        .and_then(Value::as_str)
        .map(|name| name.chars().take(100).collect::<String>())
        .filter(|name| !name.trim().is_empty());
    Ok((jwks_uri.to_string(), name))
}

/// What an assertion's `aud` must be.
#[derive(Debug, Clone, Copy)]
pub(super) enum Audience<'a> {
    /// Exactly this string, not an array (spec 4.2, Session assertions).
    Exactly(&'a str),
    /// Any of these, as a string or in an array (RFC 7523 §3, client
    /// assertions name the token endpoint or the issuer).
    AnyOf(&'a [&'a str]),
}

/// The claims of a checked assertion.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Assertion {
    pub subject: String,
    pub jti: String,
}

/// Check an assertion `client` signed: signature, `iss` (the `client_id`),
/// `sub`, `aud`, lifetime and `jti`, at time `now`. The `jti` replay check is
/// the caller's.
pub(super) fn check_assertion(
    token: &str,
    client: &Client,
    audience: Audience<'_>,
    now: i64,
) -> Result<Assertion, ()> {
    let header = decode_header(token).map_err(|_| ())?;
    if !matches!(
        header.alg,
        Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::RS256
            | Algorithm::PS256
            | Algorithm::EdDSA
    ) {
        return Err(());
    }
    if matches!(
        header.typ.as_deref(),
        Some("poppy-browser+jwt" | "dpop+jwt")
    ) {
        return Err(());
    }
    // A JWT without `kid` is accepted only when the agent publishes one key.
    let jwk = match header.kid.as_deref() {
        Some(kid) => client
            .jwks
            .keys
            .iter()
            .find(|jwk| jwk.common.key_id.as_deref() == Some(kid)),
        None if client.jwks.keys.len() == 1 => client.jwks.keys.first(),
        None => None,
    }
    .ok_or(())?;
    let key = DecodingKey::from_jwk(jwk).map_err(|_| ())?;
    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[client.client_id.as_str()]);
    validation.set_required_spec_claims(&["exp", "iat", "iss", "aud", "sub"]);
    validation.validate_exp = false;
    validation.validate_aud = false;
    let claims = decode::<Value>(token, &key, &validation)
        .map_err(|_| ())?
        .claims;
    let aud = claims.get("aud").ok_or(())?;
    let audience_ok = match audience {
        Audience::Exactly(expected) => aud.as_str() == Some(expected),
        Audience::AnyOf(allowed) => match aud {
            Value::String(aud) => allowed.contains(&aud.as_str()),
            Value::Array(auds) => auds
                .iter()
                .filter_map(Value::as_str)
                .any(|aud| allowed.contains(&aud)),
            _ => false,
        },
    };
    if !audience_ok {
        return Err(());
    }
    let iat = claims.get("iat").and_then(Value::as_i64).ok_or(())?;
    let exp = claims.get("exp").and_then(Value::as_i64).ok_or(())?;
    if iat > now + CLOCK_SKEW_SECS || exp + CLOCK_SKEW_SECS < now || exp - iat > MAX_LIFETIME_SECS {
        return Err(());
    }
    let text = |name: &str| {
        claims
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= MAX_CLAIM_LEN)
            .map(str::to_string)
            .ok_or(())
    };
    Ok(Assertion {
        subject: text("sub")?,
        jti: text("jti")?,
    })
}

#[cfg(test)]
pub(crate) mod test_agent {
    //! A personal agent with an ES256 key, minted per test run.
    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
    use base64::Engine as _;
    use serde_json::{Value, json};

    pub(crate) const CLIENT_ID: &str = "https://pa.example/agent.json";

    pub(crate) struct AgentKey {
        encoding: jsonwebtoken::EncodingKey,
        pub jwks: Value,
    }

    impl AgentKey {
        pub fn generate() -> Self {
            let pkcs8 = EcdsaKeyPair::generate_pkcs8(
                &ECDSA_P256_SHA256_FIXED_SIGNING,
                &SystemRandom::new(),
            )
            .unwrap();
            let pair =
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref()).unwrap();
            let point = pair.public_key().as_ref();
            let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
            Self {
                encoding: jsonwebtoken::EncodingKey::from_ec_der(pkcs8.as_ref()),
                jwks: json!({ "keys": [{
                    "kty": "EC", "crv": "P-256", "alg": "ES256", "kid": "k1",
                    "x": b64(&point[1..33]), "y": b64(&point[33..65]),
                }] }),
            }
        }

        pub fn sign(&self, typ: Option<&str>, claims: &Value) -> String {
            let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
            header.kid = Some("k1".into());
            header.typ = typ.map(str::to_string);
            jsonwebtoken::encode(&header, claims, &self.encoding).unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_agent::{AgentKey, CLIENT_ID};
    use super::*;
    use serde_json::json;

    const TOKEN_URL: &str = "https://co.example/v1/channels/c/poppy/oauth/token";

    fn client(key: &AgentKey) -> Client {
        Client {
            client_id: CLIENT_ID.into(),
            name: None,
            jwks: serde_json::from_value(key.jwks.clone()).unwrap(),
        }
    }

    fn claims(now: i64) -> Value {
        json!({
            "iss": CLIENT_ID, "sub": "usr_1", "aud": TOKEN_URL,
            "iat": now, "exp": now + 60, "jti": "a1b2c3d4e5f6a7b8",
        })
    }

    #[test]
    fn accepts_a_session_assertion() {
        let key = AgentKey::generate();
        let now = 1_790_000_000;
        let assertion = check_assertion(
            &key.sign(None, &claims(now)),
            &client(&key),
            Audience::Exactly(TOKEN_URL),
            now,
        )
        .unwrap();
        assert_eq!(assertion.subject, "usr_1");
    }

    #[test]
    fn rejects_bad_claims_and_browser_assertions() {
        let key = AgentKey::generate();
        let client = client(&key);
        let now = 1_790_000_000;
        let check = |token: String| {
            check_assertion(&token, &client, Audience::Exactly(TOKEN_URL), now).is_err()
        };
        for (name, value) in [
            ("iss", json!("https://other.example/agent.json")),
            ("aud", json!([TOKEN_URL])),
            ("aud", json!("https://other.example/token")),
            ("exp", json!(now + 600)),
            ("iat", json!(now + 120)),
            ("sub", json!("")),
        ] {
            let mut claims = claims(now);
            claims[name] = value;
            assert!(check(key.sign(None, &claims)), "{name}");
        }
        let mut no_jti = claims(now);
        no_jti.as_object_mut().unwrap().remove("jti");
        assert!(check(key.sign(None, &no_jti)));
        assert!(check(key.sign(Some("poppy-browser+jwt"), &claims(now))));
        // A client assertion may name the token endpoint inside an array.
        let mut listed = claims(now);
        listed["aud"] = json!([TOKEN_URL]);
        assert!(
            check_assertion(
                &key.sign(None, &listed),
                &client,
                Audience::AnyOf(&[TOKEN_URL]),
                now
            )
            .is_ok()
        );
        let other = AgentKey::generate();
        assert!(check(other.sign(None, &claims(now))));
    }

    #[test]
    fn documents_must_name_themselves_and_keep_keys_on_their_host() {
        let ok = json!({
            "client_id": CLIENT_ID, "client_name": "PA",
            "jwks_uri": "https://pa.example/jwks.json",
            "token_endpoint_auth_method": "private_key_jwt",
        });
        assert_eq!(
            check_document(CLIENT_ID, &ok).unwrap(),
            ("https://pa.example/jwks.json".into(), Some("PA".into()))
        );
        for (name, value) in [
            ("client_id", json!("https://pa.example/other.json")),
            ("jwks_uri", json!("https://evil.example/jwks.json")),
            ("jwks_uri", json!("http://pa.example/jwks.json")),
            ("token_endpoint_auth_method", json!("client_secret_basic")),
        ] {
            let mut document = ok.clone();
            document[name] = value;
            assert!(check_document(CLIENT_ID, &document).is_err(), "{name}");
        }
        let mut http = ok.clone();
        http["client_id"] = json!("http://pa.example/agent.json");
        assert!(check_document("http://pa.example/agent.json", &http).is_err());
    }
}
