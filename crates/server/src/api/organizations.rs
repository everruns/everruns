// Organization CRUD HTTP routes (Multitenancy)
//
// Note: Organization routes are NOT org-scoped (they are at the root level)
// because they manage organizations themselves.

use crate::auth::middleware::{AuthState, AuthUser};
use crate::storage::StorageBackend;
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::{DEFAULT_ORG_ID, Organization, generate_org_public_id, validate_org_public_id};

use super::common::{ApiOptionExt, ApiResultExt, ErrorResponse, ListResponse};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

/// App state for organization routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self { db, auth }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Request to create a new organization
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateOrganizationRequest {
    /// The display name of the organization.
    #[schema(example = "Acme Corp")]
    pub name: String,
}

/// Request to update an organization
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateOrganizationRequest {
    /// The display name of the organization.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Acme Corporation")]
    pub name: Option<String>,
}

/// Response for organization operations
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OrganizationResponse {
    /// External identifier (org_<32-hex-chars>)
    pub id: String,
    /// Display name
    pub name: String,
    /// When the organization was created
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When the organization was last updated
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<Organization> for OrganizationResponse {
    fn from(org: Organization) -> Self {
        Self {
            id: org.public_id,
            name: org.name,
            created_at: org.created_at,
            updated_at: org.updated_at,
        }
    }
}

/// Build organization routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/orgs",
            get(list_organizations).post(create_organization),
        )
        .route(
            "/v1/orgs/:org",
            get(get_organization).patch(update_organization),
        )
        .with_state(state)
}

/// GET /v1/orgs - List organizations the current user belongs to
#[utoipa::path(
    get,
    path = "/v1/orgs",
    tag = "Organizations",
    responses(
        (status = 200, description = "List of organizations", body = ListResponse<OrganizationResponse>)
    ),
    security(
        ("bearerAuth" = []),
        ("cookieAuth" = [])
    )
)]
pub async fn list_organizations(
    user: AuthUser,
) -> Result<Json<ListResponse<OrganizationResponse>>, (StatusCode, Json<ErrorResponse>)> {
    // Return the organizations from the user's auth context
    // This avoids an extra database query since we already have the memberships
    let orgs: Vec<OrganizationResponse> = user
        .organizations
        .iter()
        .map(|m| OrganizationResponse {
            id: m.public_id.clone(),
            name: m.name.clone(),
            // These timestamps are not available in OrgMembership, use placeholder
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .collect();

    Ok(Json(ListResponse::new(orgs)))
}

/// POST /v1/orgs - Create a new organization
#[utoipa::path(
    post,
    path = "/v1/orgs",
    tag = "Organizations",
    request_body = CreateOrganizationRequest,
    responses(
        (status = 201, description = "Organization created", body = OrganizationResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse)
    ),
    security(
        ("bearerAuth" = []),
        ("cookieAuth" = [])
    )
)]
pub async fn create_organization(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateOrganizationRequest>,
) -> Result<(StatusCode, Json<OrganizationResponse>), (StatusCode, Json<ErrorResponse>)> {
    use crate::storage::models::CreateOrganizationRow;

    // Validate input
    if req.name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("Organization name cannot be empty")),
        ));
    }

    if req.name.len() > 255 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "Organization name cannot exceed 255 characters",
            )),
        ));
    }

    // Generate public_id
    let public_id = generate_org_public_id();

    // Create organization
    let row = state
        .db
        .create_organization(CreateOrganizationRow {
            public_id: public_id.clone(),
            name: req.name,
        })
        .await
        .log_internal_error_json("create organization")?;

    // Add creator as organization member
    state
        .db
        .add_organization_member(row.org_id, user.id)
        .await
        .log_internal_error_json("add organization member")?;

    let org = Organization {
        public_id: row.public_id,
        name: row.name,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };

    Ok((StatusCode::CREATED, Json(OrganizationResponse::from(org))))
}

/// GET /v1/orgs/:org - Get organization details
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}",
    tag = "Organizations",
    params(
        ("org" = String, Path, description = "Organization public ID")
    ),
    responses(
        (status = 200, description = "Organization details", body = OrganizationResponse),
        (status = 404, description = "Organization not found", body = ErrorResponse)
    ),
    security(
        ("bearerAuth" = []),
        ("cookieAuth" = [])
    )
)]
pub async fn get_organization(
    State(state): State<AppState>,
    user: AuthUser,
    Path(org_public_id): Path<String>,
) -> Result<Json<OrganizationResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate format
    if !validate_org_public_id(&org_public_id) {
        return Err(ErrorResponse::not_found("Organization"));
    }

    // Check user membership (return 404 for non-members to prevent enumeration)
    if !user.is_member_of_public(&org_public_id) {
        return Err(ErrorResponse::not_found("Organization"));
    }

    // Fetch organization details
    let row = state
        .db
        .get_organization_by_public_id(&org_public_id)
        .await
        .log_internal_error_json("get organization")?
        .ok_or_not_found_json("Organization")?;

    let org = Organization {
        public_id: row.public_id,
        name: row.name,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };

    Ok(Json(OrganizationResponse::from(org)))
}

/// PATCH /v1/orgs/:org - Update organization
#[utoipa::path(
    patch,
    path = "/v1/orgs/{org}",
    tag = "Organizations",
    params(
        ("org" = String, Path, description = "Organization public ID")
    ),
    request_body = UpdateOrganizationRequest,
    responses(
        (status = 200, description = "Organization updated", body = OrganizationResponse),
        (status = 404, description = "Organization not found", body = ErrorResponse)
    ),
    security(
        ("bearerAuth" = []),
        ("cookieAuth" = [])
    )
)]
pub async fn update_organization(
    State(state): State<AppState>,
    user: AuthUser,
    Path(org_public_id): Path<String>,
    Json(req): Json<UpdateOrganizationRequest>,
) -> Result<Json<OrganizationResponse>, (StatusCode, Json<ErrorResponse>)> {
    use crate::storage::models::UpdateOrganization;

    // Validate format
    if !validate_org_public_id(&org_public_id) {
        return Err(ErrorResponse::not_found("Organization"));
    }

    // Check user membership
    if !user.is_member_of_public(&org_public_id) {
        return Err(ErrorResponse::not_found("Organization"));
    }

    // Validate input
    if let Some(ref name) = req.name {
        if name.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new("Organization name cannot be empty")),
            ));
        }
        if name.len() > 255 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new(
                    "Organization name cannot exceed 255 characters",
                )),
            ));
        }
    }

    // Get org_id from public_id
    let org_row = state
        .db
        .get_organization_by_public_id(&org_public_id)
        .await
        .log_internal_error_json("get organization")?
        .ok_or_not_found_json("Organization")?;

    // Cannot update default organization name
    if org_row.org_id == DEFAULT_ORG_ID && req.name.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("Cannot update default organization")),
        ));
    }

    // Update organization
    let input = UpdateOrganization { name: req.name };

    let row = state
        .db
        .update_organization(org_row.org_id, input)
        .await
        .log_internal_error_json("update organization")?
        .ok_or_not_found_json("Organization")?;

    let org = Organization {
        public_id: row.public_id,
        name: row.name,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };

    Ok(Json(OrganizationResponse::from(org)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_organization_response_from() {
        let org = Organization {
            public_id: "org_00000000000000000000000000000001".to_string(),
            name: "Test Org".to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let response = OrganizationResponse::from(org.clone());
        assert_eq!(response.id, org.public_id);
        assert_eq!(response.name, org.name);
    }
}
