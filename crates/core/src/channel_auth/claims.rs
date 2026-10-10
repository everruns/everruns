//! Credential extraction, JWT verification against a key set, and claim
//! requirements. Pure functions: nothing here touches the network.

use std::collections::HashSet;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    AGENTID_ISSUER, AGENTID_PROVIDER, ChannelAuthError, ChannelAuthPrincipal,
    ChannelAuthRequirements, OIDC_PROVIDER,
};

/// Compare two byte slices in constant time with respect to their contents.
///
/// Returns `false` at once when the lengths differ (length is not treated as
/// secret) and otherwise inspects every byte, so the running time does not
/// depend on where the first mismatch is.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// The token of an `Authorization: Bearer <token>` header.
pub fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let auth = headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = auth.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token)
}

pub(super) fn extract_basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let auth = headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, encoded) = auth.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = BASE64_STANDARD.decode(encoded).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (username, password) = decoded.split_once(':')?;
    Some((username.to_string(), password.to_string()))
}

pub(super) fn verify_shared_secret(
    headers: &HeaderMap,
    expected: &str,
) -> Result<(), ChannelAuthError> {
    let provided = extract_bearer(headers).ok_or(ChannelAuthError::Unauthorized)?;
    if constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(ChannelAuthError::Unauthorized)
    }
}

pub(super) fn normalize_issuer(issuer: &str) -> String {
    issuer.trim().trim_end_matches('/').to_string()
}

pub(super) fn principal_from_claims(
    claims: &Value,
    verifier_authority: &str,
) -> Result<ChannelAuthPrincipal, ChannelAuthError> {
    let issuer = claims
        .get("iss")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ChannelAuthError::Unauthorized)?;
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ChannelAuthError::Unauthorized)?;
    let issuer = normalize_issuer(issuer);
    // THREAT[TM-AUTH-031]: Endpoint owners control their verifier configuration.
    // Scope runtime identities to the verifier that proved the claims so a
    // different JWKS or introspection service cannot assert another authority's
    // issuer/subject pair and inherit its private grants.
    let authority_hash = Sha256::digest(verifier_authority.as_bytes());
    let authority_hex: String = authority_hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(ChannelAuthPrincipal {
        provider: OIDC_PROVIDER.to_string(),
        identity_realm: format!("{issuer}#{authority_hex}"),
        issuer,
        subject: subject.to_string(),
    })
}

/// Verify a bearer JWT against an already fetched key set.
///
/// `agentid` is true only when the issuer is AgentID and the keys came from
/// AgentID discovery (see `ChannelAuthConfig::is_agentid`). Such tokens must be
/// ES256 and carry `actor_type = "agent"`, and their subjects bind under the
/// `agentid` provider with the raw issuer as realm, so the same agent inbox is
/// one virtual user across every AgentID channel of an org.
pub(super) fn verify_jwt_with_jwks(
    token: &str,
    issuer: &str,
    jwks: &JwkSet,
    requirements: &ChannelAuthRequirements,
    verifier_authority: &str,
    agentid: bool,
) -> Result<ChannelAuthPrincipal, ChannelAuthError> {
    if agentid {
        let claims = verify_agentid_claims(token, jwks, requirements)?;
        let subject = claims
            .get("sub")
            .and_then(Value::as_str)
            .ok_or(ChannelAuthError::Unauthorized)?;
        return Ok(ChannelAuthPrincipal {
            provider: AGENTID_PROVIDER.to_string(),
            issuer: AGENTID_ISSUER.to_string(),
            subject: subject.to_string(),
            identity_realm: AGENTID_ISSUER.to_string(),
        });
    }
    let claims = decode_verified_claims(token, issuer, jwks, requirements, false)?;
    principal_from_claims(&claims, verifier_authority)
}

/// Verify an AgentID token (ES256, `iss`, `aud`, `exp`, the claim
/// requirements) and return its claims. `actor_type` and `sub` are checked
/// only after the signature: an unverified claim decides nothing.
pub fn verify_agentid_claims(
    token: &str,
    jwks: &JwkSet,
    requirements: &ChannelAuthRequirements,
) -> Result<Value, ChannelAuthError> {
    let claims = decode_verified_claims(token, AGENTID_ISSUER, jwks, requirements, true)?;
    if claims.get("actor_type").and_then(Value::as_str) != Some("agent") {
        return Err(ChannelAuthError::Unauthorized);
    }
    if claims
        .get("sub")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(ChannelAuthError::Unauthorized);
    }
    Ok(claims)
}

fn decode_verified_claims(
    token: &str,
    issuer: &str,
    jwks: &JwkSet,
    requirements: &ChannelAuthRequirements,
    es256_only: bool,
) -> Result<Value, ChannelAuthError> {
    let header = decode_header(token).map_err(|_| ChannelAuthError::Unauthorized)?;
    let kid = header.kid.ok_or(ChannelAuthError::Unauthorized)?;
    let jwk = jwks
        .keys
        .iter()
        .find(|jwk| jwk.common.key_id.as_deref() == Some(kid.as_str()))
        .ok_or(ChannelAuthError::Unauthorized)?;
    let alg = header.alg;
    if !is_public_key_algorithm(alg) || (es256_only && alg != Algorithm::ES256) {
        return Err(ChannelAuthError::Unauthorized);
    }
    let key = DecodingKey::from_jwk(jwk).map_err(|_| ChannelAuthError::Unauthorized)?;
    let mut validation = Validation::new(alg);
    validation.set_issuer(&[issuer]);
    validation.set_audience(&requirements.audiences);
    validation.validate_nbf = true;
    validation.set_required_spec_claims(&["exp", "iss", "aud"]);
    let claims = decode::<Value>(token, &key, &validation)
        .map_err(|_| ChannelAuthError::Unauthorized)?
        .claims;
    validate_claim_requirements(&claims, requirements)?;
    Ok(claims)
}

fn is_public_key_algorithm(alg: Algorithm) -> bool {
    matches!(
        alg,
        Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::EdDSA
    )
}

pub(super) fn validate_claim_requirements(
    claims: &Value,
    requirements: &ChannelAuthRequirements,
) -> Result<(), ChannelAuthError> {
    if !requirements.audiences.is_empty() {
        let audiences = claim_audience_set(claims);
        if !requirements
            .audiences
            .iter()
            .any(|audience| audiences.contains(audience))
        {
            return Err(ChannelAuthError::Unauthorized);
        }
    }
    if !requirements.scopes.is_empty() {
        let token_scopes = claim_string_set(claims, "scope")
            .into_iter()
            .chain(claim_array_strings(claims, "scp"))
            .collect::<HashSet<_>>();
        if !requirements
            .scopes
            .iter()
            .all(|scope| token_scopes.contains(scope))
        {
            return Err(ChannelAuthError::Unauthorized);
        }
    }
    if !requirements.subjects.is_empty() {
        let subject = claims
            .get("sub")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !requirements
            .subjects
            .iter()
            .any(|allowed| allowed == subject)
        {
            return Err(ChannelAuthError::Unauthorized);
        }
    }
    if !requirements.domains.is_empty() {
        let domain = claims
            .get("hd")
            .and_then(Value::as_str)
            .or_else(|| email_domain(claims.get("email").and_then(Value::as_str)))
            .unwrap_or_default();
        if !requirements.domains.iter().any(|allowed| allowed == domain) {
            return Err(ChannelAuthError::Unauthorized);
        }
    }
    if !requirements.groups.is_empty() {
        let groups = claim_array_strings(claims, "groups");
        if !requirements
            .groups
            .iter()
            .any(|group| groups.contains(group))
        {
            return Err(ChannelAuthError::Unauthorized);
        }
    }
    for (name, expected) in &requirements.claims {
        if claims.get(name) != Some(expected) {
            return Err(ChannelAuthError::Unauthorized);
        }
    }
    Ok(())
}

fn claim_string_set(claims: &Value, name: &str) -> HashSet<String> {
    claims
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .filter(|scope| !scope.is_empty())
        .map(str::to_string)
        .collect()
}

fn claim_array_strings(claims: &Value, name: &str) -> HashSet<String> {
    claims
        .get(name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn claim_audience_set(claims: &Value) -> HashSet<String> {
    let mut audiences = claim_array_strings(claims, "aud");
    if let Some(audience) = claims.get("aud").and_then(Value::as_str) {
        audiences.insert(audience.to_string());
    }
    audiences
}

fn email_domain(email: Option<&str>) -> Option<&str> {
    email?.split_once('@').map(|(_, domain)| domain)
}
