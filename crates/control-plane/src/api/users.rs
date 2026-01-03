// Users API routes
// Decision: Expose user listing for admin settings page (member management)

use crate::storage::Database;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};

use super::common::ListResponse;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use crate::auth::middleware::{AuthState, AuthUser, FromRef};

/// App state for users routes
#[derive(Clone)]
pub struct UsersState {
    pub db: Arc<Database>,
    pub auth: AuthState,
}

impl FromRef<UsersState> for AuthState {
    fn from_ref(input: &UsersState) -> Self {
        input.auth.clone()
    }
}

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

/// Create users routes
pub fn routes(state: UsersState) -> Router {
    Router::new()
        .route("/v1/users", get(list_users))
        .with_state(state)
}

/// GET /v1/users - List all users
///
/// Lists all users in the system with optional search filtering.
/// Requires authentication (admin access recommended).
#[utoipa::path(
    get,
    path = "/v1/users",
    params(
        ("search" = Option<String>, Query, description = "Search by name or email")
    ),
    responses(
        (status = 200, description = "List of users", body = ListResponse<User>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    tag = "users"
)]
pub async fn list_users(
    State(state): State<UsersState>,
    _auth: AuthUser, // Require authentication
    Query(query): Query<ListUsersQuery>,
) -> Result<Json<ListResponse<User>>, StatusCode> {
    let rows = state
        .db
        .list_users(query.search.as_deref())
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
}
