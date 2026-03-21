// Users API routes
// Decision: Expose user listing for admin settings page (member management)
// Decision: Cookie-based org selection for consistent auth across all requests (including SSE)

use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, patch, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Utc};
use everruns_core::validate_org_public_id;

use super::common::{ListResponse, impl_auth_state};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use crate::auth::middleware::{AuthState, AuthUser, ResolvedOrg};
use crate::storage::models::UpdateUser;

/// Cookie name for storing selected organization
pub const ORG_COOKIE_NAME: &str = "everruns_org";

/// App state for users routes
#[derive(Clone)]
pub struct UsersState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl_auth_state!(UsersState);

/// User response for listing
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub roles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_provider: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Query parameters for listing users
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListUsersQuery {
    /// Search query to filter by name or email
    #[serde(default)]
    pub search: Option<String>,
}

/// Request to switch organization
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SwitchOrgRequest {
    /// Organization public ID to switch to
    #[schema(example = "org_2f3c1b3e6a9d4c6f8a1d4e9c9b7f21a0")]
    pub org_id: String,
}

/// Response from switch org endpoint
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SwitchOrgResponse {
    /// Whether the switch was successful
    pub success: bool,
    /// The organization ID that was switched to
    pub org_id: String,
}

/// Request to update current user's profile
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateProfileRequest {
    /// New display name
    #[schema(example = "Jane Doe")]
    pub name: String,
}

/// Response from profile update
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProfileResponse {
    pub id: String,
    pub email: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

/// Create users routes
pub fn routes(state: UsersState) -> Router {
    Router::new()
        .route("/v1/users", get(list_users))
        .route("/v1/users/me", patch(update_profile))
        .route("/v1/users/me/switch-org", post(switch_org))
        .with_state(state)
}

/// GET /v1/users - List users in current organization
///
/// Lists users that belong to the current organization with optional search filtering.
/// TM-TENANT-008: Enforces org isolation to prevent cross-tenant user enumeration.
#[utoipa::path(
    get,
    path = "/v1/users",
    params(
        ("search" = Option<String>, Query, description = "Search by name or email")
    ),
    responses(
        (status = 200, description = "List of users in organization", body = ListResponse<User>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    tag = "users"
)]
pub async fn list_users(
    State(state): State<UsersState>,
    org: ResolvedOrg,
    Query(query): Query<ListUsersQuery>,
) -> Result<Json<ListResponse<User>>, StatusCode> {
    // TM-TENANT-008: Filter users by org membership to enforce tenant isolation
    let rows = state
        .db
        .list_users_by_org(org.org_id, query.search.as_deref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to list users: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let users: Vec<User> = rows
        .into_iter()
        .map(|row| {
            let roles: Vec<String> = serde_json::from_value(row.roles.clone()).unwrap_or_default();
            User {
                id: row.id.to_string(),
                email: row.email,
                name: row.name,
                avatar_url: row.avatar_url,
                roles,
                auth_provider: row.auth_provider,
                created_at: row.created_at,
            }
        })
        .collect();

    Ok(Json(ListResponse::new(users)))
}

/// PATCH /v1/users/me - Update current user's profile
#[utoipa::path(
    patch,
    path = "/v1/users/me",
    request_body = UpdateProfileRequest,
    responses(
        (status = 200, description = "Profile updated", body = ProfileResponse),
        (status = 400, description = "Invalid request (empty name)"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    tag = "users"
)]
pub async fn update_profile(
    State(state): State<UsersState>,
    auth: AuthUser,
    Json(req): Json<UpdateProfileRequest>,
) -> Result<Json<ProfileResponse>, StatusCode> {
    let name = req.name.trim().to_string();
    if name.is_empty() || name.len() > 255 {
        return Err(StatusCode::BAD_REQUEST);
    }

    let input = UpdateUser {
        name: Some(name),
        ..Default::default()
    };

    let row = state
        .db
        .update_user(auth.id, input)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update user profile: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(ProfileResponse {
        id: row.id.to_string(),
        email: row.email,
        name: row.name,
        avatar_url: row.avatar_url,
    }))
}

/// POST /v1/users/me/switch-org - Switch current organization
///
/// Sets a cookie with the selected organization. This org will be used for all
/// subsequent requests (including SSE connections via EventSource).
/// The user must be a member of the requested organization.
#[utoipa::path(
    post,
    path = "/v1/users/me/switch-org",
    request_body = SwitchOrgRequest,
    responses(
        (status = 200, description = "Organization switched successfully", body = SwitchOrgResponse),
        (status = 400, description = "Invalid organization ID format"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Organization not found or user is not a member")
    ),
    tag = "users"
)]
pub async fn switch_org(
    State(state): State<UsersState>,
    auth: AuthUser,
    jar: CookieJar,
    Json(req): Json<SwitchOrgRequest>,
) -> Result<(CookieJar, Json<SwitchOrgResponse>), StatusCode> {
    // Validate org ID format
    if !validate_org_public_id(&req.org_id) {
        tracing::warn!("Invalid org ID format: {}", req.org_id);
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify user is a member of this org by querying the database.
    // Previously this checked auth.organizations (populated at auth time),
    // which meant newly created orgs couldn't be switched to until re-login.
    let user_orgs = state
        .db
        .list_user_organizations(auth.id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list user organizations: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let is_member = user_orgs.iter().any(|org| org.public_id == req.org_id);

    if !is_member {
        tracing::warn!(
            "User {} attempted to switch to org {} but is not a member",
            auth.id,
            req.org_id
        );
        return Err(StatusCode::NOT_FOUND);
    }

    // Build the org cookie
    let org_cookie = Cookie::build((ORG_COOKIE_NAME, req.org_id.clone()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .build();

    let jar = jar.add(org_cookie);

    tracing::info!("User {} switched to org {}", auth.id, req.org_id);

    Ok((
        jar,
        Json(SwitchOrgResponse {
            success: true,
            org_id: req.org_id,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_serialization() {
        let user = User {
            id: "123".to_string(),
            email: "test@example.com".to_string(),
            name: "Test User".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            auth_provider: Some("local".to_string()),
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&user).unwrap();
        assert!(json.contains("test@example.com"));
        assert!(json.contains("Test User"));
    }

    #[test]
    fn test_list_users_query_deserialize() {
        let query: ListUsersQuery = serde_json::from_str(r#"{"search": "test"}"#).unwrap();
        assert_eq!(query.search, Some("test".to_string()));

        let query: ListUsersQuery = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(query.search, None);
    }

    #[test]
    fn test_update_profile_request_deserialize() {
        let req: UpdateProfileRequest = serde_json::from_str(r#"{"name": "New Name"}"#).unwrap();
        assert_eq!(req.name, "New Name");
    }

    #[test]
    fn test_profile_response_serialization() {
        let resp = ProfileResponse {
            id: "123".to_string(),
            email: "test@example.com".to_string(),
            name: "Test User".to_string(),
            avatar_url: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("Test User"));
        assert!(!json.contains("avatar_url"));
    }
}
