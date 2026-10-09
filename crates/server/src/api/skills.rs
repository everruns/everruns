// Skills registry HTTP routes
// Routes: /v1/skills/...
//
// CRUD for Agent Skills (agentskills.io format).
// Supports both SKILL.md text upload and ZIP archive upload.

use crate::api::command_http::CommandRouterExt;
use crate::api::common::{ErrorResponse, UrlBuilder, WithUrls};
use crate::api::state::ApiState;
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::skills;
use crate::domains::skills::record::Skill;
use crate::domains::skills::{SKILL_DANGEROUS, SKILL_MANAGE, SKILL_VIEW};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
};
use axum_extra::extract::Multipart;
use everruns_core::{
    Caller, ResourceConfigResponse, SkillValidationResult, evaluate_policies_with,
    validate_skill_md,
};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

// ============================================
// Constants
// ============================================

/// Maximum ZIP archive size (10 MB + overhead)
const MAX_ARCHIVE_UPLOAD: usize = 11 * 1024 * 1024;

// ============================================
// Request/Response types
// ============================================

/// Request to validate a SKILL.md
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ValidateSkillRequest {
    /// Full SKILL.md content to validate, including the YAML frontmatter and the markdown body.
    #[schema(
        example = "---\nname: refund-policy\ndescription: Issue refunds inside the policy window.\n---\n\nUse this when a customer asks for a refund within 30 days of purchase."
    )]
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
) -> Result<Json<ResourceConfigResponse>, (StatusCode, Json<ErrorResponse>)> {
    if !org.feature_flags.skills {
        return Err(ErrorResponse::feature_not_enabled("skills"));
    }
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[&SKILL_VIEW, &SKILL_MANAGE, &SKILL_DANGEROUS],
    );
    Ok(Json(ResourceConfigResponse { policies }))
}

// ============================================
// Routes
// ============================================

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/skills/config", get(skill_config))
        .route(
            "/v1/skills/upload",
            post(upload_skill).layer(DefaultBodyLimit::max(MAX_ARCHIVE_UPLOAD)),
        )
        .route("/v1/skills/validate", post(validate_skill))
        .command::<skills::CreateSkill>()
        .command::<skills::ListSkills>()
        .command::<skills::ListSkillsUsage>()
        .command::<skills::GetSkill>()
        .command::<skills::GetSkillContent>()
        .command::<skills::UpdateSkillCmd>()
        .command::<skills::DeleteSkill>()
        .command::<skills::DestroySkill>()
        .with_state(state)
}

// ============================================
// Handlers
// ============================================

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
    State(state): State<ApiState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<WithUrls<Skill>>), (StatusCode, Json<ErrorResponse>)> {
    if !org.feature_flags.skills {
        return Err(ErrorResponse::feature_not_enabled("skills"));
    }
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
    SKILL_MANAGE
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(|e| ErrorResponse::new(e.message).into_response(StatusCode::FORBIDDEN))?;
    // Upload writes storage directly, so it records its history entry itself.
    let change = crate::domains::change_history::rest::RestChange::begin(
        state.db.clone(),
        caller.clone(),
        "upload_skill",
        crate::domains::change_history::EntityKind::Skill,
        crate::domains::change_history::ChangeAction::Created,
        None,
        &["archive"],
    )
    .await?;

    let skill =
        crate::domains::skills::archive::create_from_archive(&state.db, caller.org_id, data)
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

    state
        .capability_service
        .invalidate_skills_cache(caller.org_id)
        .await;
    change.finish(&skill.id.to_string()).await;

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((StatusCode::CREATED, Json(urls.wrap(skill))))
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
    org: ResolvedOrg,
    State(_state): State<ApiState>,
    Json(req): Json<ValidateSkillRequest>,
) -> Result<Json<SkillValidationResult>, (StatusCode, Json<ErrorResponse>)> {
    if !org.feature_flags.skills {
        return Err(ErrorResponse::feature_not_enabled("skills"));
    }
    Ok(Json(validate_skill_md(&req.skill_md)))
}
