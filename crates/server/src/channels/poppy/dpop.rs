// DPoP proofs (RFC 9449, Poppy spec 4.3): the personal agent signs a fresh
// proof for every request with a key it never publishes, and a Session Token
// works only together with a proof from the key it was issued to.
//
// Design Decisions:
// - Asymmetric algorithms only, so an HMAC proof signed with the public key
//   as its secret can never verify.
// - The key is identified by its RFC 7638 thumbprint, computed from the
//   proof header's own JSON, not from a re-serialised parsed key.
// - `iat` must be within one minute either way; `jti` is single use for the
//   replay window, per channel. No server nonce yet (spec: "MAY").
// THREAT[TM-POPPY-003].

use base64::Engine as _;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::Jwk};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// How far a proof's `iat` may be from the server's clock.
const IAT_WINDOW_SECS: i64 = 60;
/// Longest `jti` accepted.
const MAX_JTI_LEN: usize = 256;

/// A proof that passed every check but the `jti` replay check.
#[derive(Debug, PartialEq)]
pub(super) struct Proof {
    /// RFC 7638 thumbprint of the proof's key.
    pub jkt: String,
    pub jti: String,
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Check one proof for a request with `method` to `url`, at time `now`.
/// `access_token` is the Session Token sent with it, `None` at the token
/// endpoint. Every failure is `Err` with a short reason for the response.
pub(super) fn check(
    proof: &str,
    method: &str,
    url: &str,
    access_token: Option<&str>,
    now: i64,
) -> Result<Proof, &'static str> {
    let invalid = "Invalid DPoP proof";
    let header = decode_header(proof).map_err(|_| invalid)?;
    if !matches!(
        header.alg,
        Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::RS256
            | Algorithm::PS256
            | Algorithm::EdDSA
    ) {
        return Err("Unsupported DPoP proof algorithm");
    }
    if header.typ.as_deref() != Some("dpop+jwt") {
        return Err("DPoP proof typ must be dpop+jwt");
    }
    let raw_header: Value = proof
        .split('.')
        .next()
        .and_then(|part| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(part)
                .ok()
        })
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or(invalid)?;
    let raw_jwk = raw_header.get("jwk").ok_or("DPoP proof carries no jwk")?;
    // A private key in the header means the agent leaked it.
    if ["d", "p", "q", "dp", "dq", "qi", "k"]
        .iter()
        .any(|member| raw_jwk.get(member).is_some())
    {
        return Err("DPoP proof jwk must be a public key");
    }
    let jkt = thumbprint(raw_jwk).ok_or(invalid)?;
    let jwk: Jwk = serde_json::from_value(raw_jwk.clone()).map_err(|_| invalid)?;
    let key = DecodingKey::from_jwk(&jwk).map_err(|_| invalid)?;
    let mut validation = Validation::new(header.alg);
    validation.validate_exp = false;
    validation.validate_aud = false;
    validation.set_required_spec_claims::<&str>(&[]);
    let claims = decode::<Value>(proof, &key, &validation)
        .map_err(|_| invalid)?
        .claims;
    let text = |name: &str| claims.get(name).and_then(Value::as_str);
    if text("htm") != Some(method) {
        return Err("DPoP proof htm does not match the request");
    }
    if !same_url(text("htu").ok_or(invalid)?, url) {
        return Err("DPoP proof htu does not match the request");
    }
    let iat = claims.get("iat").and_then(Value::as_i64).ok_or(invalid)?;
    if (now - iat).abs() > IAT_WINDOW_SECS {
        return Err("DPoP proof iat is outside the acceptance window");
    }
    let jti = text("jti")
        .filter(|jti| !jti.is_empty() && jti.len() <= MAX_JTI_LEN)
        .ok_or("DPoP proof needs a jti")?;
    match access_token {
        Some(token) => {
            if text("ath") != Some(b64(&Sha256::digest(token.as_bytes())).as_str()) {
                return Err("DPoP proof ath does not match the access token");
            }
        }
        None if claims.get("ath").is_some() => {
            return Err("DPoP proof at the token endpoint has no ath");
        }
        None => {}
    }
    Ok(Proof {
        jkt,
        jti: jti.to_string(),
    })
}

/// RFC 7638 thumbprint: the key type's required members, in lexicographic
/// order, without whitespace.
pub(super) fn thumbprint(jwk: &Value) -> Option<String> {
    let members: &[&str] = match jwk.get("kty")?.as_str()? {
        "EC" => &["crv", "kty", "x", "y"],
        "RSA" => &["e", "kty", "n"],
        "OKP" => &["crv", "kty", "x"],
        _ => return None,
    };
    let mut canonical = serde_json::Map::new();
    for member in members {
        canonical.insert(
            (*member).to_string(),
            Value::String(jwk.get(*member)?.as_str()?.to_string()),
        );
    }
    // `serde_json::Map` keeps keys sorted unless `preserve_order` is on, and
    // the members above are already in order either way.
    let json = serde_json::to_string(&Value::Object(canonical)).ok()?;
    Some(b64(&Sha256::digest(json.as_bytes())))
}

/// RFC 9449 §4.3: compare without query and fragment, after syntax-based
/// normalisation (scheme and host case, default port).
fn same_url(htu: &str, request: &str) -> bool {
    let (Ok(htu), Ok(request)) = (url::Url::parse(htu), url::Url::parse(request)) else {
        return false;
    };
    htu.scheme() == request.scheme()
        && htu.host_str() == request.host_str()
        && htu.port_or_known_default() == request.port_or_known_default()
        && htu.path() == request.path()
}

#[cfg(test)]
pub(crate) mod test_key {
    //! A DPoP key minted per test run.
    use super::*;
    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
    use serde_json::json;

    pub(crate) struct DpopKey {
        encoding: jsonwebtoken::EncodingKey,
        pub jwk: Value,
    }

    impl DpopKey {
        pub fn generate() -> Self {
            let pkcs8 = EcdsaKeyPair::generate_pkcs8(
                &ECDSA_P256_SHA256_FIXED_SIGNING,
                &SystemRandom::new(),
            )
            .unwrap();
            let pair =
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref()).unwrap();
            let point = pair.public_key().as_ref();
            Self {
                encoding: jsonwebtoken::EncodingKey::from_ec_der(pkcs8.as_ref()),
                jwk: json!({
                    "kty": "EC", "crv": "P-256",
                    "x": b64(&point[1..33]), "y": b64(&point[33..65]),
                }),
            }
        }

        pub fn proof(&self, claims: Value) -> String {
            let mut header = jsonwebtoken::Header::new(Algorithm::ES256);
            header.typ = Some("dpop+jwt".into());
            header.jwk = Some(serde_json::from_value(self.jwk.clone()).unwrap());
            jsonwebtoken::encode(&header, &claims, &self.encoding).unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_key::DpopKey;
    use super::*;
    use serde_json::json;

    const URL: &str = "https://co.example/api/v1/channels/c/poppy/conversations";

    fn claims(now: i64) -> Value {
        json!({ "jti": "j1", "htm": "POST", "htu": URL, "iat": now })
    }

    #[test]
    fn accepts_a_proof_and_reports_its_thumbprint() {
        let key = DpopKey::generate();
        let now = 1_790_000_000;
        let proof = check(&key.proof(claims(now)), "POST", URL, None, now).unwrap();
        assert_eq!(proof.jkt, thumbprint(&key.jwk).unwrap());
        assert_eq!(proof.jti, "j1");
        // Query strings and default ports are not part of the comparison.
        let url = "https://CO.example:443/api/v1/channels/c/poppy/conversations?wait=5";
        assert!(check(&key.proof(claims(now)), "POST", url, None, now).is_ok());
    }

    #[test]
    fn binds_to_method_url_time_and_token() {
        let key = DpopKey::generate();
        let now = 1_790_000_000;
        let proof = key.proof(claims(now));
        assert!(check(&proof, "GET", URL, None, now).is_err());
        assert!(check(&proof, "POST", "https://co.example/other", None, now).is_err());
        assert!(check(&proof, "POST", URL, None, now + 120).is_err());
        assert!(check(&proof, "POST", URL, Some("token"), now).is_err());
        let mut with_ath = claims(now);
        with_ath["ath"] = json!(b64(&Sha256::digest(b"token")));
        let proof = key.proof(with_ath);
        assert!(check(&proof, "POST", URL, Some("token"), now).is_ok());
        assert!(check(&proof, "POST", URL, Some("other"), now).is_err());
        assert!(check(&proof, "POST", URL, None, now).is_err());
    }

    #[test]
    fn rejects_wrong_typ_and_private_keys() {
        let key = DpopKey::generate();
        let now = 1_790_000_000;
        let mut header = jsonwebtoken::Header::new(Algorithm::ES256);
        header.jwk = Some(serde_json::from_value(key.jwk.clone()).unwrap());
        let unsigned = format!(
            "{}.{}.sig",
            b64(serde_json::to_string(&header).unwrap().as_bytes()),
            b64(claims(now).to_string().as_bytes())
        );
        assert!(check(&unsigned, "POST", URL, None, now).is_err());
        let mut leaked = key.jwk.clone();
        leaked["d"] = json!("secret");
        let raw = json!({ "typ": "dpop+jwt", "alg": "ES256", "jwk": leaked });
        let forged = format!(
            "{}.{}.sig",
            b64(raw.to_string().as_bytes()),
            b64(claims(now).to_string().as_bytes())
        );
        assert_eq!(
            check(&forged, "POST", URL, None, now),
            Err("DPoP proof jwk must be a public key")
        );
    }

    #[test]
    fn thumbprint_matches_rfc_7638_example() {
        // RFC 7638 §3.1.
        let jwk = json!({
            "kty": "RSA",
            "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
            "e": "AQAB",
            "alg": "RS256",
            "kid": "2011-04-29"
        });
        assert_eq!(
            thumbprint(&jwk).unwrap(),
            "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs"
        );
    }
}
