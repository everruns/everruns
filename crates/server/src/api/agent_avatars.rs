// Agent avatar routes.
//
//   PUT    /v1/agents/{agent_id}/avatar         upload (multipart `file`), replaces
//   DELETE /v1/agents/{agent_id}/avatar         remove
//   POST   /v1/agents/{agent_id}/avatar/set     `set_agent_avatar` command (JSON)
//   GET    /v1/avatars/{avatar_id}/{variant}    public, immutable PNG
//
// Design decisions:
// - Upload renders every variant up front (`domains::agents::avatar`), so the
//   read route is a keyed lookup and never resizes.
// - Reads are unauthenticated on purpose: Slack, A2A clients and email cannot
//   send our credentials. The avatar id is a fresh random UUID per upload and
//   is only learned from the agent (or the surfaces it publishes to), and the
//   content is the same picture those surfaces already show publicly.
// - `Cache-Control: immutable` is safe because an id is never reused: a new
//   upload gets a new id and the old one stops resolving.
// - The multipart upload stays a plain route: a binary body has no CLI/MCP
//   form. The CLI and MCP set avatars through the `set_agent_avatar` command,
//   which takes a preset id or the image inline as base64 (capped at 1 MB) and
//   shares this file's write path (`domains::agents::avatar_commands`).

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, put},
};
use axum_extra::extract::Multipart;
use everruns_contracts::typed_id::AvatarId;
use everruns_core::Caller;

use super::agents::AppState;
use super::command_http::CommandRouterExt;
use super::common::ErrorResponse;
use crate::auth::ResolvedOrg;
use crate::domains::agents::AGENT_MANAGE;
use crate::domains::agents::avatar::RenderedAvatarVariant;
use crate::domains::agents::avatar::{
    AVATAR_CONTENT_TYPE, MAX_AVATAR_UPLOAD_BYTES, is_known_variant,
};
use crate::domains::agents::avatar_commands::{self as avatars, MAX_INLINE_AVATAR_BYTES};
use crate::domains::agents::avatar_presets::{AvatarPreset, PRESETS, find_preset};
use crate::domains::agents::record::AgentAvatar;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Select a curated avatar by its stable catalog ID.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectAvatarPreset {
    /// Stable ID returned by the public avatar preset catalog.
    #[schema(example = "familiars-patch")]
    pub preset_id: String,
}

/// The curated preset currently assigned to an agent, if any.
#[derive(Serialize, ToSchema)]
pub struct AvatarPresetSelection {
    /// Catalog ID, or null when the agent has an upload or no avatar.
    #[schema(example = "familiars-patch")]
    pub preset_id: Option<String>,
}

use crate::storage::AgentRow;

type ApiError = (StatusCode, Json<ErrorResponse>);

/// Cached for a year and never revalidated: ids are never reused.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/agents/{agent_id}/avatar",
            put(upload_agent_avatar)
                .delete(delete_agent_avatar)
                // Multipart framing on top of the largest accepted image.
                .layer(DefaultBodyLimit::max(MAX_AVATAR_UPLOAD_BYTES + 64 * 1024)),
        )
        .route("/v1/avatars/{avatar_id}/{variant}", get(get_avatar_variant))
        .route("/v1/avatar-presets", get(list_avatar_presets))
        .route(
            "/v1/avatar-presets/{preset_id}/{variant}",
            get(get_avatar_preset_variant),
        )
        .route(
            "/v1/agents/{agent_id}/avatar/preset",
            put(select_avatar_preset).get(get_avatar_preset_selection),
        )
        .merge(
            Router::new()
                .command::<crate::domains::agents::SetAgentAvatar>()
                // Base64 (4/3) of the largest inline image, plus the JSON around it.
                .layer(DefaultBodyLimit::max(
                    MAX_INLINE_AVATAR_BYTES.div_ceil(3) * 4 + 64 * 1024,
                )),
        )
}

fn bad_request(message: impl Into<String>) -> ApiError {
    ErrorResponse::new(message).into_response(StatusCode::BAD_REQUEST)
}

/// The agent the caller may manage, or the error to return.
/// Avatar routes write storage directly, so they record the agent's history
/// entry themselves (`change_history::rest`).
async fn avatar_change(
    state: &AppState,
    org: &ResolvedOrg,
    agent: &AgentRow,
    operation: &'static str,
) -> Result<crate::domains::change_history::rest::RestChange, ApiError> {
    use crate::domains::change_history::{ChangeAction, EntityKind, rest::RestChange};
    let (db, caller) = (state.db.clone(), Caller::from(org));
    let (kind, action) = (EntityKind::Agent, ChangeAction::Updated);
    let entity_ref = Some(agent.public_id.as_str());
    Ok(RestChange::begin(db, caller, operation, kind, action, entity_ref, &["avatar"]).await?)
}

async fn manageable_agent(
    state: &AppState,
    org: &ResolvedOrg,
    agent_id: &str,
) -> Result<AgentRow, ApiError> {
    AGENT_MANAGE
        .evaluate_with(state.auth.permission_resolver.as_ref(), &Caller::from(org))
        .map_err(|_| {
            ErrorResponse::new("Permission denied").into_response(StatusCode::FORBIDDEN)
        })?;
    Ok(avatars::manageable_agent(&state.db, org.org_id, agent_id).await?)
}

/// PUT /v1/agents/{agent_id}/avatar - Upload the agent's avatar
#[utoipa::path(
    put,
    path = "/v1/agents/{agent_id}/avatar",
    description = "Upload an avatar for an agent as multipart field `file` (PNG, JPEG, GIF or WebP, at least 64x64, at most 10 MB). The image is center-cropped to a square and pre-rendered as square and circular PNG presets. Replaces any previous avatar, whose URLs stop resolving. Also sets the icon of the agent's one-click Slack apps.",
    params(("agent_id" = String, Path, description = "Agent ID")),
    request_body(content_type = "multipart/form-data", description = "Multipart form with a `file` field"),
    responses(
        (status = 200, description = "Avatar stored", body = AgentAvatar),
        (status = 400, description = "Missing or invalid image", body = ErrorResponse),
        (status = 403, description = "Not allowed to manage this agent", body = ErrorResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn upload_agent_avatar(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<AgentAvatar>, ApiError> {
    let agent = manageable_agent(&state, &org, &agent_id).await?;

    let mut upload = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| bad_request(format!("Invalid multipart body: {error}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let content_type = field.content_type().unwrap_or_default().to_string();
        let data = field
            .bytes()
            .await
            .map_err(|error| bad_request(format!("Failed to read file: {error}")))?;
        upload = Some((content_type, data));
        break;
    }
    let (content_type, data) = upload.ok_or_else(|| bad_request("No 'file' field in request"))?;

    let variants = avatars::render_upload(data, content_type).await?;
    store_avatar(&state, &org, &agent, "upload".to_string(), variants).await
}

async fn store_avatar(
    state: &AppState,
    org: &ResolvedOrg,
    agent: &AgentRow,
    source: String,
    variants: Vec<RenderedAvatarVariant>,
) -> Result<Json<AgentAvatar>, ApiError> {
    let change = avatar_change(state, org, agent, "set_agent_avatar").await?;
    let avatar = avatars::store_avatar(
        &state.db,
        state.slack_provisioner.clone(),
        state.encryption.clone(),
        org.org_id,
        agent,
        source,
        variants,
    )
    .await?;
    change.finish(&agent.public_id).await;
    Ok(Json(avatar))
}

/// The public catalog contains presentation metadata only, never agent configuration.
#[utoipa::path(get, path = "/v1/avatar-presets", responses((status = 200, body = Vec<AvatarPreset>)), security(()), tag = "agents")]
pub async fn list_avatar_presets() -> Json<Vec<AvatarPreset>> {
    Json(PRESETS.iter().map(|p| p.metadata.clone()).collect())
}

/// Retrieve a public PNG preview of a curated preset using the agent avatar sizes and shapes.
#[utoipa::path(get, path = "/v1/avatar-presets/{preset_id}/{variant}",
    params(("preset_id" = String, Path), ("variant" = String, Path)),
    responses((status = 200, content_type = "image/png"), (status = 404)), security(()), tag = "agents")]
pub async fn get_avatar_preset_variant(Path((id, variant)): Path<(String, String)>) -> Response {
    if !is_known_variant(&variant) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(preset) = find_preset(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // One bounded cache entry per compiled preset. CPU work stays off async workers.
    let data = tokio::task::spawn_blocking(move || {
        preset
            .variants()
            .ok()?
            .iter()
            .find(|v| v.variant == variant)
            .map(|v| v.data.clone())
    })
    .await;
    match data {
        Ok(Some(data)) => (
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "public, max-age=3600"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            data,
        )
            .into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Replace an agent's avatar with a curated preset, storing fresh immutable image URLs and updating its Slack app icons.
#[utoipa::path(put, path = "/v1/agents/{agent_id}/avatar/preset",
    params(("agent_id" = String, Path)), request_body = SelectAvatarPreset,
    responses((status = 200, body = AgentAvatar), (status = 400, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)), tag = "agents")]
pub async fn select_avatar_preset(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(request): Json<SelectAvatarPreset>,
) -> Result<Json<AgentAvatar>, ApiError> {
    let agent = manageable_agent(&state, &org, &agent_id).await?;
    let (source, variants) = avatars::preset_variants(&request.preset_id).await?;
    store_avatar(&state, &org, &agent, source, variants).await
}

/// Return the agent's current curated preset ID, or null for a custom upload or no avatar. Requires permission to manage the agent.
#[utoipa::path(get, path = "/v1/agents/{agent_id}/avatar/preset", params(("agent_id" = String, Path)),
    responses((status = 200, body = AvatarPresetSelection), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)), tag = "agents")]
pub async fn get_avatar_preset_selection(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<AvatarPresetSelection>, ApiError> {
    let agent = manageable_agent(&state, &org, &agent_id).await?;
    let source = state
        .db
        .get_agent_avatar_source(org.org_id, agent.id.uuid())
        .await
        .map_err(|_| ErrorResponse::internal_error())?;
    Ok(Json(AvatarPresetSelection {
        preset_id: source.and_then(|s| s.strip_prefix("preset:").map(str::to_owned)),
    }))
}

/// DELETE /v1/agents/{agent_id}/avatar - Remove the agent's avatar
#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/avatar",
    params(("agent_id" = String, Path, description = "Agent ID")),
    responses(
        (status = 204, description = "Avatar removed (or there was none)"),
        (status = 403, description = "Not allowed to manage this agent", body = ErrorResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn delete_agent_avatar(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let agent = manageable_agent(&state, &org, &agent_id).await?;
    let change = avatar_change(&state, &org, &agent, "delete_agent_avatar").await?;
    state
        .db
        .clear_agent_avatar(org.org_id, agent.id.uuid())
        .await
        .map_err(|error| {
            tracing::error!(%error, "Failed to remove agent avatar");
            ErrorResponse::internal_error()
        })?;
    change.finish(&agent.public_id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /v1/avatars/{avatar_id}/{variant} - Avatar image
#[utoipa::path(
    get,
    path = "/v1/avatars/{avatar_id}/{variant}",
    description = "Public, immutable avatar image. `variant` is `square-{size}.png` or `circle-{size}.png` for a size in the avatar's `sizes`, or `source.png` for the uploaded square crop.",
    params(
        ("avatar_id" = String, Path, description = "Avatar ID (avatar_...)"),
        ("variant" = String, Path, description = "e.g. square-128.png, circle-64.png, source.png")
    ),
    responses(
        (status = 200, description = "PNG image", content_type = "image/png"),
        (status = 404, description = "Unknown avatar or variant")
    ),
    security(()),
    tag = "agents"
)]
pub async fn get_avatar_variant(
    State(state): State<AppState>,
    Path((avatar_id, variant)): Path<(String, String)>,
) -> Response {
    let not_found = || StatusCode::NOT_FOUND.into_response();
    let Ok(avatar_id) = avatar_id.parse::<AvatarId>() else {
        return not_found();
    };
    if !is_known_variant(&variant) {
        return not_found();
    }
    let row = match state
        .db
        .get_agent_avatar_variant(avatar_id.uuid(), &variant)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(),
        Err(error) => {
            tracing::error!(%error, "Failed to read agent avatar");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let content_type = HeaderValue::from_str(&row.content_type)
        .unwrap_or_else(|_| HeaderValue::from_static(AVATAR_CONTENT_TYPE));
    let etag = format!("\"{}-{}\"", avatar_id, variant);
    let mut response = Response::new(Body::from(row.data));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(IMMUTABLE));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // Embeddable anywhere: chat surfaces and A2A clients render it cross-origin.
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("cross-origin"),
    );
    if let Ok(etag) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, etag);
    }
    response
}
