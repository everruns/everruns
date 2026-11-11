//! Org tokens (cp-dp-protocol.md, "Org tokens"): short EdDSA JWTs that tell
//! a data plane's gateways which org a caller acts for, checked there with
//! this server's JWKS and no call back.
//!
//! Decisions:
//! - One signing key at a time, generated on first use and stored under the
//!   server's secrets key. A retired key stays in the JWKS for a day, longer
//!   than any token it signed lives.
//! - A token lives ten minutes. A caller (the console, the CLI, an API key
//!   at the edge) exchanges its own credential for one at
//!   `POST /v1/auth/token` and refreshes it before it ends, so a revoked
//!   user or key stops within ten minutes.
//! - The scope is `sessions:write`: the org's members drive the org's
//!   sessions. A data plane's `admin` scope is for operators and is never
//!   minted here.

use std::sync::Arc;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use base64::Engine as _;
use everruns_contracts::dataplane::OrgTokenClaims;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Value, json};

use super::ControlPlane;
use crate::storage::{DpSigningKeyRow, EncryptionService};

/// How long an org token lives.
pub const TOKEN_TTL_SECS: i64 = 600;
/// The scope members get.
pub const SESSIONS_SCOPE: &str = "sessions:write";
/// How long a retired key stays published.
const RETIRED_KEPT: chrono::Duration = chrono::Duration::hours(24);

fn generate(encryption: Option<&Arc<EncryptionService>>) -> anyhow::Result<DpSigningKeyRow> {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
        .map_err(|_| anyhow::anyhow!("could not generate an Ed25519 key"))?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .map_err(|_| anyhow::anyhow!("could not read the generated Ed25519 key"))?;
    let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(pair.public_key().as_ref());
    let kid = format!("dpk_{}", uuid::Uuid::new_v4().simple());
    let (private_key, encrypted) = match encryption {
        Some(encryption) => (encryption.encrypt(pkcs8.as_ref())?, true),
        None => (pkcs8.as_ref().to_vec(), false),
    };
    Ok(DpSigningKeyRow {
        public_jwk: json!({
            "kty": "OKP", "crv": "Ed25519", "alg": "EdDSA", "use": "sig", "kid": kid, "x": x,
        }),
        kid,
        private_key,
        encrypted,
        retired_at: None,
    })
}

/// The current signing key, made on first use.
async fn current(cp: &ControlPlane) -> anyhow::Result<DpSigningKeyRow> {
    if let Some(row) = cp
        .db
        .dp_signing_keys()
        .await?
        .into_iter()
        .find(|k| k.retired_at.is_none())
    {
        return Ok(row);
    }
    cp.db
        .dp_insert_signing_key(&generate(cp.encryption.as_ref())?)
        .await?;
    cp.db
        .dp_signing_keys()
        .await?
        .into_iter()
        .find(|k| k.retired_at.is_none())
        .ok_or_else(|| anyhow::anyhow!("no org token signing key"))
}

/// `GET /.well-known/jwks.json`.
pub async fn jwks(cp: &ControlPlane) -> anyhow::Result<Value> {
    current(cp).await?;
    let now = chrono::Utc::now();
    let keys: Vec<Value> = cp
        .db
        .dp_signing_keys()
        .await?
        .into_iter()
        .filter(|k| k.retired_at.is_none_or(|at| now - at < RETIRED_KEPT))
        .map(|k| k.public_jwk)
        .collect();
    Ok(json!({ "keys": keys }))
}

/// An org token for `sub` acting in `org_id`, on the org's data plane.
pub async fn issue(cp: &ControlPlane, org_id: i64, sub: &str) -> anyhow::Result<(String, i64)> {
    let org = cp
        .db
        .dp_org(org_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the org is not on a data plane"))?;
    let key = current(cp).await?;
    let pkcs8 = match (key.encrypted, &cp.encryption) {
        (false, _) => key.private_key.clone(),
        (true, Some(encryption)) => encryption.decrypt(&key.private_key)?,
        (true, None) => anyhow::bail!("the signing key is encrypted but encryption is off"),
    };
    let now = chrono::Utc::now().timestamp();
    let claims = OrgTokenClaims {
        sub: sub.to_owned(),
        org: super::org_public(org_id),
        dp: org.data_plane_id,
        scope: SESSIONS_SCOPE.to_owned(),
        iat: now,
        exp: now + TOKEN_TTL_SECS,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(key.kid);
    let token = jsonwebtoken::encode(&header, &claims, &EncodingKey::from_ed_der(&pkcs8))?;
    Ok((token, claims.exp))
}
