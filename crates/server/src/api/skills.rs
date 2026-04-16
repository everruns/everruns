// Skills registry HTTP routes
// Routes: /v1/skills/...
//
// CRUD for Agent Skills (agentskills.io format).
// Supports both SKILL.md text upload and ZIP archive upload.

use crate::api::common::{
    ApiResult, ErrorResponse, ListResponse, UrlBuilder, WithUrls, impl_auth_state,
};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::Command;
use crate::services::CapabilityService;
use crate::services::SkillService;
use crate::services::skill::{SKILL_DANGEROUS, SKILL_MANAGE, SKILL_VIEW};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use axum_extra::extract::Multipart;
use everruns_core::{
    Caller, ResourceConfigResponse, Skill, SkillContent, SkillStatus, SkillValidationResult,
    evaluate_policies_with,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

// ============================================
// Constants
// ============================================

/// Maximum ZIP archive size (10 MB + overhead)
const MAX_ARCHIVE_UPLOAD: usize = 11 * 1024 * 1024;

// ============================================
// Request/Response types
// ============================================

/// Request to create a skill from SKILL.md content
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSkillRequest {
    /// Full SKILL.md content (YAML frontmatter + markdown body)
    #[schema(
        example = "---\nname: pdf-processing\ndescription: Extract text from PDFs.\n---\n\n# PDF Processing\n\nUse pdfplumber..."
    )]
    pub skill_md: String,
}

/// Request to update a skill
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateSkillRequest {
    /// Updated SKILL.md content (re-parses frontmatter)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_md: Option<String>,
    /// Update status
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<SkillStatus>,
}

/// Request to validate a SKILL.md
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ValidateSkillRequest {
    /// SKILL.md content to validate
    pub skill_md: String,
}

// ============================================
// App State
// ============================================

/// Query parameters for listing skills.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListSkillsQuery {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
    /// Include archived skills. Deleted skills never appear in lists.
    pub include_archived: Option<bool>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub service: Arc<SkillService>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
    ) -> Self {
        Self {
            service: Arc::new(SkillService::new(db.clone())),
            db,
            capability_service,
            auth,
        }
    }

    /// Build a domain Ctx from this AppState for the given org.
    pub fn ctx(&self, org: &ResolvedOrg) -> crate::domains::common::Ctx {
        crate::domains::common::Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.capability_service.clone(),
            None,
        )
    }
}

impl_auth_state!(AppState);

// ============================================
// Helpers
// ============================================

/// GET /v1/skills/config
#[utoipa::path(
    get,
    path = "/v1/skills/config",
    responses(
        (status = 200, description = "Resource config for skills", body = ResourceConfigResponse),
    ),
    tag = "skills"
)]
pub async fn skill_config(
    State(auth): State<AuthState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[&SKILL_VIEW, &SKILL_MANAGE, &SKILL_DANGEROUS],
    );
    Json(ResourceConfigResponse { policies })
}

// ============================================
// Routes
// ============================================

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/skills", post(create_skill).get(list_skills))
        .route("/v1/skills/config", get(skill_config))
        .route(
            "/v1/skills/upload",
            post(upload_skill).layer(DefaultBodyLimit::max(MAX_ARCHIVE_UPLOAD)),
        )
        .route("/v1/skills/validate", post(validate_skill))
        .route(
            "/v1/skills/{skill_id}",
            get(get_skill).patch(update_skill).delete(delete_skill),
        )
        .route("/v1/skills/{skill_id}/delete", post(destroy_skill))
        .route("/v1/skills/{skill_id}/content", get(get_skill_content))
        .with_state(state)
}

// ============================================
// Handlers
// ============================================

/// POST /v1/skills - Create skill from SKILL.md
#[utoipa::path(
    post,
    path = "/v1/skills",
    request_body = CreateSkillRequest,
    responses(
        (status = 201, description = "Skill created", body = WithUrls<Skill>),
        (status = 409, description = "Duplicate skill name", body = ErrorResponse),
        (status = 422, description = "Invalid SKILL.md", body = ErrorResponse),
    ),
    tag = "skills"
)]
pub async fn create_skill(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateSkillRequest>,
) -> Result<(StatusCode, Json<WithUrls<Skill>>), (StatusCode, Json<ErrorResponse>)> {
    let skill = crate::domains::skills::CreateSkill(req)
        .execute(&state.ctx(&org))
        .await?;

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((StatusCode::CREATED, Json(urls.wrap(skill))))
}

/// POST /v1/skills/upload - Create skill from ZIP archive
#[utoipa::path(
    post,
    path = "/v1/skills/upload",
    responses(
        (status = 201, description = "Skill created from archive", body = WithUrls<Skill>),
        (status = 409, description = "Duplicate skill name", body = ErrorResponse),
        (status = 413, description = "Archive too large", body = ErrorResponse),
        (status = 422, description = "Invalid archive or SKILL.md", body = ErrorResponse),
    ),
    tag = "skills"
)]
pub async fn upload_skill(
    org: ResolvedOrg,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<WithUrls<Skill>>), (StatusCode, Json<ErrorResponse>)> {
    let mut file_data: Option<Vec<u8>> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        ErrorResponse::new(format!("Failed to parse multipart: {e}"))
            .into_response(StatusCode::BAD_REQUEST)
    })? {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            let data = field.bytes().await.map_err(|e| {
                ErrorResponse::new(format!("Failed to read file: {e}"))
                    .into_response(StatusCode::BAD_REQUEST)
            })?;
            file_data = Some(data.to_vec());
        }
    }

    let data = file_data.ok_or_else(|| {
        ErrorResponse::new("No 'file' field found in multipart upload")
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    let caller = Caller::from(&org);
    let skill = state
        .service
        .create_from_archive(&caller, data)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("already exists") {
                ErrorResponse::new(msg).into_response(StatusCode::CONFLICT)
            } else if msg.contains("too large") {
                ErrorResponse::new(msg).into_response(StatusCode::PAYLOAD_TOO_LARGE)
            } else if msg.contains("Invalid")
                || msg.contains("traversal")
                || msg.contains("must contain")
            {
                ErrorResponse::new(msg).into_response(StatusCode::UNPROCESSABLE_ENTITY)
            } else {
                tracing::error!("Failed to upload skill: {}", e);
                ErrorResponse::internal_error()
            }
        })?;

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((StatusCode::CREATED, Json(urls.wrap(skill))))
}

/// GET /v1/skills - List all skills
#[utoipa::path(
    get,
    path = "/v1/skills",
    responses(
        (status = 200, description = "List of skills", body = ListResponse<WithUrls<Skill>>),
    ),
    params(ListSkillsQuery),
    tag = "skills"
)]
pub async fn list_skills(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListSkillsQuery>,
) -> ApiResult<ListResponse<WithUrls<Skill>>> {
    let skills = crate::domains::skills::ListSkills {
        search: query.search,
        include_archived: query.include_archived.unwrap_or(false),
    }
    .execute(&state.ctx(&org))
    .await?;

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(ListResponse::new(skills).with_urls(&urls)))
}

/// GET /v1/skills/{skill_id} - Get skill by ID
#[utoipa::path(
    get,
    path = "/v1/skills/{skill_id}",
    params(
        ("skill_id" = String, Path, description = "Skill ID (prefixed, e.g., skill_...)")
    ),
    responses(
        (status = 200, description = "Skill found", body = WithUrls<Skill>),
        (status = 404, description = "Skill not found"),
    ),
    tag = "skills"
)]
pub async fn get_skill(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
) -> ApiResult<WithUrls<Skill>> {
    let skill = crate::domains::skills::GetSkill { id: skill_id }
        .execute(&state.ctx(&org))
        .await?;

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap(skill)))
}

/// GET /v1/skills/{skill_id}/content - Get full skill content
#[utoipa::path(
    get,
    path = "/v1/skills/{skill_id}/content",
    params(
        ("skill_id" = String, Path, description = "Skill ID")
    ),
    responses(
        (status = 200, description = "Skill content", body = SkillContent),
        (status = 404, description = "Skill not found"),
    ),
    tag = "skills"
)]
pub async fn get_skill_content(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
) -> ApiResult<SkillContent> {
    let content = crate::domains::skills::GetSkillContent { id: skill_id }
        .execute(&state.ctx(&org))
        .await?;

    Ok(Json(content))
}

/// PATCH /v1/skills/{skill_id} - Update skill
#[utoipa::path(
    patch,
    path = "/v1/skills/{skill_id}",
    request_body = UpdateSkillRequest,
    responses(
        (status = 200, description = "Skill updated", body = WithUrls<Skill>),
        (status = 404, description = "Skill not found"),
        (status = 409, description = "Duplicate skill name", body = ErrorResponse),
        (status = 422, description = "Invalid SKILL.md", body = ErrorResponse),
    ),
    tag = "skills"
)]
pub async fn update_skill(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
    Json(req): Json<UpdateSkillRequest>,
) -> ApiResult<WithUrls<Skill>> {
    let skill = crate::domains::skills::UpdateSkillCmd { id: skill_id, req }
        .execute(&state.ctx(&org))
        .await?;

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap(skill)))
}

/// DELETE /v1/skills/{skill_id} - Delete skill
#[utoipa::path(
    delete,
    path = "/v1/skills/{skill_id}",
    params(
        ("skill_id" = String, Path, description = "Skill ID")
    ),
    responses(
        (status = 204, description = "Skill deleted"),
        (status = 404, description = "Skill not found"),
    ),
    tag = "skills"
)]
pub async fn delete_skill(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    crate::domains::skills::DeleteSkill { id: skill_id }
        .execute(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn destroy_skill(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    crate::domains::skills::DestroySkill { id: skill_id }
        .execute(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/skills/validate - Validate SKILL.md content
#[utoipa::path(
    post,
    path = "/v1/skills/validate",
    request_body = ValidateSkillRequest,
    responses(
        (status = 200, description = "Validation result", body = SkillValidationResult),
    ),
    tag = "skills"
)]
pub async fn validate_skill(
    _org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<ValidateSkillRequest>,
) -> Json<SkillValidationResult> {
    Json(state.service.validate(&req.skill_md))
}
