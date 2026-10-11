// Everruns agents as Personal Agent Protocol ("Poppy", draft 0.1) personal
// agents: the client side of the protocol, the other end of the Poppy
// channel. See `knowledge/integrations/poppy-personal-agent.md`.
//
// Design Decisions:
// - One personal agent identity per deployment: one `client_id` (the HTTPS
//   URL of the client metadata document this server publishes at
//   `/v1/poppy/agent.json`) and one ES256 key, published at
//   `/v1/poppy/jwks.json`. A company sees "Everruns" as the personal agent;
//   which person it speaks for is the User ID.
// - User IDs are pairwise: random per (org, session owner, company issuer),
//   so two companies cannot correlate a person (spec 4.2). Each comes with
//   its own DPoP key, encrypted at rest like the signing key.
// - The protocol runs here, in the control plane, through the egress
//   boundary. Workers reach it with the `worker_poppy_call` internal command
//   and never hold a key or a Session Token.
// - Not yet: Sign-In (spec 5), so Sessions are anonymous; APIs and operations
//   (spec 6, 8); event streams. The conversation API covers what a chat needs.

use std::sync::{Arc, OnceLock};

use axum::{
    Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::Value;

use everruns_capabilities::capabilities::PoppyKey;
use everruns_capabilities::capabilities::poppy_client::protocol::{AGENT_PATH, JWKS_PATH};

use crate::storage::pact_delegation::PactSigningKeyRow;
use crate::storage::{EncryptionService, StorageBackend};

pub mod commands;

/// The personal agent's display name at companies.
pub const CLIENT_NAME: &str = "Everruns";

static PUBLIC_BASE_URL: OnceLock<String> = OnceLock::new();

/// Set the deployment's public API base URL (the auth base URL, with the API
/// prefix), from which the `client_id` is derived. First call wins: one
/// process is one deployment.
pub fn configure_public_base_url(base_url: &str) {
    let _ = PUBLIC_BASE_URL.set(base_url.trim_end_matches('/').to_string());
}

/// The deployment's public API base URL.
pub fn public_base_url() -> String {
    PUBLIC_BASE_URL
        .get()
        .cloned()
        .unwrap_or_else(crate::auth::AuthConfig::api_base_url_from_env)
}

/// Pure: the `client_id` for a deployment served at `base_url`.
pub fn client_id(base_url: &str) -> String {
    everruns_capabilities::capabilities::poppy_client::protocol::client_id(base_url)
}

/// Pure: the client metadata document (spec 4.1) for `base_url`.
pub fn client_metadata(base_url: &str) -> Value {
    everruns_capabilities::capabilities::poppy_client::protocol::client_metadata(
        base_url,
        CLIENT_NAME,
    )
}

/// `key` as a stored row, its private key sealed when encryption is on.
pub(crate) fn key_row(
    key: &PoppyKey,
    encryption: Option<&Arc<EncryptionService>>,
) -> anyhow::Result<PactSigningKeyRow> {
    let (private_key, encrypted) = match encryption {
        Some(encryption) => (encryption.encrypt(key.pkcs8())?, true),
        None => (key.pkcs8().to_vec(), false),
    };
    Ok(PactSigningKeyRow {
        kid: key.kid().to_string(),
        private_key,
        encrypted,
        public_jwk: key.public_jwk().clone(),
    })
}

/// The key a stored row holds.
pub(crate) fn key_from_row(
    private_key: &[u8],
    encrypted: bool,
    encryption: Option<&Arc<EncryptionService>>,
) -> anyhow::Result<PoppyKey> {
    let pkcs8 = match (encrypted, encryption) {
        (false, _) => private_key.to_vec(),
        (true, Some(encryption)) => encryption.decrypt(private_key)?,
        (true, None) => anyhow::bail!("Poppy key is encrypted but encryption is off"),
    };
    PoppyKey::from_pkcs8(&pkcs8).map_err(anyhow::Error::msg)
}

/// The deployment's signing key, generated and stored on first use.
pub(crate) async fn signing_key(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
) -> anyhow::Result<PoppyKey> {
    let row = match db.poppy_agent_key().await? {
        Some(row) => row,
        None => {
            let key = PoppyKey::generate().map_err(anyhow::Error::msg)?;
            db.insert_poppy_agent_key(&key_row(&key, encryption)?)
                .await?
        }
    };
    key_from_row(&row.private_key, row.encrypted, encryption)
}

#[derive(Clone)]
pub struct IdentityState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
}

/// The public identity routes, under the API prefix.
pub fn routes(state: IdentityState) -> Router {
    Router::new()
        .route(AGENT_PATH, get(agent_json))
        .route(JWKS_PATH, get(jwks_json))
        .with_state(state)
}

fn public_json(body: &Value) -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=300"),
            ),
        ],
        body.to_string(),
    )
        .into_response()
}

async fn agent_json() -> Response {
    public_json(&client_metadata(&public_base_url()))
}

async fn jwks_json(State(state): State<IdentityState>) -> Response {
    match signing_key(&state.db, state.encryption.as_ref()).await {
        Ok(key) => public_json(&key.jwks()),
        Err(error) => {
            tracing::error!(%error, "Poppy agent key unavailable");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_names_itself_and_keeps_keys_on_its_host() {
        let document = client_metadata("https://app.example/api/");
        assert_eq!(
            document["client_id"],
            "https://app.example/api/v1/poppy/agent.json"
        );
        assert_eq!(
            document["jwks_uri"],
            "https://app.example/api/v1/poppy/jwks.json"
        );
        assert_eq!(document["token_endpoint_auth_method"], "private_key_jwt");
    }
}
