// Organization CRUD HTTP routes (Multitenancy)
//
// Note: Organization routes are NOT org-scoped (they are at the root level)
// because they manage organizations themselves.

use crate::auth::middleware::{AuthState, AuthUser, OrgAdmin, OrgContext};
use crate::storage::{StorageBackend, models::UpdateOrganizationSettings};
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::{
    DEFAULT_ORG_ID, OrgRole, Organization, generate_org_public_id, validate_org_public_id,
};

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
    /// Default LLM model for this organization. Must be an installed model.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub default_model_id: Option<everruns_core::ModelId>,
    /// Default harness to preselect in the UI for new sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a000070008000000000000602")]
    pub default_harness_id: Option<everruns_core::HarnessId>,
    /// Base harness to use when a session is started without an explicit harness_id.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a000070008000000000000601")]
    pub base_harness_id: Option<everruns_core::HarnessId>,
}

/// Response for organization operations
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OrganizationResponse {
    /// External identifier (org_<32-hex-chars>)
    pub id: String,
    /// Display name
    pub name: String,
    /// Default LLM model for the organization.
    #[schema(value_type = Option<String>)]
    pub default_model_id: Option<everruns_core::ModelId>,
    /// Default harness to preselect in the UI.
    #[schema(value_type = Option<String>)]
    pub default_harness_id: Option<everruns_core::HarnessId>,
    /// Base harness used when session creation omits harness_id.
    #[schema(value_type = Option<String>)]
    pub base_harness_id: Option<everruns_core::HarnessId>,
    /// When the organization was created
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When the organization was last updated
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Build organization routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/orgs",
            get(list_organizations).post(create_organization),
        )
        .route(
            "/v1/orgs/{org}",
            get(get_organization).patch(update_organization),
        )
        // Organization members
        .route("/v1/orgs/{org}/members", get(list_members).post(add_member))
        .route(
            "/v1/orgs/{org}/members/{user_id}",
            axum::routing::patch(update_member_role).delete(remove_member),
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
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<ListResponse<OrganizationResponse>>, (StatusCode, Json<ErrorResponse>)> {
    // Query the database for fresh membership data.
    // Previously this read from user.organizations (populated at auth time),
    // which meant newly created orgs were invisible until re-login.
    let org_rows = state
        .db
        .list_user_organizations(user.id)
        .await
        .log_internal_error_json("list user organizations")?;

    let mut orgs = Vec::with_capacity(org_rows.len());
    for row in &org_rows {
        // Fetch full org details (including settings, timestamps) per org
        if let Some(org_row) = state
            .db
            .get_organization(row.org_id)
            .await
            .log_internal_error_json("get organization")?
        {
            orgs.push(build_organization_response(&state.db, row.org_id, org_row).await?);
        }
    }

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
            created_by: Some(user.id),
        })
        .await
        .log_internal_error_json("create organization")?;

    // Add creator as organization owner
    state
        .db
        .add_organization_member(row.org_id, user.id, "owner")
        .await
        .log_internal_error_json("add organization member")?;

    // Initialize built-in harnesses for the new organization
    if let Err(e) = crate::org_init::initialize_org_harnesses(&state.db, row.org_id).await {
        tracing::warn!(
            org_id = row.org_id,
            error = %e,
            "Failed to initialize built-in harnesses for new org (non-fatal)"
        );
    }

    let response = build_organization_response(&state.db, row.org_id, row).await?;
    Ok((StatusCode::CREATED, Json(response)))
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

    // Check user membership from DB (return 404 for non-members to prevent enumeration)
    if !is_member_of_public_db(&state.db, user.id, &org_public_id).await? {
        return Err(ErrorResponse::not_found("Organization"));
    }

    // Fetch organization details
    let row = state
        .db
        .get_organization_by_public_id(&org_public_id)
        .await
        .log_internal_error_json("get organization")?
        .ok_or_not_found_json("Organization")?;

    Ok(Json(
        build_organization_response(&state.db, row.org_id, row).await?,
    ))
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

    // Check user membership from DB
    if !is_member_of_public_db(&state.db, user.id, &org_public_id).await? {
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

    let UpdateOrganizationRequest {
        name,
        default_model_id,
        default_harness_id,
        base_harness_id,
    } = req;

    // Validate referenced IDs exist
    if let Some(ref model_id) = default_model_id {
        // Verify the model exists and is installed
        let model = state
            .db
            .get_llm_model_with_provider(org_row.org_id, model_id.uuid())
            .await
            .log_internal_error_json("resolve default model")?
            .ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("Model not found")),
                )
            })?;
        if !model.installed {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new(
                    "Default model must be an installed model",
                )),
            ));
        }
    }
    if let Some(default_harness_id) = default_harness_id {
        state
            .db
            .get_harness(org_row.org_id, default_harness_id)
            .await
            .log_internal_error_json("resolve default harness")?
            .ok_or_not_found_json("Harness")?;
    }
    if let Some(base_harness_id) = base_harness_id {
        state
            .db
            .get_harness(org_row.org_id, base_harness_id)
            .await
            .log_internal_error_json("resolve base harness")?
            .ok_or_not_found_json("Harness")?;
    }

    // Update organization
    let input = UpdateOrganization { name };

    let row = state
        .db
        .update_organization(org_row.org_id, input)
        .await
        .log_internal_error_json("update organization")?
        .ok_or_not_found_json("Organization")?;

    if default_model_id.is_some() || default_harness_id.is_some() || base_harness_id.is_some() {
        state
            .db
            .patch_organization_settings(
                org_row.org_id,
                UpdateOrganizationSettings {
                    default_model_id: default_model_id.map(Some),
                    default_harness_id: default_harness_id.map(Some),
                    base_harness_id: base_harness_id.map(Some),
                },
            )
            .await
            .log_internal_error_json("update organization settings")?;
    }

    Ok(Json(
        build_organization_response(&state.db, row.org_id, row).await?,
    ))
}

/// Check membership by querying the DB (avoids stale auth context).
async fn is_member_of_public_db(
    db: &StorageBackend,
    user_id: uuid::Uuid,
    org_public_id: &str,
) -> Result<bool, (StatusCode, Json<ErrorResponse>)> {
    let orgs = db
        .list_user_organizations(user_id)
        .await
        .log_internal_error_json("list user organizations")?;
    Ok(orgs.iter().any(|o| o.public_id == org_public_id))
}

async fn build_organization_response(
    db: &StorageBackend,
    org_id: i64,
    row: crate::storage::OrganizationRow,
) -> Result<OrganizationResponse, (StatusCode, Json<ErrorResponse>)> {
    let settings = db
        .get_organization_settings(org_id)
        .await
        .log_internal_error_json("get organization settings")?;

    let org = Organization {
        public_id: row.public_id,
        name: row.name,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };

    Ok(OrganizationResponse {
        id: org.public_id,
        name: org.name,
        default_model_id: settings.as_ref().and_then(|s| s.default_model_id),
        default_harness_id: settings.as_ref().and_then(|s| s.default_harness_id),
        base_harness_id: settings.as_ref().and_then(|s| s.base_harness_id),
        created_at: org.created_at,
        updated_at: org.updated_at,
    })
}

// ============================================================================
// Organization Members
// ============================================================================

/// Response for organization member
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemberResponse {
    pub user_id: String,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub role: String,
    pub joined_at: String,
}

/// Request to add a member
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddMemberRequest {
    pub user_id: String,
    #[serde(default = "default_member_role")]
    pub role: String,
}

fn default_member_role() -> String {
    "member".to_string()
}

/// Request to update member role
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateMemberRoleRequest {
    pub role: String,
}

/// GET /v1/orgs/:org/members - List organization members
pub async fn list_members(
    State(state): State<AppState>,
    org: OrgContext,
) -> Result<Json<ListResponse<MemberResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let members = state
        .db
        .list_organization_members_with_users(org.org_id)
        .await
        .log_internal_error_json("list organization members")?;

    let items: Vec<MemberResponse> = members
        .into_iter()
        .map(|m| MemberResponse {
            user_id: m.user_id.to_string(),
            email: m.email,
            name: m.name,
            avatar_url: m.avatar_url,
            role: m.role,
            joined_at: m.joined_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(ListResponse::new(items)))
}

/// POST /v1/orgs/:org/members - Add a member (Admin+)
pub async fn add_member(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    user: AuthUser,
    Json(req): Json<AddMemberRequest>,
) -> Result<(StatusCode, Json<MemberResponse>), (StatusCode, Json<ErrorResponse>)> {
    // Parse and validate role
    let role: OrgRole = req.role.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "Invalid role. Must be 'owner', 'admin', or 'member'",
            )),
        )
    })?;

    // Only owners can add owners
    if role == OrgRole::Owner && !org.role.has_permission(OrgRole::Owner) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new("Only owners can add owners")),
        ));
    }

    let target_user_id: uuid::Uuid = req.user_id.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("Invalid user_id")),
        )
    })?;

    // Verify user exists
    let target_user = state
        .db
        .get_user(target_user_id)
        .await
        .log_internal_error_json("get user")?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse::new("User not found")),
            )
        })?;

    // Check if already a member
    let existing = state
        .db
        .get_organization_member(org.org_id, target_user_id)
        .await
        .log_internal_error_json("check membership")?;

    if existing.is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse::new("User is already a member")),
        ));
    }

    // Add member
    let member_row = state
        .db
        .add_organization_member(org.org_id, target_user_id, role.as_str())
        .await
        .log_internal_error_json("add organization member")?;

    let _ = user; // Silence unused warning

    Ok((
        StatusCode::CREATED,
        Json(MemberResponse {
            user_id: target_user_id.to_string(),
            email: target_user.email,
            name: target_user.name,
            avatar_url: target_user.avatar_url,
            role: member_row.role,
            joined_at: member_row.created_at.to_rfc3339(),
        }),
    ))
}

/// PATCH /v1/orgs/:org/members/:user_id - Update member role
pub async fn update_member_role(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    Path((_org_public_id, user_id_str)): Path<(String, String)>,
    Json(req): Json<UpdateMemberRoleRequest>,
) -> Result<Json<MemberResponse>, (StatusCode, Json<ErrorResponse>)> {
    let new_role: OrgRole = req.role.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "Invalid role. Must be 'owner', 'admin', or 'member'",
            )),
        )
    })?;

    let target_user_id: uuid::Uuid = user_id_str.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("Invalid user_id")),
        )
    })?;

    // Get current member info
    let current = state
        .db
        .get_organization_member(org.org_id, target_user_id)
        .await
        .log_internal_error_json("get member")?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse::new("Member not found")),
            )
        })?;

    let current_role: OrgRole = current.role.parse().unwrap_or(OrgRole::Member);

    // Only owners can change owner roles
    if (current_role == OrgRole::Owner || new_role == OrgRole::Owner)
        && !org.role.has_permission(OrgRole::Owner)
    {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new("Only owners can change owner roles")),
        ));
    }

    // Cannot demote last owner
    if current_role == OrgRole::Owner && new_role != OrgRole::Owner {
        let owner_count = state
            .db
            .count_organization_owners(org.org_id)
            .await
            .log_internal_error_json("count owners")?;
        if owner_count <= 1 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new("Cannot remove the last owner")),
            ));
        }
    }

    // Update role
    let updated = state
        .db
        .update_organization_member_role(org.org_id, target_user_id, new_role.as_str())
        .await
        .log_internal_error_json("update member role")?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse::new("Member not found")),
            )
        })?;

    Ok(Json(MemberResponse {
        user_id: target_user_id.to_string(),
        email: current.email,
        name: current.name,
        avatar_url: current.avatar_url,
        role: updated.role,
        joined_at: current.joined_at.to_rfc3339(),
    }))
}

/// DELETE /v1/orgs/:org/members/:user_id - Remove member (Owner or self)
pub async fn remove_member(
    State(state): State<AppState>,
    org: OrgContext,
    user: AuthUser,
    Path((_org_public_id, user_id_str)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let target_user_id: uuid::Uuid = user_id_str.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("Invalid user_id")),
        )
    })?;

    let is_self = target_user_id == user.id;

    // Must be owner to remove others (self-removal always allowed)
    if !is_self && !org.role.has_permission(OrgRole::Owner) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new("Only owners can remove members")),
        ));
    }

    // Check if target is owner — cannot remove last owner
    let member = state
        .db
        .get_organization_member(org.org_id, target_user_id)
        .await
        .log_internal_error_json("get member")?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse::new("Member not found")),
            )
        })?;

    if member.role == "owner" {
        let owner_count = state
            .db
            .count_organization_owners(org.org_id)
            .await
            .log_internal_error_json("count owners")?;
        if owner_count <= 1 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new("Cannot remove the last owner")),
            ));
        }
    }

    let removed = state
        .db
        .remove_organization_member(org.org_id, target_user_id)
        .await
        .log_internal_error_json("remove organization member")?;

    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Member not found")),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_organization_response_fields() {
        let response = OrganizationResponse {
            id: "org_00000000000000000000000000000001".to_string(),
            name: "Test Org".to_string(),
            default_model_id: None,
            default_harness_id: Some("harness_01933b5a000070008000000000000602".parse().unwrap()),
            base_harness_id: Some("harness_01933b5a000070008000000000000601".parse().unwrap()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        assert_eq!(response.id, "org_00000000000000000000000000000001");
        assert_eq!(response.name, "Test Org");
        assert!(response.default_harness_id.is_some());
        assert!(response.base_harness_id.is_some());
    }

    #[test]
    fn test_create_request_deserialization() {
        let json = r#"{"name": "Acme Corp"}"#;
        let req: CreateOrganizationRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "Acme Corp");
    }

    #[test]
    fn test_create_request_empty_name() {
        let json = r#"{"name": ""}"#;
        let req: CreateOrganizationRequest = serde_json::from_str(json).unwrap();
        assert!(req.name.is_empty());
    }

    #[test]
    fn test_update_request_partial() {
        let json = r#"{}"#;
        let req: UpdateOrganizationRequest = serde_json::from_str(json).unwrap();
        assert!(req.name.is_none());
        assert!(req.default_harness_id.is_none());
        assert!(req.base_harness_id.is_none());

        let json = r#"{
            "name": "New Name",
            "default_harness_id": "harness_01933b5a000070008000000000000602",
            "base_harness_id": "harness_01933b5a000070008000000000000601"
        }"#;
        let req: UpdateOrganizationRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name.unwrap(), "New Name");
        assert!(req.default_harness_id.is_some());
        assert!(req.base_harness_id.is_some());
    }
}
