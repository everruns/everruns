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
use serde_json::{Value, json};

use crate::channels::a2a::pact_keys::ProviderKey;
use crate::storage::{EncryptionService, StorageBackend};

pub mod commands;
pub(crate) mod protocol;

/// Where the client metadata document is served, under the API base.
pub const AGENT_PATH: &str = "/v1/poppy/agent.json";
/// Where the signing key is served, under the API base.
pub const JWKS_PATH: &str = "/v1/poppy/jwks.json";
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
    format!("{}{AGENT_PATH}", base_url.trim_end_matches('/'))
}

/// Pure: the client metadata document (spec 4.1) for `base_url`.
pub fn client_metadata(base_url: &str) -> Value {
    let base = base_url.trim_end_matches('/');
    json!({
        "client_id": client_id(base),
        "client_name": CLIENT_NAME,
        "jwks_uri": format!("{base}{JWKS_PATH}"),
        "redirect_uris": [],
        "token_endpoint_auth_method": "private_key_jwt",
    })
}

/// The deployment's signing key, generated and stored on first use.
pub(crate) async fn signing_key(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
) -> anyhow::Result<ProviderKey> {
    let row = match db.poppy_agent_key().await? {
        Some(row) => row,
        None => {
            let row = ProviderKey::generate_row(encryption)?;
            db.insert_poppy_agent_key(&row).await?
        }
    };
    ProviderKey::from_row(&row, encryption)
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
