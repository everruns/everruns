// Skills registry HTTP routes
// Routes: /v1/skills/...
//
// CRUD for Agent Skills (agentskills.io format).
// Supports both SKILL.md text upload and ZIP archive upload.

use crate::api::common::{ApiOptionExt, ApiResultExt, ErrorResponse, ListResponse};
use crate::auth::{AuthState, ResolvedOrg};
use crate::services::SkillService;
use crate::storage::StorageBackend;
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    routing::{get, post},
};
use axum_extra::extract::Multipart;
use everruns_core::{Skill, SkillContent, SkillId, SkillStatus, SkillValidationResult};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;

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

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<SkillService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self {
            service: Arc::new(SkillService::new(db)),
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

// ============================================
// Routes
// ============================================

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/skills", post(create_skill).get(list_skills))
        .route(
            "/v1/skills/upload",
            post(upload_skill).layer(DefaultBodyLimit::max(MAX_ARCHIVE_UPLOAD)),
        )
        .route("/v1/skills/validate", post(validate_skill))
        .route(
            "/v1/skills/{skill_id}",
            get(get_skill).patch(update_skill).delete(delete_skill),
        )
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
        (status = 201, description = "Skill created", body = Skill),
        (status = 409, description = "Duplicate skill name", body = ErrorResponse),
        (status = 422, description = "Invalid SKILL.md", body = ErrorResponse),
    ),
    tag = "skills"
)]
pub async fn create_skill(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateSkillRequest>,
) -> Result<(StatusCode, Json<Skill>), (StatusCode, Json<ErrorResponse>)> {
    let skill = state.service.create(org.org_id, req).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("already exists") {
            ErrorResponse::new(msg).into_response(StatusCode::CONFLICT)
        } else if msg.contains("Invalid SKILL.md") {
            ErrorResponse::new(msg).into_response(StatusCode::UNPROCESSABLE_ENTITY)
        } else {
            tracing::error!("Failed to create skill: {}", e);
            ErrorResponse::internal_error()
        }
    })?;

    Ok((StatusCode::CREATED, Json(skill)))
}

/// POST /v1/skills/upload - Create skill from ZIP archive
#[utoipa::path(
    post,
    path = "/v1/skills/upload",
    responses(
        (status = 201, description = "Skill created from archive", body = Skill),
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
) -> Result<(StatusCode, Json<Skill>), (StatusCode, Json<ErrorResponse>)> {
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

    let skill = state
        .service
        .create_from_archive(org.org_id, data)
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

    Ok((StatusCode::CREATED, Json(skill)))
}

/// GET /v1/skills - List all skills
#[utoipa::path(
    get,
    path = "/v1/skills",
    responses(
        (status = 200, description = "List of skills", body = ListResponse<Skill>),
    ),
    tag = "skills"
)]
pub async fn list_skills(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> Result<Json<ListResponse<Skill>>, StatusCode> {
    let skills = state
        .service
        .list(org.org_id)
        .await
        .log_internal_error("list skills")?;

    Ok(Json(ListResponse::new(skills)))
}

/// GET /v1/skills/{skill_id} - Get skill by ID
#[utoipa::path(
    get,
    path = "/v1/skills/{skill_id}",
    params(
        ("skill_id" = String, Path, description = "Skill ID (prefixed, e.g., skill_...)")
    ),
    responses(
        (status = 200, description = "Skill found", body = Skill),
        (status = 404, description = "Skill not found"),
    ),
    tag = "skills"
)]
pub async fn get_skill(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(skill_id): Path<String>,
) -> Result<Json<Skill>, (StatusCode, Json<ErrorResponse>)> {
    let skill_id: SkillId = skill_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid skill ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;

    let skill = state
        .service
        .get(org.org_id, skill_id.uuid())
        .await
        .log_internal_error_json("get skill")?
        .ok_or_not_found_json("Skill")?;

    Ok(Json(skill))
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
) -> Result<Json<SkillContent>, (StatusCode, Json<ErrorResponse>)> {
    let skill_id: SkillId = skill_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid skill ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;

    let content = state
        .service
        .get_content(org.org_id, skill_id.uuid())
        .await
        .log_internal_error_json("get skill content")?
        .ok_or_not_found_json("Skill")?;

    Ok(Json(content))
}

/// PATCH /v1/skills/{skill_id} - Update skill
#[utoipa::path(
    patch,
    path = "/v1/skills/{skill_id}",
    request_body = UpdateSkillRequest,
    responses(
        (status = 200, description = "Skill updated", body = Skill),
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
) -> Result<Json<Skill>, (StatusCode, Json<ErrorResponse>)> {
    let skill_id: SkillId = skill_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid skill ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;

    let skill = state
        .service
        .update(org.org_id, skill_id.uuid(), req)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("already exists") {
                ErrorResponse::new(msg).into_response(StatusCode::CONFLICT)
            } else if msg.contains("Invalid SKILL.md") {
                ErrorResponse::new(msg).into_response(StatusCode::UNPROCESSABLE_ENTITY)
            } else {
                tracing::error!("Failed to update skill: {}", e);
                ErrorResponse::internal_error()
            }
        })?
        .ok_or_else(|| ErrorResponse::not_found("Skill"))?;

    Ok(Json(skill))
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
    let skill_id: SkillId = skill_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid skill ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;

    let deleted = state
        .service
        .delete(org.org_id, skill_id.uuid())
        .await
        .log_internal_error_json("delete skill")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::not_found("Skill"))
    }
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
