// HTTP Message Signatures Directory endpoint
//
// Decision: /.well-known/http-message-signatures-directory is public (no auth).
//   Target servers need to discover bot public keys without credentials.
// Decision: Serves all non-expired keys as a JWKS (RFC 7517 Section 5).
// Decision: Content-Type is application/json (widely supported; the draft
//   media type application/http-message-signatures-directory+json is not
//   yet registered).
// See: draft-meunier-http-message-signatures-directory-05

use crate::storage::StorageBackend;
use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
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
    let keys = state
        .db
        .list_all_http_signing_keys()
        .await
        .unwrap_or_default();

    let jwks: Vec<serde_json::Value> = keys
        .into_iter()
        .map(|k| {
            let mut jwk = k.public_key;
            // Ensure kid is set from the key_id (JWK thumbprint)
            if let Some(obj) = jwk.as_object_mut() {
                obj.entry("kid").or_insert_with(|| k.key_id.into());
            }
            jwk
        })
        .collect();

    Json(JwksResponse { keys: jwks })
}
