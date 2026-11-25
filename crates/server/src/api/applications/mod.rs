//! Management commands and the separate Application consumer surface.
use crate::api::{
    command_http::CommandRouterExt, common::impl_auth_state, dispatch::impl_dispatchable,
};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::applications::*;
use crate::domains::common::{CommandError, Ctx};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use everruns_contracts::typed_id::ApplicationId;
use serde_json::{Value, json};
pub(crate) mod resources;
#[derive(Clone)]
pub struct AppState {
    pub rooms: super::rooms::AppState,
    pub auth: AuthState,
    pub verifier: super::channel_auth::ChannelAuthVerifier,
    pub oidc: Option<crate::domains::agent_channels::record::ChannelAuthConfig>,
}
impl AppState {
    pub fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        self.rooms.ctx(org)
    }
    pub fn consumer_ctx(&self, org: i64) -> Ctx {
        Ctx::new(
            everruns_core::Caller::internal(org),
            self.rooms.db.clone(),
            self.rooms.capability_service.clone(),
            self.rooms.encryption.clone(),
            self.auth.permission_resolver.clone(),
        )
        .with_session_service(self.rooms.session_service.clone())
        .with_message_service(self.rooms.message_service.clone())
        .with_relevance(self.rooms.relevance.clone())
    }
}
impl_auth_state!(AppState);
impl_dispatchable!(AppState);
pub fn routes(state: AppState) -> Router {
    Router::new()
        .command::<ListApplications>()
        .command::<GetApplication>()
        .command::<CreateApplication>()
        .command::<UpdateApplication>()
        .command::<SetApplicationStatus>()
        .command::<CreateApplicationResource>()
        .route(
            "/v1/platform/application-launchers",
            get(application_launchers),
        )
        .route("/v1/application-hosts/{slug}", get(application_host))
        .route(
            "/v1/applications/{application_id}/config",
            get(application_config),
        )
        .route(
            "/v1/applications/{application_id}/runtime-auth",
            post(application_exchange),
        )
        .route(
            "/v1/applications/{application_id}/manifest",
            get(application_manifest),
        )
        .route(
            "/v1/applications/{application_id}/rooms/{slug}",
            get(resources::application_room),
        )
        .route(
            "/v1/applications/{application_id}/rooms/{slug}/messages",
            get(resources::application_messages).post(resources::application_post_message),
        )
        .route(
            "/v1/applications/{application_id}/rooms/{slug}/replies/{message_id}",
            get(resources::application_replies),
        )
        .route(
            "/v1/applications/{application_id}/memory/{slug}",
            get(resources::application_memory)
                .put(resources::application_write_memory)
                .delete(resources::application_delete_memory),
        )
        .with_state(state)
}
#[utoipa::path(get, operation_id="application_host",path="/v1/application-hosts/{slug}",description="Resolve a published Application hostname.",params(("slug" = String, Path)),responses((status=200,description="Success",body=serde_json::Value),(status=401,description="Sign-in required"),(status=403,description="Access denied"),(status=404,description="Unavailable")),tag="applications")]
async fn application_host(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let row = state
        .rooms
        .db
        .application_by_slug(&slug)
        .await
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(
        json!({"application_id":ApplicationId::from_uuid(row.id)}),
    ))
}
#[utoipa::path(get, operation_id="application_config",path="/v1/applications/{application_id}/config",description="Read public Application presentation.",params(("application_id" = String, Path)),responses((status=200,description="Success",body=serde_json::Value),(status=401,description="Sign-in required"),(status=403,description="Access denied"),(status=404,description="Unavailable")),tag="applications")]
async fn application_config(
    State(state): State<AppState>,
    Path(id): Path<ApplicationId>,
) -> Result<Json<Value>, StatusCode> {
    // Lookup by ID without revealing draft configuration or audience addresses.
    let org = state
        .rooms
        .db
        .get_application_organization_id(&id.to_string())
        .await
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let row = state
        .rooms
        .db
        .application(org, id)
        .await
        .map_err(internal)?
        .filter(|r| r.status == "published")
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(
        json!({"name":row.name,"description":row.description,"kind":"application"}),
    ))
}
#[utoipa::path(post, operation_id="application_exchange",path="/v1/applications/{application_id}/runtime-auth",description="Exchange a verified deployment identity for Application access.",params(("application_id" = String, Path)),responses((status=200,description="Success",body=serde_json::Value),(status=401,description="Sign-in required"),(status=403,description="Access denied"),(status=404,description="Unavailable")),tag="applications")]
async fn application_exchange(
    State(state): State<AppState>,
    Path(id): Path<ApplicationId>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let policy = state.oidc.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let identity = state
        .verifier
        .verify_principal(
            policy,
            &headers,
            super::channel_auth::LegacyChannelAuth::default(),
        )
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let db = &state.rooms.db;
    let org = db
        .get_application_organization_id(&id.to_string())
        .await
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let row = db
        .application(org, id)
        .await
        .map_err(internal)?
        .filter(|r| r.status == "published")
        .ok_or(StatusCode::NOT_FOUND)?;
    let config: ApplicationConfig = serde_json::from_value(row.config).map_err(internal)?;
    if !crate::domains::agent_channels::work_chat::identity_admitted(
        db,
        org,
        &config.audience(),
        &identity.subject,
        identity.email.as_deref(),
    )
    .await
    .map_err(internal)?
    {
        return Err(StatusCode::FORBIDDEN);
    }
    // THREAT[TM-TENANT-025]: External identities cannot inherit another audience's private account.
    // External audiences have a private realm per Application; organization
    // mode deliberately reuses the member's existing runtime account.
    let (user, binding) = if config.membership
        == crate::domains::agent_channels::record::work_chat::WorkChatMembership::OrgUsers
    {
        let mut member = db
            .get_user_by_external_id(&identity.subject)
            .await
            .map_err(internal)?;
        if !match &member {
            Some(u) => db
                .is_organization_member(org, u.id)
                .await
                .map_err(internal)?,
            None => false,
        } {
            member = None;
        }
        if member.is_none()
            && let Some(email) = &identity.email
        {
            member = db
                .get_user_by_email(email)
                .await
                .map_err(internal)?
                .filter(|u| u.email_verified);
        }
        let member = member.ok_or(StatusCode::FORBIDDEN)?;
        let user = db
            .default_virtual_user(org, member.id)
            .await
            .map_err(|_| StatusCode::FORBIDDEN)?;
        let binding = db
            .list_virtual_user_bindings(org, user.id)
            .await
            .map_err(internal)?
            .into_iter()
            .find(|b| b.management_user_id == Some(member.id))
            .ok_or(StatusCode::UNAUTHORIZED)?;
        (user, binding)
    } else {
        let realm = format!("{}:application:{}", identity.identity_realm, id);
        let user = db
            .resolve_runtime_identity(crate::storage::runtime_identity::VerifiedRuntimeIdentity {
                org_id: org,
                provider: identity.provider.clone(),
                realm: realm.clone(),
                subject: identity.subject.clone(),
                name: "User".into(),
                avatar_url: None,
                management_user_id: None,
            })
            .await
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        let binding = db
            .list_virtual_user_bindings(org, user.id)
            .await
            .map_err(internal)?
            .into_iter()
            .find(|b| {
                b.provider == identity.provider && b.realm == realm && b.subject == identity.subject
            })
            .ok_or(StatusCode::UNAUTHORIZED)?;
        (user, binding)
    };
    if user.status != "active" {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let token = crate::auth::jwt::JwtService::new(state.auth.config.jwt.clone())
        .generate_application_token(
            org,
            user.id,
            binding.id,
            crate::auth::jwt::ApplicationIdentityClaims {
                id,
                version: row.version,
                subject: identity.subject,
                email: identity.email,
            },
        )
        .map_err(internal)?;
    Ok(Json(
        json!({"access_token":token,"token_type":"Bearer","expires_in":900,"virtual_user_id":user.id}),
    ))
}
async fn account(
    state: &AppState,
    id: ApplicationId,
    headers: &HeaderMap,
) -> Result<crate::auth::runtime::RuntimeAccount, StatusCode> {
    let bearer = super::channel_auth::extract_bearer(headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let account = crate::auth::runtime::RuntimeAccount::from_token(&state.auth, bearer)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    if account.application_id != Some(id) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(account)
}
#[utoipa::path(get, operation_id="application_manifest",path="/v1/applications/{application_id}/manifest",description="List resources available to the signed-in Application audience.",params(("application_id" = String, Path)),responses((status=200,description="Success",body=serde_json::Value),(status=401,description="Sign-in required"),(status=403,description="Access denied"),(status=404,description="Unavailable")),tag="applications")]
async fn application_manifest(
    State(state): State<AppState>,
    Path(id): Path<ApplicationId>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let account = account(&state, id, &headers).await?;
    let row = state
        .rooms
        .db
        .application(account.org_id, id)
        .await
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let resources = state
        .rooms
        .db
        .application_resources(id.uuid())
        .await
        .map_err(internal)?;
    Ok(Json(
        json!({"name":row.name,"description":row.description,"resources":resources,"virtual_user_id":account.id}),
    ))
}
fn internal(error: impl std::fmt::Display) -> StatusCode {
    tracing::error!(%error,"Application operation failed");
    StatusCode::INTERNAL_SERVER_ERROR
}
fn command_error(error: CommandError) -> StatusCode {
    let response: (StatusCode, Json<crate::common_dto::ErrorResponse>) = error.into();
    response.0
}

#[utoipa::path(get, operation_id="application_launchers",path="/v1/platform/application-launchers",description="List published Application sidebar launchers.",responses((status=200,description="Success",body=serde_json::Value),(status=401,description="Sign-in required"),(status=403,description="Access denied"),(status=404,description="Unavailable")),tag="applications")]
async fn application_launchers(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let applications = state
        .rooms
        .db
        .list_applications(org.org_id)
        .await
        .map_err(internal)?;
    let links: Vec<_> = applications
        .into_iter()
        .filter(|a| a.status == "published" && a.config.show_in_sidebar)
        .filter_map(|a| {
            a.public_url
                .map(|url| json!({"id":a.id,"name":a.name,"url":url}))
        })
        .collect();
    Ok(Json(json!(links)))
}

pub fn deployment_oidc() -> Option<crate::domains::agent_channels::record::ChannelAuthConfig> {
    use crate::domains::agent_channels::record::{
        ChannelAuthConfig, ChannelAuthMode, ChannelAuthProviderConfig, ChannelAuthRequirements,
    };
    let issuer = std::env::var("CHAT_OIDC_ISSUER")
        .ok()
        .filter(|s| s.starts_with("https://"))?;
    let client = std::env::var("CHAT_OIDC_CLIENT_ID")
        .ok()
        .filter(|s| !s.is_empty())?;
    Some(ChannelAuthConfig {
        mode: ChannelAuthMode::Oidc,
        provider: Some(ChannelAuthProviderConfig::Oidc {
            issuer,
            jwks_url: None,
        }),
        requirements: ChannelAuthRequirements {
            audiences: vec![client],
            ..Default::default()
        },
    })
}
