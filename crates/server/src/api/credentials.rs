//! Authenticated secure-entry and management adapter. No value is returned.
use super::{
    common::{ApiResult, ErrorResponse},
    state::ApiState,
};
use crate::auth::ResolvedOrg;
use crate::domains::common::{Command, Ctx};
use crate::domains::credentials::commands::*;
use crate::domains::credentials::{CREDENTIAL_MANAGE, CREDENTIAL_VIEW, management as domain};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, put},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/credential-bindings", get(list).post(create))
        .route(
            "/v1/credential-bindings/{id}",
            put(set_value).delete(revoke),
        )
        .route("/v1/resources/{id}/grants", get(grants).post(grant))
        .route(
            "/v1/resources/{id}/grants/{grant_id}",
            axum::routing::delete(revoke_grant),
        )
        .with_state(state)
}
fn context(
    state: &ApiState,
    org: &ResolvedOrg,
    manage: bool,
) -> Result<Ctx, (StatusCode, Json<ErrorResponse>)> {
    let ctx = state.ctx(org);
    (if manage {
        &CREDENTIAL_MANAGE
    } else {
        &CREDENTIAL_VIEW
    })
    .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
    .map_err(|_| {
        ErrorResponse::new("Credential permission required").into_response(StatusCode::FORBIDDEN)
    })?;
    Ok(ctx)
}

#[utoipa::path(get,path="/v1/credential-bindings",params(("scope"=String,Query,description="session, agent, virtual_user, organization"),("owner_id"=Option<String>,Query,description="Owner ID, or me for the current virtual user"),("usage"=String,Query,description="user or service")),responses((status=200,description="Write-only credential metadata",body=Vec<domain::ManagedCredential>)),tag="credentials")]
pub async fn list(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Query(input): Query<domain::CredentialOwnerInput>,
) -> ApiResult<Vec<domain::ManagedCredential>> {
    Ok(Json(
        ListScopedCredentials { owner: input }
            .run(&context(&state, &org, false)?)
            .await?,
    ))
}
#[utoipa::path(post,path="/v1/credential-bindings",request_body=domain::CreateScopedCredential,responses((status=201,description="Binding declared; value is provisioned separately")),tag="credentials")]
pub async fn create(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Json(input): Json<domain::CreateScopedCredential>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<ErrorResponse>)> {
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":input.run(&context(&state,&org,true)?).await?})),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct SetCredentialValue {
    pub value: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub revision: i64,
}
#[utoipa::path(put,path="/v1/credential-bindings/{id}",params(("id"=Uuid,Path,description="Binding ID")),request_body=SetCredentialValue,responses((status=204,description="Credential securely stored; no value is returned")),tag="credentials")]
pub async fn set_value(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    Json(input): Json<SetCredentialValue>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    domain::set_value(
        &context(&state, &org, true)?,
        id,
        input.value,
        input.expires_at,
        input.revision,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize, ToSchema)]
pub struct Revision {
    pub revision: i64,
}
#[utoipa::path(delete,path="/v1/credential-bindings/{id}",params(("id"=Uuid,Path,description="Binding ID"),("revision"=i64,Query,description="Last observed resource revision")),responses((status=204,description="Credential revoked")),tag="credentials")]
pub async fn revoke(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    Query(input): Query<Revision>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    RevokeScopedCredential {
        id,
        revision: input.revision,
    }
    .run(&context(&state, &org, true)?)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/v1/resources/{id}/grants",params(("id"=Uuid,Path,description="Resource ID")),responses((status=200,description="Explicit access grants",body=Vec<Value>)),tag="credentials")]
pub async fn grants(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Vec<Value>> {
    Ok(Json(
        ListResourceGrants { id }
            .run(&context(&state, &org, false)?)
            .await?,
    ))
}
#[utoipa::path(post,path="/v1/resources/{id}/grants",params(("id"=Uuid,Path,description="Resource ID")),request_body=domain::GrantCredential,responses((status=201,description="Access granted")),tag="credentials")]
pub async fn grant(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(id): Path<Uuid>,
    Json(input): Json<domain::GrantCredential>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<ErrorResponse>)> {
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":GrantResource{id,grant:input}.run(&context(&state,&org,true)?).await?})),
    ))
}
#[utoipa::path(delete,path="/v1/resources/{id}/grants/{grant_id}",params(("id"=Uuid,Path,description="Resource ID"),("grant_id"=Uuid,Path,description="Grant ID")),responses((status=204,description="Grant revoked")),tag="credentials")]
pub async fn revoke_grant(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path((id, grant_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    RevokeResourceGrant { id, grant_id }
        .run(&context(&state, &org, true)?)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
