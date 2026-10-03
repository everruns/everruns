//! Exchanges verified endpoint identity for a bounded consumer self credential.
use super::common::impl_auth_state;
use crate::auth::AuthState;
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use std::sync::Arc;
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
    pub encryption: Option<Arc<EncryptionService>>,
    pub verifier: super::endpoint_auth::EndpointAuthVerifier,
}
impl_auth_state!(AppState);
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/e/{endpoint_id}/runtime-auth", post(exchange))
        .with_state(state)
}
#[utoipa::path(summary = "Exchange verified endpoint authentication for a bounded runtime credential.", post, path = "/v1/e/{endpoint_id}/runtime-auth", params(("endpoint_id" = String, Path)),  responses((status = 200, description = "Success", body = serde_json::Value), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
async fn exchange(
    State(state): State<AppState>,
    Path(endpoint): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let (context, channel) =
        super::endpoint_ingress::resolve_endpoint(&state.db, state.encryption.as_ref(), &endpoint)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
    if super::endpoint_ingress::endpoint_liveness(&context, &channel).is_err() {
        return Err(StatusCode::NOT_FOUND);
    }
    let public_chat_auth = channel.public_chat_config().and_then(|c| c.auth);
    let auth = channel
        .auth
        .as_deref()
        .or(public_chat_auth.as_ref())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let principal = state
        .verifier
        .verify_principal(
            auth,
            &headers,
            super::endpoint_auth::LegacyEndpointAuth {
                shared_secret: None,
                api_key: None,
            },
        )
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let user = state
        .db
        .resolve_runtime_identity(crate::storage::runtime_identity::VerifiedRuntimeIdentity {
            org_id: context.org_id,
            provider: "oidc".into(),
            realm: principal.identity_realm.clone(),
            subject: principal.subject.clone(),
            name: "User".into(),
            avatar_url: None,
            management_user_id: None,
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if user.status != "active" {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let bindings = state
        .db
        .list_virtual_user_bindings(context.org_id, user.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let binding = bindings
        .iter()
        .find(|b| {
            b.provider == "oidc"
                && b.realm == principal.identity_realm
                && b.subject == principal.subject
        })
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let token = crate::auth::jwt::JwtService::new(state.auth.config.jwt.clone())
        .generate_runtime_token(context.org_id, user.id, endpoint, binding.id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        serde_json::json!({"access_token":token,"token_type":"Bearer","expires_in":900,"virtual_user_id":user.id}),
    ))
}
