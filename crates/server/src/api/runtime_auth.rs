//! Exchanges verified endpoint identity for a bounded consumer self credential.
use super::common::impl_auth_state;
use crate::auth::AuthState;
use crate::domains::agent_channels::api_sessions::{ApiAuthError, api_channel_config};
use crate::domains::agent_channels::record::ChannelType;
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
    pub verifier: super::channel_auth::ChannelAuthVerifier,
}
impl_auth_state!(AppState);
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/channels/{channel_id}/runtime-auth", post(exchange))
        .route("/v1/e/{channel_id}/runtime-auth", post(exchange))
        .with_state(state)
}
#[utoipa::path(summary = "Exchange verified channel authentication for a bounded runtime credential.", post, path = "/v1/channels/{channel_id}/runtime-auth", params(("channel_id" = String, Path)),  responses((status = 200, description = "Success", body = serde_json::Value), (status = 400, description = "An agent key without an End-User id"), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
async fn exchange(
    State(state): State<AppState>,
    Path(endpoint): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let (context, channel) =
        super::channel_ingress::resolve_channel(&state.db, state.encryption.as_ref(), &endpoint)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
    if super::channel_ingress::channel_liveness(&context, &channel).is_err() {
        return Err(StatusCode::NOT_FOUND);
    }
    let principal = if channel.channel_type == ChannelType::Api {
        api_channel_principal(&state, &context, &channel, &headers).await?
    } else {
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
                super::channel_auth::LegacyChannelAuth {
                    shared_secret: None,
                    api_key: None,
                },
            )
            .await
            .map_err(|_| StatusCode::UNAUTHORIZED)?
            .ok_or(StatusCode::UNAUTHORIZED)?;
        Principal {
            provider: principal.provider,
            realm: principal.identity_realm,
            subject: principal.subject,
        }
    };
    // An api channel's tokens name the channel's own id, which is what its
    // routes compare against, whichever alias the URL used.
    let endpoint = if channel.channel_type == ChannelType::Api {
        channel.public_id.to_string()
    } else {
        endpoint
    };
    let user = state
        .db
        .resolve_runtime_identity(crate::storage::runtime_identity::VerifiedRuntimeIdentity {
            org_id: context.org_id,
            provider: principal.provider.clone(),
            realm: principal.realm.clone(),
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
            b.provider == principal.provider
                && b.realm == principal.realm
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

/// The identity a runtime token is minted for.
struct Principal {
    provider: String,
    realm: String,
    subject: String,
}

/// On an `api` channel the exchange takes what the channel's routes take,
/// minus runtime tokens (a token never renews itself): a key with `end_user`
/// and an `End-User` id, or a token of one of the channel's identity
/// providers. A key acting as itself is an application, not an end user.
async fn api_channel_principal(
    state: &AppState,
    context: &super::channel_ingress::IngressContext,
    channel: &super::channel_ingress::IngressChannel,
    headers: &HeaderMap,
) -> Result<Principal, StatusCode> {
    let config = api_channel_config(channel).map_err(|_| StatusCode::FORBIDDEN)?;
    let checks = super::agent_api_auth::CallerChecks {
        db: &state.db,
        verifier: &state.verifier,
        runtime_auth: None,
    };
    match super::agent_api_auth::verified_identity(&checks, context, channel, &config, headers)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Ok(Some(identity)) => Ok(Principal {
            provider: identity.provider,
            realm: identity.realm,
            subject: identity.subject,
        }),
        Ok(None) => Err(StatusCode::BAD_REQUEST),
        Err(ApiAuthError::Forbidden) => Err(StatusCode::FORBIDDEN),
        Err(ApiAuthError::Misconfigured) => Err(StatusCode::FORBIDDEN),
        Err(ApiAuthError::Unauthorized) => Err(StatusCode::UNAUTHORIZED),
    }
}
