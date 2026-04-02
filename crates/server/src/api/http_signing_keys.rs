// HTTP Message Signatures Directory endpoint
//
// Decision: /.well-known/http-message-signatures-directory is public (no auth).
//   Target servers need to discover bot public keys without credentials.
// Decision: Public key derived at startup from BOT_AUTH_SIGNING_KEY_SEED env var.
//   No DB needed — server-wide config, single key.
// Decision: Content-Type is application/json (widely supported; the draft
//   media type is not yet IANA-registered).
// See: draft-meunier-http-message-signatures-directory-05

use axum::{Json, Router, extract::State, routing::get};
use everruns_core::capabilities::{BotAuthPublicKey, derive_bot_auth_public_key};
use serde::Serialize;

#[derive(Clone)]
pub struct AppState {
    /// Pre-computed JWKS from BOT_AUTH_SIGNING_KEY_SEED. Empty when bot-auth is disabled.
    pub keys: Vec<BotAuthPublicKey>,
}

impl AppState {
    /// Build from environment. Returns state with empty keys if bot-auth is not configured.
    pub fn from_env() -> Self {
        let keys = std::env::var("BOT_AUTH_SIGNING_KEY_SEED")
            .ok()
            .and_then(|seed| derive_bot_auth_public_key(&seed))
            .into_iter()
            .collect();
        Self { keys }
    }
}

pub fn routes(state: AppState) -> Router {
    Router::new().route(
        "/.well-known/http-message-signatures-directory",
        get(signatures_directory).with_state(state),
    )
}

/// JWKS response — JSON Web Key Set containing Ed25519 public keys.
#[derive(Serialize)]
struct JwksResponse {
    keys: Vec<serde_json::Value>,
}

async fn signatures_directory(State(state): State<AppState>) -> Json<JwksResponse> {
    let jwks: Vec<serde_json::Value> = state
        .keys
        .iter()
        .map(|k| {
            let mut jwk = k.jwk.clone();
            if let Some(obj) = jwk.as_object_mut() {
                obj.entry("kid").or_insert_with(|| k.key_id.clone().into());
            }
            jwk
        })
        .collect();

    Json(JwksResponse { keys: jwks })
}
