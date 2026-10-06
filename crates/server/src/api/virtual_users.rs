// Org-scoped runtime account management and self-service.
// Routes use ResolvedOrg: org derived from auth context (API key or cookie).

use crate::api::state::ApiState;
use crate::auth::runtime::RuntimeAccount;
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::virtual_users::types::{
    CreateVirtualUserRequest, ListVirtualUsersQuery, UpdateVirtualUserRequest,
};
use crate::domains::virtual_users::{
    VIRTUAL_USER_DANGEROUS, VIRTUAL_USER_MANAGE, VIRTUAL_USER_VIEW,
};
use crate::records::VirtualUser;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::{Caller, ResourceConfigResponse, evaluate_policies_with};

use super::common::{ErrorResponse, PaginatedResponse};
use super::dispatch::Dispatchable;
use crate::domains::common::Command;

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/virtual-users/config", get(virtual_user_config))
        .route("/v1/virtual-users/me", get(get_me).patch(update_me))
        .route(
            "/v1/virtual-users/{identity_id}/sessions",
            get(list_sessions),
        )
        .route(
            "/v1/virtual-users/{identity_id}/bindings",
            get(list_bindings),
        )
        .route(
            "/v1/virtual-users/{identity_id}/bindings/{binding_id}",
            axum::routing::delete(revoke_binding),
        )
        .route(
            "/v1/virtual-users/{identity_id}/preferences",
            get(list_preferences),
        )
        .route(
            "/v1/virtual-users/{identity_id}/preferences/{key}",
            get(get_preference)
                .put(set_preference)
                .delete(delete_preference),
        )
        .route(
            "/v1/virtual-users",
            post(create_virtual_user).get(list_virtual_users),
        )
        .route(
            "/v1/virtual-users/{identity_id}",
            get(get_virtual_user)
                .patch(update_virtual_user)
                .delete(delete_virtual_user),
        )
        .route(
            "/v1/virtual-users/{identity_id}/delete",
            post(destroy_virtual_user),
        )
        .with_state(state)
}

pub async fn virtual_user_config(
    State(auth): State<AuthState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[
            &VIRTUAL_USER_VIEW,
            &VIRTUAL_USER_MANAGE,
            &VIRTUAL_USER_DANGEROUS,
        ],
    );
    Json(ResourceConfigResponse { policies })
}

#[utoipa::path(summary = "Create virtual user.", post, path = "/v1/virtual-users",  request_body = CreateVirtualUserRequest, responses((status = 201, description = "Success", body = VirtualUser), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
pub async fn create_virtual_user(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Json(req): Json<CreateVirtualUserRequest>,
) -> Result<(StatusCode, Json<VirtualUser>), (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_created(crate::domains::virtual_users::CreateVirtualUser(req))
        .await
}

#[utoipa::path(summary = "List virtual users.", get, path = "/v1/virtual-users", params(ListVirtualUsersQuery), responses((status = 200, description = "Success", body = PaginatedResponse<VirtualUser>), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
pub async fn list_virtual_users(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Query(query): Query<ListVirtualUsersQuery>,
) -> Result<Json<PaginatedResponse<VirtualUser>>, (StatusCode, Json<ErrorResponse>)> {
    let result = crate::domains::virtual_users::ListVirtualUsers {
        usage: query.usage,
        search: query.search,
        include_archived: query.include_archived.unwrap_or(false),
        offset: query.offset,
        limit: query.limit,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(PaginatedResponse::new(
        result.data,
        result.total,
        result.offset,
        result.limit,
    )))
}

#[utoipa::path(summary = "Get virtual user.", get, path = "/v1/virtual-users/{identity_id}", params(("identity_id" = String, Path)),  responses((status = 200, description = "Success", body = VirtualUser), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
pub async fn get_virtual_user(
    account: RuntimeAccount,
    State(state): State<ApiState>,
    Path(identity_id): Path<String>,
) -> Result<Json<VirtualUser>, (StatusCode, Json<ErrorResponse>)> {
    if account.permits_self(&identity_id) {
        return get_me(account, State(state)).await;
    }
    let org = account.management.as_ref().ok_or_else(|| {
        ErrorResponse::new("Self-service only").into_response(StatusCode::FORBIDDEN)
    })?;
    state
        .dispatcher(org)
        .run(crate::domains::virtual_users::GetVirtualUser { id: identity_id })
        .await
}

#[utoipa::path(summary = "Update virtual user.", patch, path = "/v1/virtual-users/{identity_id}", params(("identity_id" = String, Path)), request_body = UpdateVirtualUserRequest, responses((status = 200, description = "Success", body = VirtualUser), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
pub async fn update_virtual_user(
    account: RuntimeAccount,
    State(state): State<ApiState>,
    Path(identity_id): Path<String>,
    Json(req): Json<UpdateVirtualUserRequest>,
) -> Result<Json<VirtualUser>, (StatusCode, Json<ErrorResponse>)> {
    // Management lifecycle changes always use management policy, including one's own account.
    if account.management.is_none() {
        if !account.permits_self(&identity_id) {
            return Err(
                ErrorResponse::new("Self-service only").into_response(StatusCode::FORBIDDEN)
            );
        }
        return update_me(account, State(state), Json(req)).await;
    }
    let org = account.management.as_ref().unwrap();
    state
        .dispatcher(org)
        .run(crate::domains::virtual_users::UpdateVirtualUserCmd {
            id: identity_id,
            req,
        })
        .await
}

#[utoipa::path(summary = "Delete virtual user.", delete, path = "/v1/virtual-users/{identity_id}", params(("identity_id" = String, Path)),  responses((status = 204, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
pub async fn delete_virtual_user(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(identity_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(crate::domains::virtual_users::DeleteVirtualUser { id: identity_id })
        .await
}

#[utoipa::path(summary = "Destroy virtual user.", post, path = "/v1/virtual-users/{identity_id}/delete", params(("identity_id" = String, Path)),  responses((status = 204, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
pub async fn destroy_virtual_user(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(identity_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(crate::domains::virtual_users::DestroyVirtualUser { id: identity_id })
        .await
}

async fn authorized_profile(
    state: &ApiState,
    org: &RuntimeAccount,
    raw: &str,
) -> Result<everruns_contracts::typed_id::VirtualUserId, (StatusCode, Json<ErrorResponse>)> {
    if org.permits_self(raw) {
        return Ok(org.id);
    }
    let Some(management) = org.management.as_ref() else {
        return Err(ErrorResponse::new("Self-service only").into_response(StatusCode::FORBIDDEN));
    };
    let id = raw.parse().map_err(|_| {
        ErrorResponse::new("Invalid virtual user ID").into_response(StatusCode::BAD_REQUEST)
    })?;
    VIRTUAL_USER_MANAGE
        .evaluate_with(
            state.auth.permission_resolver.as_ref(),
            &Caller::from(management),
        )
        .map_err(|_| {
            ErrorResponse::new("Permission denied").into_response(StatusCode::FORBIDDEN)
        })?;
    state
        .db
        .get_virtual_user(org.org_id, id)
        .await
        .map_err(|_| {
            ErrorResponse::new("Runtime account unavailable")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .ok_or_else(|| {
            ErrorResponse::new("Virtual user not found").into_response(StatusCode::NOT_FOUND)
        })?;
    Ok(id)
}
#[utoipa::path(summary = "Read the authenticated consumer runtime account.", get, path = "/v1/virtual-users/me",   responses((status = 200, description = "Success", body = VirtualUser), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
async fn get_me(
    org: RuntimeAccount,
    State(state): State<ApiState>,
) -> Result<Json<VirtualUser>, (StatusCode, Json<ErrorResponse>)> {
    let id = org.id;
    crate::domains::virtual_users::queries::get_by_id(&state.db, org.org_id, id)
        .await
        .map_err(|_| {
            ErrorResponse::new("Runtime account unavailable")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .map(Json)
        .ok_or_else(|| {
            ErrorResponse::new("Virtual user not found").into_response(StatusCode::NOT_FOUND)
        })
}
#[utoipa::path(summary = "Update the runtime profile without changing management identity or lifecycle.", patch, path = "/v1/virtual-users/me",  request_body = UpdateVirtualUserRequest, responses((status = 200, description = "Success", body = VirtualUser), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
async fn update_me(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Json(mut req): Json<UpdateVirtualUserRequest>,
) -> Result<Json<VirtualUser>, (StatusCode, Json<ErrorResponse>)> {
    let id = org.id;
    if req.status.is_some() {
        return Err(ErrorResponse::new("Self-service cannot change lifecycle")
            .into_response(StatusCode::FORBIDDEN));
    }
    if req
        .name
        .as_ref()
        .is_some_and(|name| name.trim().is_empty() || name.len() > 255)
    {
        return Err(ErrorResponse::new("Invalid name").into_response(StatusCode::BAD_REQUEST));
    }
    req.status = None;
    // The self shortcut shares profile validation and storage, with explicit self authority.
    if let everruns_db::UpdateField::Set(ref value) = req.locale {
        crate::domains::virtual_users::queries::validate_locale(value).map_err(|_| {
            ErrorResponse::new("Invalid locale").into_response(StatusCode::BAD_REQUEST)
        })?;
    }
    if let everruns_db::UpdateField::Set(ref value) = req.timezone {
        crate::domains::virtual_users::queries::validate_timezone(value).map_err(|_| {
            ErrorResponse::new("Invalid timezone").into_response(StatusCode::BAD_REQUEST)
        })?;
    }
    state
        .db
        .update_virtual_user(
            org.org_id,
            id,
            crate::storage::models::UpdateVirtualUser {
                name: req.name,
                description: req.description,
                avatar_url: req.avatar_url,
                locale: req.locale,
                timezone: req.timezone,
                status: None,
            },
        )
        .await
        .map_err(|_| {
            ErrorResponse::new("Runtime account unavailable")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    get_me(org, State(state)).await
}
#[utoipa::path(summary = "List verified external identity bindings.", get, path = "/v1/virtual-users/{identity_id}/bindings", params(("identity_id" = String, Path)),  responses((status = 200, description = "Success", body = serde_json::Value), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
async fn list_bindings(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Path(raw): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let id = authorized_profile(&state, &org, &raw).await?;
    let bindings = state
        .db
        .list_virtual_user_bindings(org.org_id, id)
        .await
        .map_err(|_| {
            ErrorResponse::new("Bindings unavailable")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    Ok(Json(serde_json::json!({"data":bindings})))
}
#[utoipa::path(summary = "List agent-facing preferences.", get, path = "/v1/virtual-users/{identity_id}/preferences", params(("identity_id" = String, Path)),  responses((status = 200, description = "Success", body = serde_json::Value), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
async fn list_preferences(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Path(raw): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let id = authorized_profile(&state, &org, &raw).await?;
    let rows = state
        .db
        .list_virtual_user_preferences(id, 100)
        .await
        .map_err(|_| {
            ErrorResponse::new("Preferences unavailable")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    Ok(Json(
        serde_json::json!({"data":rows.into_iter().map(preference_response).collect::<Vec<_>>()}),
    ))
}
#[utoipa::path(summary = "Read one agent-facing preference.", get, path = "/v1/virtual-users/{identity_id}/preferences/{key}", params(("identity_id" = String, Path),("key" = String, Path)),  responses((status = 200, description = "Success", body = serde_json::Value), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
async fn get_preference(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Path((raw, key)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let id = authorized_profile(&state, &org, &raw).await?;
    let row = state
        .db
        .get_virtual_user_preference(id, &key)
        .await
        .map_err(|_| {
            ErrorResponse::new("Preferences unavailable")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    row.map(|r| Json(preference_response(r))).ok_or_else(|| {
        ErrorResponse::new("Preference not found").into_response(StatusCode::NOT_FOUND)
    })
}
#[utoipa::path(summary = "Set a bounded agent-facing preference.", put, path = "/v1/virtual-users/{identity_id}/preferences/{key}", params(("identity_id" = String, Path),("key" = String, Path)), request_body = crate::api::user_preferences::SetPreferenceRequest, responses((status = 200, description = "Success", body = serde_json::Value), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
async fn set_preference(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Path((raw, key)): Path<(String, String)>,
    Json(body): Json<crate::api::user_preferences::SetPreferenceRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let id = authorized_profile(&state, &org, &raw).await?;
    crate::api::user_preferences::validate_preference(
        &key,
        &serde_json::to_string(&body.value).unwrap_or_default(),
    )
    .map_err(|status| ErrorResponse::new("Invalid preference").into_response(status))?;
    let row = state
        .db
        .set_virtual_user_preference(
            id,
            &key,
            &serde_json::to_string(&body.value).unwrap_or_default(),
            100,
        )
        .await
        .map_err(|_| {
            ErrorResponse::new("Invalid preference or preference limit exceeded")
                .into_response(StatusCode::BAD_REQUEST)
        })?;
    Ok(Json(preference_response(row)))
}
#[utoipa::path(summary = "Delete an agent-facing preference.", delete, path = "/v1/virtual-users/{identity_id}/preferences/{key}", params(("identity_id" = String, Path),("key" = String, Path)),  responses((status = 204, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
async fn delete_preference(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Path((raw, key)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let id = authorized_profile(&state, &org, &raw).await?;
    state
        .db
        .delete_virtual_user_preference(id, &key)
        .await
        .map_err(|_| {
            ErrorResponse::new("Preferences unavailable")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(summary = "Revoke an external binding and its runtime self authority.", delete, path = "/v1/virtual-users/{identity_id}/bindings/{binding_id}", params(("identity_id" = String, Path),("binding_id" = String, Path)),  responses((status = 204, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
async fn revoke_binding(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Path((raw, binding)): Path<(String, uuid::Uuid)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let id = authorized_profile(&state, &org, &raw).await?;
    if state
        .db
        .revoke_virtual_user_binding(org.org_id, id, binding)
        .await
        .map_err(|_| {
            ErrorResponse::new("Binding unavailable")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(
            ErrorResponse::new("Binding not found or cannot be unlinked")
                .into_response(StatusCode::NOT_FOUND),
        )
    }
}
fn preference_response(row: crate::storage::models::VirtualUserPreferenceRow) -> serde_json::Value {
    serde_json::json!({"key":row.key,"value":serde_json::from_str::<serde_json::Value>(&row.value).unwrap_or(serde_json::Value::Null),"created_at":row.created_at,"updated_at":row.updated_at})
}

#[utoipa::path(summary = "List recent sessions associated with the authorized virtual user.", get, path = "/v1/virtual-users/{identity_id}/sessions", params(("identity_id" = String, Path)),  responses((status = 200, description = "Success", body = serde_json::Value), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
async fn list_sessions(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Path(raw): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let id = authorized_profile(&state, &org, &raw).await?;
    let sessions = state
        .db
        .list_virtual_user_sessions(org.org_id, id)
        .await
        .map_err(|_| {
            ErrorResponse::new("Sessions unavailable")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    Ok(Json(
        serde_json::json!({"data":sessions.into_iter().map(|s|serde_json::json!({"id":s.id,"title":s.title,"status":s.status,"source":s.source,"created_at":s.created_at,"updated_at":s.updated_at})).collect::<Vec<_>>()}),
    ))
}
