// Common DTOs for public API
//
// These types are shared across multiple API endpoints.
// ApiResult: standard return type for API handlers
// impl_auth_state!: macro to eliminate repeated FromRef<AppState> for AuthState impls

use axum::Json;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use everruns_core::typed_id::SessionId;
use everruns_durable::UpdateField;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{DeserializeOwned, Error as DeError},
};
use serde_json::Value;
use std::sync::Arc;
use utoipa::ToSchema;

use crate::storage::StorageBackend;

const LINK_DECORATION_MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

/// Standard return type for API handlers that return JSON with error responses.
///
/// Replaces the repeated pattern:
/// ```ignore
/// Result<Json<T>, (StatusCode, Json<ErrorResponse>)>
/// ```
pub type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

/// Response body for resource stats.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = ResourceStats)]
pub struct ResourceStatsResponse {
    /// Total number of sessions ever created for this resource.
    #[schema(example = 1_842u64)]
    pub session_count: u64,
    /// Sessions in a running state right now.
    #[schema(example = 7u64)]
    pub active_session_count: u64,
    /// Sessions in an idle state right now (created but no recent activity).
    #[schema(example = 23u64)]
    pub idle_session_count: u64,
    /// Sessions that have started at least one turn.
    #[schema(example = 1_810u64)]
    pub started_session_count: u64,
    /// Sessions currently paused waiting for client-side tool results.
    #[schema(example = 2u64)]
    pub waiting_for_tool_results_session_count: u64,
    /// Total number of agent executions (turns) recorded across all sessions for this resource.
    #[schema(example = 14_206u64)]
    pub execution_count: u64,
    /// Cumulative session wall-clock duration in milliseconds across all sessions.
    #[schema(example = 38_421_500u64)]
    pub total_session_duration_ms: u64,
    /// Average session duration in milliseconds. `None` when `session_count` is 0.
    #[schema(example = 20_858u64)]
    pub avg_session_duration_ms: Option<u64>,
    /// Cumulative LLM input tokens billed across all sessions.
    #[schema(example = 18_457_321u64)]
    pub total_input_tokens: u64,
    /// Cumulative LLM output tokens billed across all sessions.
    #[schema(example = 2_184_109u64)]
    pub total_output_tokens: u64,
    /// Cumulative cache-read tokens across all sessions (where the provider supports prompt caching).
    #[schema(example = 5_234_122u64)]
    pub total_cache_read_tokens: u64,
    /// Cumulative cache-write tokens across all sessions (charged when first writing a cache entry).
    #[schema(example = 412_005u64)]
    pub total_cache_creation_tokens: u64,
    /// Cumulative provider-reported actual cost in USD across all sessions
    /// (e.g. OpenRouter's `usage.cost`). Excludes generations with no reported cost.
    #[schema(example = 31.07)]
    pub total_actual_cost_usd: f64,
    /// Cumulative price-table estimated cost in USD across all sessions.
    /// Excludes generations with no profile cost data.
    #[schema(example = 40.55)]
    pub total_estimated_cost_usd: f64,
    /// Cumulative best-effort cost in USD across all sessions: actual provider
    /// cost where reported, otherwise the price-table estimate. Actual takes priority.
    #[schema(example = 42.18)]
    pub total_cost_usd: f64,
    /// Timestamp of the first session for this resource (RFC 3339). `None` if no sessions exist.
    #[schema(example = "2026-01-04T11:23:00Z")]
    pub first_session_at: Option<DateTime<Utc>>,
    /// Timestamp of the most recent session for this resource (RFC 3339). `None` if no sessions exist.
    #[schema(example = "2026-05-27T15:24:00Z")]
    pub last_session_at: Option<DateTime<Utc>>,
    /// Timestamp of the most recent execution (turn) for this resource (RFC 3339). `None` if no executions exist.
    #[schema(example = "2026-05-27T15:24:42Z")]
    pub last_execution_at: Option<DateTime<Utc>>,
}

impl From<crate::storage::SessionAggregateStatsRow> for ResourceStatsResponse {
    fn from(row: crate::storage::SessionAggregateStatsRow) -> Self {
        let session_count = row.session_count.max(0) as u64;
        Self {
            session_count,
            active_session_count: row.active_session_count.max(0) as u64,
            idle_session_count: row.idle_session_count.max(0) as u64,
            started_session_count: row.started_session_count.max(0) as u64,
            waiting_for_tool_results_session_count: row
                .waiting_for_tool_results_session_count
                .max(0) as u64,
            execution_count: row.execution_count.max(0) as u64,
            total_session_duration_ms: row.total_session_duration_ms.max(0) as u64,
            avg_session_duration_ms: (session_count > 0)
                .then(|| row.total_session_duration_ms.max(0) as u64 / session_count),
            total_input_tokens: row.total_input_tokens.max(0) as u64,
            total_output_tokens: row.total_output_tokens.max(0) as u64,
            total_cache_read_tokens: row.total_cache_read_tokens.max(0) as u64,
            total_cache_creation_tokens: row.total_cache_creation_tokens.max(0) as u64,
            total_actual_cost_usd: row.total_actual_cost_usd.max(0.0),
            total_estimated_cost_usd: row.total_estimated_cost_usd.max(0.0),
            total_cost_usd: row.total_cost_usd.max(0.0),
            first_session_at: row.first_session_at,
            last_session_at: row.last_session_at,
            last_execution_at: row.last_execution_at,
        }
    }
}

/// Implement `FromRef<$state> for AuthState` for an API module's state struct.
///
/// Every API module with auth repeats the same 5-line impl. This macro eliminates
/// that boilerplate. The state struct must have a field `auth: AuthState`.
///
/// Usage:
/// ```ignore
/// impl_auth_state!(AppState);
/// impl_auth_state!(UsersState);
/// ```
macro_rules! impl_auth_state {
    ($state:ty) => {
        impl ::axum::extract::FromRef<$state> for crate::auth::AuthState {
            fn from_ref(input: &$state) -> Self {
                input.auth.clone()
            }
        }
    };
}
pub(crate) use impl_auth_state;

/// Standard error response.
///
/// Wire shape is [RFC 9457 Problem Details](https://www.rfc-editor.org/rfc/rfc9457):
/// every error response includes `title` and `status`, and may include
/// `detail`, `code`, `allowed_actions`, `retry_after_seconds`, `instance`,
/// and `type`. The content type is rewritten to `application/problem+json`
/// by [`problem_json_content_type`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ErrorResponse {
    /// RFC 9457 problem type URI. Optional; identifies the problem class.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://docs.everruns.com/errors/session_not_found")]
    pub type_uri: Option<String>,
    /// Short, human-readable summary of the problem (e.g. "Not Found").
    #[schema(example = "Session not found")]
    pub title: String,
    /// HTTP status code; mirrors the response status line.
    #[schema(example = 404)]
    pub status: u16,
    /// Human-readable explanation specific to this occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(
        example = "Session session_01933b5a000070008000000000000001 not found in org org_01933b5a000070008000000000000001."
    )]
    pub detail: Option<String>,
    /// Request URI for this occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "/v1/sessions/session_01933b5a000070008000000000000001")]
    pub instance: Option<String>,
    /// Stable, machine-readable error code (snake_case).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "session_not_found")]
    pub code: Option<String>,
    /// Recovery actions the caller can take next.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_actions: Vec<AllowedAction>,
    /// Seconds the caller should wait before retrying (429 / transient 503).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 30)]
    pub retry_after_seconds: Option<u32>,
}

/// Agent-actionable link describing a follow-up the caller can take. Used in
/// two contexts:
///
/// * **Error recovery** — `ErrorResponse.allowed_actions` carries `rel`s like
///   `retry`, `retry-later`, `unarchive`, `get-existing` so the agent knows
///   the right next call after a 4xx/429.
/// * **Entity hypermedia** — `WithUrls<T>.allowed_actions` carries state-aware
///   `rel`s like `cancel`, `events`, `self`, `update` on the entity itself
///   so the agent can follow links instead of reconstructing routes from
///   prose.
///
/// The shape is intentionally identical across both contexts; the closed
/// `rel` vocabulary documented in `knowledge/execution/api-conventions.md` distinguishes
/// them.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct AllowedAction {
    /// Link relation describing the action. Closed vocabulary documented
    /// in `knowledge/execution/api-conventions.md` — examples: `self`, `cancel`, `pause`,
    /// `resume`, `events`, `retry`, `retry-later`, `unarchive`,
    /// `get-existing`, `delete`, `update`.
    pub rel: String,
    /// OpenAPI `operationId` the caller should invoke. Lets an MCP client
    /// resolve the call without parsing `href`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    /// HTTP method to use against `href`. Required for entity hypermedia
    /// actions; usually omitted on error-recovery actions where the same
    /// operation is retried with its original method.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "POST")]
    pub method: Option<String>,
    /// Short, agent-readable hint (e.g. "Shorten 'name' to <= 200 chars.",
    /// "Cancel the active turn for this session.").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Absolute (preferred) or relative URL the caller may invoke
    /// directly. **Always present on entity hypermedia actions**
    /// (`WithUrls<T>.allowed_actions`); **optional on error-recovery
    /// actions** (`ErrorResponse.allowed_actions`) where the matching
    /// `operation_id` is enough and the URI is implicit from the failed
    /// call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    /// OpenAPI `$ref` to the request-body schema, when the action takes one
    /// (e.g. `#/components/schemas/UpdateSessionRequest`). Lets a tool-calling
    /// agent fetch the input shape without scanning the whole spec.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_ref: Option<String>,
}

impl AllowedAction {
    pub fn new(rel: impl Into<String>) -> Self {
        Self {
            rel: rel.into(),
            ..Self::default()
        }
    }

    pub fn with_operation_id(mut self, operation_id: impl Into<String>) -> Self {
        self.operation_id = Some(operation_id.into());
        self
    }

    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_href(mut self, href: impl Into<String>) -> Self {
        self.href = Some(href.into());
        self
    }

    pub fn with_schema_ref(mut self, schema_ref: impl Into<String>) -> Self {
        self.schema_ref = Some(schema_ref.into());
        self
    }
}

impl ErrorResponse {
    /// Construct an error whose `detail` is the supplied message.
    ///
    /// `title` is filled in from the HTTP reason phrase when this is later
    /// passed through [`into_response`](Self::into_response). For full control,
    /// build the struct directly or chain `.with_title(...)`.
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: Some(detail.into()),
            ..Self::default()
        }
    }

    /// Convert to axum response tuple, filling in `status` and (if missing)
    /// `title` from the HTTP reason phrase.
    pub fn into_response(mut self, status: StatusCode) -> (StatusCode, Json<Self>) {
        self.status = status.as_u16();
        if self.title.is_empty() {
            self.title = status.canonical_reason().unwrap_or("Error").to_string();
        }
        (status, Json(self))
    }

    /// Create an internal server error response.
    pub fn internal_error() -> (StatusCode, Json<Self>) {
        Self {
            detail: Some("Internal server error".to_string()),
            code: Some("internal_error".to_string()),
            ..Self::default()
        }
        .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    }

    /// Create a not found error response.
    pub fn not_found(resource: &str) -> (StatusCode, Json<Self>) {
        Self {
            detail: Some(format!("{} not found", resource)),
            code: Some("not_found".to_string()),
            ..Self::default()
        }
        .into_response(StatusCode::NOT_FOUND)
    }

    /// Hide a disabled feature's surface while explaining why a direct call failed.
    pub fn feature_not_enabled(flag: &str) -> (StatusCode, Json<Self>) {
        Self {
            detail: Some(format!("Feature '{flag}' is not enabled")),
            code: Some("feature_not_enabled".to_string()),
            ..Self::default()
        }
        .into_response(StatusCode::NOT_FOUND)
    }

    /// Create a conflict error response (409).
    pub fn conflict(message: &str) -> (StatusCode, Json<Self>) {
        Self {
            detail: Some(message.to_string()),
            code: Some("conflict".to_string()),
            ..Self::default()
        }
        .into_response(StatusCode::CONFLICT)
    }

    /// Create a bad gateway error response (502).
    pub fn bad_gateway() -> (StatusCode, Json<Self>) {
        Self {
            detail: Some("Bad gateway".to_string()),
            code: Some("bad_gateway".to_string()),
            ..Self::default()
        }
        .into_response(StatusCode::BAD_GATEWAY)
    }

    // ---- Builders ---------------------------------------------------------

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_type(mut self, type_uri: impl Into<String>) -> Self {
        self.type_uri = Some(type_uri.into());
        self
    }

    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    pub fn with_retry_after(mut self, seconds: u32) -> Self {
        self.retry_after_seconds = Some(seconds);
        self
    }

    pub fn with_action(mut self, action: AllowedAction) -> Self {
        self.allowed_actions.push(action);
        self
    }
}

/// Log an internal error and return a generic 500 tuple `(StatusCode, String)`.
///
/// Use this in handlers that return `Result<T, (StatusCode, String)>` to avoid
/// leaking internal error details to clients.
pub fn sanitized_internal_error(
    context: &str,
    error: &dyn std::fmt::Display,
) -> (StatusCode, String) {
    tracing::error!("{context}: {error}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
    )
}

/// Log an external-service error and return a generic 502 tuple `(StatusCode, String)`.
pub fn sanitized_bad_gateway(context: &str, error: &dyn std::fmt::Display) -> (StatusCode, String) {
    tracing::error!("{context}: {error}");
    (StatusCode::BAD_GATEWAY, "Bad gateway".to_string())
}

fn classify_anyhow_error(message: &str) -> Option<(StatusCode, Json<ErrorResponse>)> {
    let lowered = message.to_ascii_lowercase();

    if lowered.contains("duplicate key") || lowered.contains("already exists") {
        return Some(ErrorResponse::conflict(message));
    }

    let is_bad_request = [
        "invalid base url",
        "provider type cannot",
        "cannot be assigned",
        "cannot be edited",
        "must be archived before deletion",
        "cannot delete built-in",
        "cannot modify built-in",
        "cannot publish archived",
        "cannot unpublish archived",
        "cannot update archived",
        "cannot archive built-in",
        "invalid mcp capability reference",
        "invalid skill capability reference",
        "unsupported locale",
        "unsupported timezone",
    ]
    .iter()
    .any(|pattern| lowered.contains(pattern));

    if is_bad_request {
        return Some(ErrorResponse::new(message).into_response(StatusCode::BAD_REQUEST));
    }

    None
}

// ============================================================================
// Error handling extension traits
// ============================================================================

/// Extension trait for anyhow::Result to handle PolicyError as 403.
///
/// If the error is a `PolicyError`, returns 403 Forbidden.
/// Otherwise logs and returns 500 Internal Server Error.
pub trait ApiPolicyResultExt<T> {
    fn map_policy_or_internal(
        self,
        operation: &str,
    ) -> Result<T, (StatusCode, Json<ErrorResponse>)>;
}

impl<T> ApiPolicyResultExt<T> for Result<T, anyhow::Error> {
    fn map_policy_or_internal(
        self,
        operation: &str,
    ) -> Result<T, (StatusCode, Json<ErrorResponse>)> {
        self.map_err(|e| {
            if let Some(not_found) = e.downcast_ref::<crate::errors::ResourceNotFoundError>() {
                ErrorResponse::not_found(not_found.resource())
            } else if let Some(bad_request) = e.downcast_ref::<crate::errors::BadRequestError>() {
                ErrorResponse::new(bad_request.message()).into_response(StatusCode::BAD_REQUEST)
            } else if let Some(limit) = e.downcast_ref::<crate::errors::ResourceLimitError>() {
                ErrorResponse::conflict(limit.message())
            } else if let Some(policy_err) = e.downcast_ref::<everruns_core::PolicyError>() {
                ErrorResponse::new(&policy_err.message).into_response(StatusCode::FORBIDDEN)
            } else if let Some(response) = classify_anyhow_error(&e.to_string()) {
                response
            } else {
                tracing::error!("Failed to {}: {}", operation, e);
                ErrorResponse::internal_error()
            }
        })
    }
}

/// Extension trait for Result to simplify API error handling.
///
/// Provides methods to log errors and convert to appropriate HTTP responses.
///
/// # Example
///
/// ```ignore
/// use crate::api::common::ApiResultExt;
///
/// // Before:
/// let agent = state.service.get(id).await.map_err(|e| {
///     tracing::error!("Failed to get agent: {}", e);
///     StatusCode::INTERNAL_SERVER_ERROR
/// })?;
///
/// // After:
/// let agent = state.service.get(id).await.log_internal_error("get agent")?;
/// ```
pub trait ApiResultExt<T> {
    /// Log the error and convert to internal server error (StatusCode only).
    ///
    /// Use this for endpoints that return `Result<T, StatusCode>`.
    fn log_internal_error(self, operation: &str) -> Result<T, StatusCode>;

    /// Log the error and convert to internal server error with JSON body.
    ///
    /// Use this for endpoints that return `Result<T, (StatusCode, Json<ErrorResponse>)>`.
    fn log_internal_error_json(
        self,
        operation: &str,
    ) -> Result<T, (StatusCode, Json<ErrorResponse>)>;
}

impl<T, E: std::fmt::Display> ApiResultExt<T> for Result<T, E> {
    fn log_internal_error(self, operation: &str) -> Result<T, StatusCode> {
        self.map_err(|e| {
            tracing::error!("Failed to {}: {}", operation, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
    }

    fn log_internal_error_json(
        self,
        operation: &str,
    ) -> Result<T, (StatusCode, Json<ErrorResponse>)> {
        self.map_err(|e| {
            tracing::error!("Failed to {}: {}", operation, e);
            ErrorResponse::internal_error()
        })
    }
}

/// Extension trait for Option to convert to not found errors.
///
/// # Example
///
/// ```ignore
/// use crate::api::common::ApiOptionExt;
///
/// // Before:
/// let agent = result.ok_or(StatusCode::NOT_FOUND)?;
///
/// // After:
/// let agent = result.ok_or_not_found()?;
/// ```
pub trait ApiOptionExt<T> {
    /// Convert None to NOT_FOUND status code.
    fn ok_or_not_found(self) -> Result<T, StatusCode>;

    /// Convert None to NOT_FOUND with JSON error response.
    fn ok_or_not_found_json(self, resource: &str) -> Result<T, (StatusCode, Json<ErrorResponse>)>;
}

impl<T> ApiOptionExt<T> for Option<T> {
    fn ok_or_not_found(self) -> Result<T, StatusCode> {
        self.ok_or(StatusCode::NOT_FOUND)
    }

    fn ok_or_not_found_json(self, resource: &str) -> Result<T, (StatusCode, Json<ErrorResponse>)> {
        self.ok_or_else(|| ErrorResponse::not_found(resource))
    }
}

/// Deserialize PATCH-style nullable fields into explicit tri-state semantics.
pub fn deserialize_nullable_update_field<'de, D, T>(
    deserializer: D,
) -> Result<UpdateField<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_null() {
        Ok(UpdateField::Clear)
    } else {
        serde_json::from_value(value)
            .map(UpdateField::Set)
            .map_err(D::Error::custom)
    }
}

/// Response wrapper for list endpoints.
/// All list endpoints return responses wrapped in a `data` field.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ListResponse<T> {
    /// Array of items returned by the list operation.
    pub data: Vec<T>,
}

impl<T> ListResponse<T> {
    pub fn new(data: Vec<T>) -> Self {
        Self { data }
    }
}

impl<T> From<Vec<T>> for ListResponse<T> {
    fn from(data: Vec<T>) -> Self {
        Self { data }
    }
}

/// Response wrapper for paginated list endpoints.
/// Includes pagination metadata along with the data array.
///
/// `next_url` and `prev_url` are populated by the [`decorate_pagination_links`]
/// middleware after the handler returns, so handlers don't need to thread the
/// request URL into every construction site.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PaginatedResponse<T> {
    /// Array of items returned by the list operation.
    pub data: Vec<T>,
    /// Total number of items matching the query (across all pages).
    pub total: u32,
    /// Current offset (starting position).
    pub offset: u32,
    /// Maximum number of items per page.
    pub limit: u32,
    /// Absolute URL for the next page, with `offset` advanced by `limit`. Omitted from the response (field absent) when this is the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_url: Option<String>,
    /// Absolute URL for the previous page, with `offset` rolled back by `limit`. Omitted from the response (field absent) when this is the first page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_url: Option<String>,
}

impl<T> PaginatedResponse<T> {
    pub fn new(data: Vec<T>, total: u32, offset: u32, limit: u32) -> Self {
        Self {
            data,
            total,
            offset,
            limit,
            next_url: None,
            prev_url: None,
        }
    }
}

/// Pagination parameters for list endpoints.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pagination {
    pub offset: u32,
    pub limit: u32,
}

impl Pagination {
    pub fn new(offset: u32, limit: u32) -> Self {
        Self { offset, limit }
    }
}

// ============================================================================
// Resource URL enrichment
// ============================================================================

/// Builds absolute `url` (API) and `view_url` (UI) for resources.
#[derive(Debug, Clone)]
pub struct UrlBuilder {
    api_base: String,
    ui_base: String,
}

impl UrlBuilder {
    pub fn new(api_base: &str, ui_base: &str) -> Self {
        Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            ui_base: ui_base.trim_end_matches('/').to_string(),
        }
    }

    /// Create from an `AuthConfig`.
    pub fn from_auth_config(config: &crate::auth::config::AuthConfig) -> Self {
        Self::new(&config.base_url, &config.frontend_url)
    }

    /// Wrap a single resource with API and UI links plus its state-aware
    /// hypermedia actions (see `ResourceUrlable::allowed_actions`).
    pub fn wrap<T: ResourceUrlable + Serialize>(&self, item: T) -> WithUrls<T> {
        let api_path = item.api_url_path();
        let ui_path = item.ui_url_path();
        let view_url = format!("{}/{}", self.ui_base, ui_path);
        let allowed_actions = item.allowed_actions(&self.api_base);
        WithUrls {
            self_url: format!("{}/{}", self.api_base, api_path),
            view_url: view_url.clone(),
            ui_link: view_url,
            allowed_actions,
            inner: item,
        }
    }

    /// Wrap a vec of resources.
    pub fn wrap_vec<T: ResourceUrlable + Serialize>(&self, items: Vec<T>) -> Vec<WithUrls<T>> {
        items.into_iter().map(|item| self.wrap(item)).collect()
    }

    /// Add resource links to any recognizable entity objects in a JSON value.
    ///
    /// This is the protocol-agnostic link aspect used for command/MCP output
    /// and final API responses. It is additive only: existing link fields win.
    pub fn decorate_value_links(&self, value: &mut Value) -> bool {
        decorate_value_links(value, self)
    }
}

/// Middleware that enriches successful JSON API responses with entity links.
///
/// This covers endpoints that return command/domain DTOs directly instead of
/// using `WithUrls`, while preserving non-JSON, streaming, and error responses.
pub async fn decorate_json_response_links(
    builder: UrlBuilder,
    req: Request,
    next: Next,
) -> Response {
    let response = next.run(req).await;
    if !response.status().is_success()
        || !response_is_json(&response)
        || !response_within_link_decoration_limit(&response)
    {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let bytes = match to_bytes(body, LINK_DECORATION_MAX_RESPONSE_BYTES as usize).await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(error = %err, "failed to read JSON response body for link decoration");
            return ErrorResponse::internal_error().into_response();
        }
    };
    if bytes.is_empty() {
        return Response::from_parts(parts, Body::from(bytes));
    }

    let mut value = match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => value,
        Err(_) => return Response::from_parts(parts, Body::from(bytes)),
    };
    if !builder.decorate_value_links(&mut value) {
        return Response::from_parts(parts, Body::from(bytes));
    }

    match serde_json::to_vec(&value) {
        Ok(body) => {
            parts.headers.remove(header::CONTENT_LENGTH);
            Response::from_parts(parts, Body::from(body))
        }
        Err(err) => {
            tracing::warn!(error = %err, "failed to serialize link-decorated JSON response");
            Response::from_parts(parts, Body::from(bytes))
        }
    }
}

/// Middleware that adds `next_url` / `prev_url` fields to JSON responses
/// shaped like [`PaginatedResponse`] (containing `total`, `offset`, `limit`).
///
/// Recomputes the navigation URLs from the request path + query each time, so
/// handlers don't need to know the request URL. The middleware preserves all
/// other query parameters (filters, search, etc.) and only mutates the
/// `offset` value. `next_url` is omitted when the current page is the last
/// page; `prev_url` is omitted when this is the first page.
pub async fn decorate_pagination_links(builder: UrlBuilder, req: Request, next: Next) -> Response {
    let uri_path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());

    let response = next.run(req).await;
    if !response.status().is_success()
        || !response_is_json(&response)
        || !response_within_link_decoration_limit(&response)
    {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let bytes = match to_bytes(body, LINK_DECORATION_MAX_RESPONSE_BYTES as usize).await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(error = %err, "failed to read JSON response body for pagination links");
            return ErrorResponse::internal_error().into_response();
        }
    };
    if bytes.is_empty() {
        return Response::from_parts(parts, Body::from(bytes));
    }

    let mut value = match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => value,
        Err(_) => return Response::from_parts(parts, Body::from(bytes)),
    };

    if !inject_pagination_links(&mut value, &builder.api_base, &uri_path_and_query) {
        return Response::from_parts(parts, Body::from(bytes));
    }

    match serde_json::to_vec(&value) {
        Ok(body) => {
            parts.headers.remove(header::CONTENT_LENGTH);
            Response::from_parts(parts, Body::from(body))
        }
        Err(err) => {
            tracing::warn!(error = %err, "failed to serialize pagination-decorated JSON response");
            Response::from_parts(parts, Body::from(bytes))
        }
    }
}

/// Recursively walk `value` and inject `next_url` / `prev_url` on any object
/// that looks like a paginated response (has `total`, `offset`, and `limit`
/// numeric fields). Returns `true` if at least one object was decorated.
fn inject_pagination_links(value: &mut Value, api_base: &str, path_and_query: &str) -> bool {
    match value {
        Value::Object(map) => {
            let total = map.get("total").and_then(Value::as_u64);
            let offset = map.get("offset").and_then(Value::as_u64);
            let limit = map.get("limit").and_then(Value::as_u64);
            let mut changed = false;
            if let (Some(total), Some(offset), Some(limit)) = (total, offset, limit)
                && limit > 0
            {
                // Decorate each link field independently so a handler that
                // pre-populates one (e.g. cursor-based `next_url`) doesn't
                // block the middleware from filling the other.
                if !map.contains_key("next_url") && offset.saturating_add(limit) < total {
                    let next =
                        build_paginated_url(api_base, path_and_query, offset.saturating_add(limit));
                    map.insert("next_url".to_string(), Value::String(next));
                    changed = true;
                }
                if !map.contains_key("prev_url") && offset > 0 {
                    let prev =
                        build_paginated_url(api_base, path_and_query, offset.saturating_sub(limit));
                    map.insert("prev_url".to_string(), Value::String(prev));
                    changed = true;
                }
            }
            for child in map.values_mut() {
                changed |= inject_pagination_links(child, api_base, path_and_query);
            }
            changed
        }
        Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= inject_pagination_links(item, api_base, path_and_query);
            }
            changed
        }
        _ => false,
    }
}

/// Rebuild an absolute URL by replacing or appending the `offset` query
/// parameter. Other query parameters are preserved verbatim.
fn build_paginated_url(api_base: &str, path_and_query: &str, new_offset: u64) -> String {
    let (path, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_and_query, None),
    };
    let mut rebuilt = String::new();
    let mut found_offset = false;
    if let Some(query) = query {
        for (i, pair) in query.split('&').enumerate() {
            if i > 0 {
                rebuilt.push('&');
            }
            if let Some((k, _v)) = pair.split_once('=')
                && k == "offset"
            {
                rebuilt.push_str("offset=");
                rebuilt.push_str(&new_offset.to_string());
                found_offset = true;
            } else if pair == "offset" {
                rebuilt.push_str("offset=");
                rebuilt.push_str(&new_offset.to_string());
                found_offset = true;
            } else {
                rebuilt.push_str(pair);
            }
        }
    }
    if !found_offset {
        if !rebuilt.is_empty() {
            rebuilt.push('&');
        }
        rebuilt.push_str("offset=");
        rebuilt.push_str(&new_offset.to_string());
    }
    let base = api_base.trim_end_matches('/');
    if rebuilt.is_empty() {
        format!("{base}{path}")
    } else {
        format!("{base}{path}?{rebuilt}")
    }
}

/// Rewrite the `Content-Type` of error responses (4xx / 5xx) carrying a
/// `application/json` body to `application/problem+json`, per RFC 9457.
///
/// Bodies are not touched; only the header is updated. Non-JSON error bodies
/// (HTML, plain text, SSE) pass through unchanged.
pub async fn problem_json_content_type(req: Request, next: Next) -> Response {
    let response = next.run(req).await;
    let status = response.status();
    if !status.is_client_error() && !status.is_server_error() {
        return response;
    }
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
        });
    if !is_json {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    parts.headers.insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/problem+json"),
    );
    Response::from_parts(parts, body)
}

fn response_within_link_decoration_limit(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length <= LINK_DECORATION_MAX_RESPONSE_BYTES)
}

fn response_is_json(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
        })
}

fn decorate_value_links(value: &mut Value, builder: &UrlBuilder) -> bool {
    match value {
        Value::Object(map) => {
            let mut changed = false;
            if !map.contains_key("ui_link")
                && let Some(view_url) = map.get("view_url").and_then(Value::as_str)
            {
                map.insert("ui_link".to_string(), Value::String(view_url.to_string()));
                changed = true;
            }
            if let Some(route) = route_for_object(map) {
                if !map.contains_key("self_url")
                    && let Some(api_path) = route.api_path
                {
                    map.insert(
                        "self_url".to_string(),
                        Value::String(format!("{}/{}", builder.api_base, api_path)),
                    );
                    changed = true;
                }
                if !map.contains_key("view_url") {
                    let view_url = format!("{}/{}", builder.ui_base, route.ui_path);
                    map.insert("view_url".to_string(), Value::String(view_url.clone()));
                    changed = true;
                    if !map.contains_key("ui_link") {
                        map.insert("ui_link".to_string(), Value::String(view_url));
                        changed = true;
                    }
                } else if !map.contains_key("ui_link")
                    && let Some(view_url) = map.get("view_url").and_then(Value::as_str)
                {
                    map.insert("ui_link".to_string(), Value::String(view_url.to_string()));
                    changed = true;
                }
            }

            for child in map.values_mut() {
                changed |= decorate_value_links(child, builder);
            }
            changed
        }
        Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= decorate_value_links(item, builder);
            }
            changed
        }
        _ => false,
    }
}

struct LinkRoute {
    api_path: Option<String>,
    ui_path: String,
}

struct ResourceId {
    value: String,
    include_self_url: bool,
}

fn route_for_object(map: &serde_json::Map<String, Value>) -> Option<LinkRoute> {
    let id = own_resource_id(map)?;
    let mut route = route_for_id(&id.value, map)?;
    if !id.include_self_url {
        route.api_path = None;
    }
    Some(route)
}

fn own_resource_id(map: &serde_json::Map<String, Value>) -> Option<ResourceId> {
    for key in ["id", "public_id"] {
        if let Some(id) = map.get(key).and_then(Value::as_str)
            && route_for_id(id, map).is_some()
        {
            return Some(ResourceId {
                value: id.to_string(),
                include_self_url: true,
            });
        }
    }

    for key in [
        "session_id",
        "agent_id",
        "harness_id",
        "app_id",
        "identity_id",
        "mcp_server_id",
        "skill_id",
        "provider_id",
        "model_id",
        "eval_id",
        "budget_id",
    ] {
        if let Some(id) = map.get(key).and_then(Value::as_str)
            && route_for_id(id, map).is_some()
        {
            return Some(ResourceId {
                value: id.to_string(),
                include_self_url: false,
            });
        }
    }

    None
}

fn route_for_id(id: &str, map: &serde_json::Map<String, Value>) -> Option<LinkRoute> {
    let Some((prefix, _)) = id.split_once('_') else {
        return looks_like_capability(map).then(|| LinkRoute {
            api_path: Some(format!("v1/capabilities/{id}")),
            ui_path: format!("capabilities/{id}"),
        });
    };
    let route = match prefix {
        "agent" => ("v1/agents", format!("agents/{id}")),
        "harness" => ("v1/harnesses", format!("harnesses/{id}")),
        "session" => ("v1/sessions", format!("sessions/{id}/chat")),
        "app" => ("v1/apps", format!("apps/{id}")),
        "identity" => ("v1/agent-identities", format!("agent-identities/{id}")),
        "mcp" => ("v1/mcp-servers", "mcp-servers".to_string()),
        "skill" => ("v1/skills", "skills".to_string()),
        "provider" => ("v1/providers", "settings/providers".to_string()),
        "model" => ("v1/models", "models".to_string()),
        "eval" => ("v1/evals", format!("evals/{id}")),
        "bdgt" => ("v1/budgets", "budgets".to_string()),
        "sched" => {
            let session_id = map.get("session_id").and_then(Value::as_str)?;
            return Some(LinkRoute {
                api_path: Some(format!("v1/sessions/{session_id}/schedules/{id}")),
                ui_path: format!("sessions/{session_id}/schedules"),
            });
        }
        _ if looks_like_capability(map) => {
            return Some(LinkRoute {
                api_path: Some(format!("v1/capabilities/{id}")),
                ui_path: format!("capabilities/{id}"),
            });
        }
        _ => return None,
    };
    Some(LinkRoute {
        api_path: Some(format!("{}/{id}", route.0)),
        ui_path: route.1,
    })
}

fn looks_like_capability(map: &serde_json::Map<String, Value>) -> bool {
    map.contains_key("tool_definitions")
        || map.contains_key("tool_count")
        || map.contains_key("dependencies")
        || map.contains_key("config_schema")
        || map
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|value| matches!(value, "builtin" | "mcp_server" | "skill"))
        || map.contains_key("is_mcp")
        || map.contains_key("is_skill")
        || map.contains_key("is_guardrail")
}

/// Trait for resources that can have `url` and `view_url` generated.
pub trait ResourceUrlable {
    /// API path segment (e.g. `"v1/agents"`).
    fn api_path() -> &'static str;
    /// UI path segment (e.g. `"agents"`).
    fn ui_path() -> &'static str;
    /// The resource's public ID as a string.
    fn resource_id(&self) -> String;
    /// API path for this concrete resource, including its ID.
    fn api_url_path(&self) -> String {
        format!("{}/{}", Self::api_path(), self.resource_id())
    }
    /// UI path for this concrete resource.
    fn ui_url_path(&self) -> String {
        format!("{}/{}", Self::ui_path(), self.resource_id())
    }
    /// State-aware hypermedia actions for this resource. Overridden per-type
    /// to map (entity state) → (next-action links); the default is empty so
    /// existing types stay wire-compatible until they opt in. `api_base`
    /// is the absolute API base URL from `AuthConfig.base_url` —
    /// **without the `/v1` resource prefix** (e.g.
    /// `https://app.everruns.com/api`, not `…/api/v1`). The resolver is
    /// responsible for the versioned path. See
    /// `knowledge/execution/api-conventions.md` for the closed `rel` vocabulary.
    fn allowed_actions(&self, _api_base: &str) -> Vec<AllowedAction> {
        Vec::new()
    }
}

/// Wrapper that adds API and UI links to a serialized resource.
///
/// Uses `self_url` (not `url`) for the API link to avoid collision with
/// resources that already have a `url` field (e.g. McpServer). The
/// `allowed_actions` array carries state-aware hypermedia links — empty
/// (and omitted from the wire shape) until the underlying resource opts
/// into the convention by overriding `ResourceUrlable::allowed_actions`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WithUrls<T: Serialize> {
    /// Full API endpoint URL for this resource.
    pub self_url: String,
    /// Full UI URL for viewing this resource.
    pub view_url: String,
    /// Alias for `view_url`, used by command and MCP outputs.
    pub ui_link: String,
    /// State-aware hypermedia actions the caller can take on this resource
    /// next (e.g. `cancel`, `events`, `update`). Omitted from the wire
    /// shape when empty so resources that haven't opted into the
    /// convention don't grow their payloads.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub allowed_actions: Vec<AllowedAction>,
    /// The resource itself (fields are flattened into the parent object).
    #[serde(flatten)]
    pub inner: T,
}

/// Wrapper that flattens lightweight relationship counts into resource responses.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ResourceWithCounts<T: Serialize> {
    /// Number of sessions using this resource.
    pub session_count: u64,
    /// Number of non-deleted apps using this resource.
    pub app_count: u64,
    /// The resource itself (fields are flattened into the parent object).
    #[serde(flatten)]
    pub inner: T,
}

impl<T: ResourceUrlable + Serialize> ResourceUrlable for ResourceWithCounts<T> {
    fn api_path() -> &'static str {
        T::api_path()
    }

    fn ui_path() -> &'static str {
        T::ui_path()
    }

    fn resource_id(&self) -> String {
        self.inner.resource_id()
    }

    fn allowed_actions(&self, api_base: &str) -> Vec<AllowedAction> {
        self.inner.allowed_actions(api_base)
    }
}

impl<T: ResourceUrlable + Serialize> PaginatedResponse<T> {
    /// Map all items through `UrlBuilder::wrap`.
    pub fn with_urls(self, builder: &UrlBuilder) -> PaginatedResponse<WithUrls<T>> {
        PaginatedResponse {
            data: builder.wrap_vec(self.data),
            total: self.total,
            offset: self.offset,
            limit: self.limit,
            next_url: self.next_url,
            prev_url: self.prev_url,
        }
    }
}

impl<T: ResourceUrlable + Serialize> ListResponse<T> {
    /// Map all items through `UrlBuilder::wrap`.
    pub fn with_urls(self, builder: &UrlBuilder) -> ListResponse<WithUrls<T>> {
        ListResponse {
            data: builder.wrap_vec(self.data),
        }
    }
}

// ── ResourceUrlable implementations ──────────────────────────────────────────

// ---------------------------------------------------------------
// Hypermedia rollout — EVE-494.
//
// One pure resolver per cluster, kept here so the (state → rel set)
// mapping is testable without constructing a full entity. The
// `ResourceUrlable` impl delegates to the resolver. New `rel`s must
// also be documented in `knowledge/execution/api-conventions.md`.
// ---------------------------------------------------------------

/// Hypermedia actions for an `Agent`. See `knowledge/execution/api-conventions.md`.
pub fn agent_allowed_actions(
    id: &str,
    status: &everruns_platform::AgentStatus,
    api_base: &str,
) -> Vec<AllowedAction> {
    use everruns_platform::AgentStatus;
    let mut actions = vec![
        AllowedAction::new("self")
            .with_method("GET")
            .with_operation_id("get_agent")
            .with_href(format!("{api_base}/v1/agents/{id}"))
            .with_hint("Fetch the latest representation of this agent."),
        AllowedAction::new("update")
            .with_method("PATCH")
            .with_operation_id("update_agent")
            .with_href(format!("{api_base}/v1/agents/{id}"))
            .with_schema_ref("#/components/schemas/UpdateAgentRequest")
            .with_hint("Edit agent metadata (name, system prompt, capabilities, etc.)."),
        AllowedAction::new("versions")
            .with_method("GET")
            .with_operation_id("list_agent_versions")
            .with_href(format!("{api_base}/v1/agents/{id}/versions"))
            .with_hint("List the immutable version history for this agent."),
    ];
    if matches!(status, AgentStatus::Active) {
        actions.push(
            AllowedAction::new("copy")
                .with_method("POST")
                .with_operation_id("copy_agent")
                .with_href(format!("{api_base}/v1/agents/{id}/copy"))
                .with_hint("Fork this agent into a new editable copy."),
        );
    }
    actions.push(
        AllowedAction::new("delete")
            .with_method("DELETE")
            .with_operation_id("delete_agent")
            .with_href(format!("{api_base}/v1/agents/{id}"))
            .with_hint("Delete (or tombstone) this agent."),
    );
    actions
}

impl ResourceUrlable for everruns_platform::Agent {
    fn api_path() -> &'static str {
        "v1/agents"
    }
    fn ui_path() -> &'static str {
        "agents"
    }
    fn resource_id(&self) -> String {
        self.public_id.to_string()
    }
    fn allowed_actions(&self, api_base: &str) -> Vec<AllowedAction> {
        agent_allowed_actions(&self.public_id.to_string(), &self.status, api_base)
    }
}

/// Hypermedia actions for a `Harness`. See `knowledge/execution/api-conventions.md`.
pub fn harness_allowed_actions(
    id: &str,
    status: &everruns_platform::HarnessStatus,
    api_base: &str,
) -> Vec<AllowedAction> {
    use everruns_platform::HarnessStatus;
    let mut actions = vec![
        AllowedAction::new("self")
            .with_method("GET")
            .with_operation_id("get_harness")
            .with_href(format!("{api_base}/v1/harnesses/{id}"))
            .with_hint("Fetch the latest representation of this harness."),
        AllowedAction::new("update")
            .with_method("PATCH")
            .with_operation_id("update_harness")
            .with_href(format!("{api_base}/v1/harnesses/{id}"))
            .with_schema_ref("#/components/schemas/UpdateHarnessRequest")
            .with_hint("Edit harness metadata and configuration."),
    ];
    if matches!(status, HarnessStatus::Active) {
        actions.push(
            AllowedAction::new("copy")
                .with_method("POST")
                .with_operation_id("copy_harness")
                .with_href(format!("{api_base}/v1/harnesses/{id}/copy"))
                .with_hint("Fork this harness into a new editable copy."),
        );
    }
    actions.push(
        AllowedAction::new("delete")
            .with_method("DELETE")
            .with_operation_id("delete_harness")
            .with_href(format!("{api_base}/v1/harnesses/{id}"))
            .with_hint("Delete (or tombstone) this harness."),
    );
    actions
}

impl ResourceUrlable for everruns_platform::Harness {
    fn api_path() -> &'static str {
        "v1/harnesses"
    }
    fn ui_path() -> &'static str {
        "harnesses"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
    fn allowed_actions(&self, api_base: &str) -> Vec<AllowedAction> {
        harness_allowed_actions(&self.id.to_string(), &self.status, api_base)
    }
}

/// Hypermedia actions for an `App`. See `knowledge/execution/api-conventions.md`.
pub fn app_allowed_actions(
    id: &str,
    status: &everruns_platform::AppStatus,
    api_base: &str,
) -> Vec<AllowedAction> {
    use everruns_platform::AppStatus;
    let mut actions = vec![
        AllowedAction::new("self")
            .with_method("GET")
            .with_operation_id("get_app")
            .with_href(format!("{api_base}/v1/apps/{id}"))
            .with_hint("Fetch the latest representation of this app."),
        AllowedAction::new("update")
            .with_method("PATCH")
            .with_operation_id("update_app")
            .with_href(format!("{api_base}/v1/apps/{id}"))
            .with_schema_ref("#/components/schemas/UpdateAppRequest")
            .with_hint("Edit app metadata, channels, or agent binding."),
        AllowedAction::new("runs")
            .with_method("GET")
            .with_operation_id("list_app_runs")
            .with_href(format!("{api_base}/v1/apps/{id}/runs"))
            .with_hint("List recent runs (sessions) initiated through this app."),
    ];
    match status {
        AppStatus::Draft => actions.push(
            AllowedAction::new("publish")
                .with_method("POST")
                .with_operation_id("publish_app")
                .with_href(format!("{api_base}/v1/apps/{id}/publish"))
                .with_hint("Publish this draft app so its configured channels accept traffic."),
        ),
        AppStatus::Published => actions.push(
            AllowedAction::new("unpublish")
                .with_method("POST")
                .with_operation_id("unpublish_app")
                .with_href(format!("{api_base}/v1/apps/{id}/unpublish"))
                .with_hint("Revert this app to draft and stop accepting channel traffic."),
        ),
        AppStatus::Archived | AppStatus::Deleted => {}
    }
    actions.push(
        AllowedAction::new("delete")
            .with_method("DELETE")
            .with_operation_id("delete_app")
            .with_href(format!("{api_base}/v1/apps/{id}"))
            .with_hint("Delete (or tombstone) this app."),
    );
    actions
}

impl ResourceUrlable for everruns_platform::App {
    fn api_path() -> &'static str {
        "v1/apps"
    }
    fn ui_path() -> &'static str {
        "apps"
    }
    fn resource_id(&self) -> String {
        self.public_id.to_string()
    }
    fn allowed_actions(&self, api_base: &str) -> Vec<AllowedAction> {
        app_allowed_actions(&self.public_id.to_string(), &self.status, api_base)
    }
}

impl ResourceUrlable for everruns_core::budget::Budget {
    fn api_path() -> &'static str {
        "v1/budgets"
    }
    fn ui_path() -> &'static str {
        "budgets"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
    fn ui_url_path(&self) -> String {
        "budgets".to_string()
    }
}

/// Pure resolver for session hypermedia actions — split out from the
/// `ResourceUrlable` impl below so unit tests can exercise the state →
/// rel set mapping without constructing a full `Session`.
///
/// Cancel is offered only while a turn is mid-flight (`Active`);
/// pin/unpin flips on `is_pinned`; events
/// streaming, metadata edits, and delete are unconditional because the
/// API exposes them regardless of run state.
pub fn session_allowed_actions(
    id: &str,
    status: &everruns_platform::SessionStatus,
    is_pinned: bool,
    api_base: &str,
) -> Vec<AllowedAction> {
    use everruns_platform::SessionStatus;
    let mut actions = Vec::new();
    actions.push(
        AllowedAction::new("self")
            .with_method("GET")
            .with_operation_id("get_session")
            .with_href(format!("{api_base}/v1/sessions/{id}"))
            .with_hint("Fetch the latest representation of this session."),
    );
    actions.push(
        AllowedAction::new("events")
            .with_method("GET")
            .with_operation_id("list_events")
            .with_href(format!("{api_base}/v1/sessions/{id}/events"))
            .with_hint("List session events (JSON polling; supports type filters and pagination)."),
    );
    actions.push(
        AllowedAction::new("stream")
            .with_method("GET")
            .with_operation_id("stream_sse")
            .with_href(format!("{api_base}/v1/sessions/{id}/sse"))
            .with_hint("Subscribe to session events live over Server-Sent Events."),
    );
    if matches!(status, SessionStatus::Active) {
        actions.push(
            AllowedAction::new("cancel")
                .with_method("POST")
                .with_operation_id("cancel_turn")
                .with_href(format!("{api_base}/v1/sessions/{id}/cancel"))
                .with_hint("Cancel the active turn for this session."),
        );
    }
    actions.push(
        AllowedAction::new("update")
            .with_method("PATCH")
            .with_operation_id("update_session")
            .with_href(format!("{api_base}/v1/sessions/{id}"))
            .with_schema_ref("#/components/schemas/UpdateSessionRequest")
            .with_hint("Edit session metadata (title, tags, pin, etc.)."),
    );
    let pin_rel = if is_pinned {
        ("unpin", "DELETE", "unpin_session", "Unpin this session.")
    } else {
        ("pin", "PUT", "pin_session", "Pin this session.")
    };
    actions.push(
        AllowedAction::new(pin_rel.0)
            .with_method(pin_rel.1)
            .with_operation_id(pin_rel.2)
            .with_href(format!("{api_base}/v1/sessions/{id}/pin"))
            .with_hint(pin_rel.3),
    );
    actions.push(
        AllowedAction::new("delete")
            .with_method("DELETE")
            .with_operation_id("delete_session")
            .with_href(format!("{api_base}/v1/sessions/{id}"))
            .with_hint("Delete this session."),
    );
    actions
}

impl ResourceUrlable for everruns_platform::Session {
    fn api_path() -> &'static str {
        "v1/sessions"
    }
    fn ui_path() -> &'static str {
        "sessions"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
    fn ui_url_path(&self) -> String {
        format!("sessions/{}/chat", self.id)
    }
    /// Sessions hypermedia: pilot for EVE-493. Delegates to
    /// [`session_allowed_actions`] so the state → action mapping stays
    /// testable independently of the full `Session` struct. Closed
    /// `rel` vocabulary documented in `knowledge/execution/api-conventions.md`.
    fn allowed_actions(&self, api_base: &str) -> Vec<AllowedAction> {
        session_allowed_actions(
            &self.id.to_string(),
            &self.status,
            self.is_pinned.unwrap_or(false),
            api_base,
        )
    }
}

impl ResourceUrlable for everruns_core::AgentIdentity {
    fn api_path() -> &'static str {
        "v1/agent-identities"
    }
    fn ui_path() -> &'static str {
        "agent-identities"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
}

impl ResourceUrlable for everruns_core::eval::Eval {
    fn api_path() -> &'static str {
        "v1/evals"
    }
    fn ui_path() -> &'static str {
        "evals"
    }
    fn resource_id(&self) -> String {
        self.public_id.to_string()
    }
}

impl ResourceUrlable for everruns_core::McpServer {
    fn api_path() -> &'static str {
        "v1/mcp-servers"
    }
    fn ui_path() -> &'static str {
        "mcp-servers"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
    fn ui_url_path(&self) -> String {
        "mcp-servers".to_string()
    }
}

/// Hypermedia actions for a `Skill`. See `knowledge/execution/api-conventions.md`.
pub fn skill_allowed_actions(
    id: &str,
    status: &everruns_core::SkillStatus,
    api_base: &str,
) -> Vec<AllowedAction> {
    use everruns_core::SkillStatus;
    let mut actions = vec![
        AllowedAction::new("self")
            .with_method("GET")
            .with_operation_id("get_skill")
            .with_href(format!("{api_base}/v1/skills/{id}"))
            .with_hint("Fetch the latest representation of this skill."),
        AllowedAction::new("update")
            .with_method("PATCH")
            .with_operation_id("update_skill")
            .with_href(format!("{api_base}/v1/skills/{id}"))
            .with_schema_ref("#/components/schemas/UpdateSkillRequest")
            .with_hint("Edit skill metadata or contents."),
        AllowedAction::new("content")
            .with_method("GET")
            .with_operation_id("get_skill_content")
            .with_href(format!("{api_base}/v1/skills/{id}/content"))
            .with_hint("Fetch the raw SKILL.md content."),
    ];
    if matches!(status, SkillStatus::Active | SkillStatus::Disabled) {
        actions.push(
            AllowedAction::new("delete")
                .with_method("DELETE")
                .with_operation_id("delete_skill")
                .with_href(format!("{api_base}/v1/skills/{id}"))
                .with_hint("Delete (or tombstone) this skill."),
        );
    }
    actions
}

impl ResourceUrlable for everruns_core::Skill {
    fn api_path() -> &'static str {
        "v1/skills"
    }
    fn ui_path() -> &'static str {
        "skills"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
    fn ui_url_path(&self) -> String {
        "skills".to_string()
    }
    fn allowed_actions(&self, api_base: &str) -> Vec<AllowedAction> {
        skill_allowed_actions(&self.id.to_string(), &self.status, api_base)
    }
}

impl ResourceUrlable for everruns_core::provider::Provider {
    fn api_path() -> &'static str {
        "v1/providers"
    }
    fn ui_path() -> &'static str {
        "settings/providers"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
    fn ui_url_path(&self) -> String {
        "settings/providers".to_string()
    }
}

impl ResourceUrlable for everruns_core::model::Model {
    fn api_path() -> &'static str {
        "v1/models"
    }
    fn ui_path() -> &'static str {
        "models"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
    fn ui_url_path(&self) -> String {
        "models".to_string()
    }
}

impl ResourceUrlable for everruns_core::ModelWithProvider {
    fn api_path() -> &'static str {
        "v1/models"
    }
    fn ui_path() -> &'static str {
        "models"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
    fn ui_url_path(&self) -> String {
        "models".to_string()
    }
}

impl ResourceUrlable for everruns_core::session_schedule::SessionSchedule {
    fn api_path() -> &'static str {
        "v1/sessions"
    }
    fn ui_path() -> &'static str {
        "sessions"
    }
    fn resource_id(&self) -> String {
        format!("{}/schedules/{}", self.session_id, self.id)
    }
    fn ui_url_path(&self) -> String {
        format!("sessions/{}/schedules", self.session_id)
    }
}

impl ResourceUrlable for everruns_core::CapabilityInfo {
    fn api_path() -> &'static str {
        "v1/capabilities"
    }
    fn ui_path() -> &'static str {
        "capabilities"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
}

impl ResourceUrlable for crate::domains::capabilities::types::DeclarativeCapability {
    fn api_path() -> &'static str {
        "v1/capabilities/declarative"
    }
    fn ui_path() -> &'static str {
        "capabilities"
    }
    fn resource_id(&self) -> String {
        self.public_id.to_string()
    }
}

/// Verify that a session belongs to the caller's organization.
///
/// Returns Ok(()) if the session exists under org_id, or a 404 (StatusCode only)
/// if not found / wrong org. Use this before touching any session subresource
/// (files, storage, databases) to enforce tenant isolation.
pub async fn verify_session_ownership(
    db: &Arc<StorageBackend>,
    org_id: i64,
    session_id: SessionId,
) -> Result<(), StatusCode> {
    db.get_session(org_id, session_id)
        .await
        .log_internal_error("verify session ownership")?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct TestResource {
        id: String,
    }

    #[derive(Debug, Clone, Serialize)]
    struct TestResourceWithActions {
        id: String,
    }

    impl ResourceUrlable for TestResourceWithActions {
        fn api_path() -> &'static str {
            "v1/test-resources"
        }

        fn ui_path() -> &'static str {
            "test-resources"
        }

        fn resource_id(&self) -> String {
            self.id.clone()
        }

        fn allowed_actions(&self, api_base: &str) -> Vec<AllowedAction> {
            vec![AllowedAction {
                rel: "update".to_string(),
                href: Some(format!("{api_base}/{}/{}", Self::api_path(), self.id)),
                method: Some("PATCH".to_string()),
                operation_id: Some("update_test_resource".to_string()),
                schema_ref: Some("#/components/schemas/UpdateTestResourceRequest".to_string()),
                hint: None,
            }]
        }
    }

    impl ResourceUrlable for TestResource {
        fn api_path() -> &'static str {
            "v1/test-resources"
        }

        fn ui_path() -> &'static str {
            "test-resources"
        }

        fn resource_id(&self) -> String {
            self.id.clone()
        }
    }

    // Trivial derive-only serde round-trips removed; covered by the derive + handler tests.

    #[test]
    fn test_error_response_into_response() {
        let (status, json) = ErrorResponse::new("Test").into_response(StatusCode::BAD_REQUEST);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.0.detail.as_deref(), Some("Test"));
        assert_eq!(json.0.title, "Bad Request");
        assert_eq!(json.0.status, 400);
    }

    #[test]
    fn test_error_response_internal_error() {
        let (status, json) = ErrorResponse::internal_error();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json.0.detail.as_deref(), Some("Internal server error"));
        assert_eq!(json.0.code.as_deref(), Some("internal_error"));
    }

    #[test]
    fn test_error_response_not_found() {
        let (status, json) = ErrorResponse::not_found("Agent");
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json.0.detail.as_deref(), Some("Agent not found"));
        assert_eq!(json.0.code.as_deref(), Some("not_found"));
    }

    #[test]
    fn test_api_result_ext_log_internal_error() {
        let result: Result<i32, &str> = Err("db connection failed");
        let mapped = result.log_internal_error("get agent");
        assert_eq!(mapped.unwrap_err(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_api_result_ext_log_internal_error_json() {
        let result: Result<i32, &str> = Err("db connection failed");
        let (status, json) = result.log_internal_error_json("get agent").unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json.0.detail.as_deref(), Some("Internal server error"));
    }

    #[test]
    fn test_api_option_ext_ok_or_not_found() {
        let some: Option<i32> = Some(42);
        assert_eq!(some.ok_or_not_found().unwrap(), 42);

        let none: Option<i32> = None;
        assert_eq!(none.ok_or_not_found().unwrap_err(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_api_option_ext_ok_or_not_found_json() {
        let some: Option<i32> = Some(42);
        assert_eq!(some.ok_or_not_found_json("Agent").unwrap(), 42);

        let none: Option<i32> = None;
        let (status, json) = none.ok_or_not_found_json("Agent").unwrap_err();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json.0.detail.as_deref(), Some("Agent not found"));
    }

    #[test]
    fn test_paginated_response_serialization() {
        let response = PaginatedResponse::new(vec!["a", "b"], 50, 10, 5);
        let json = serde_json::to_string(&response).unwrap();

        // Verify JSON structure
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["data"], serde_json::json!(["a", "b"]));
        assert_eq!(parsed["total"], 50);
        assert_eq!(parsed["offset"], 10);
        assert_eq!(parsed["limit"], 5);
        // next_url / prev_url are absent until the middleware fills them in.
        assert!(parsed.get("next_url").is_none());
        assert!(parsed.get("prev_url").is_none());
    }

    #[test]
    fn inject_pagination_links_adds_next_and_prev_on_middle_page() {
        let api_base = "https://api.example/api";
        let mut value =
            serde_json::json!({"data": [1,2,3], "total": 100, "offset": 20, "limit": 10});
        let changed =
            inject_pagination_links(&mut value, api_base, "/v1/agents?offset=20&limit=10");
        assert!(changed);
        assert_eq!(
            value["next_url"],
            "https://api.example/api/v1/agents?offset=30&limit=10"
        );
        assert_eq!(
            value["prev_url"],
            "https://api.example/api/v1/agents?offset=10&limit=10"
        );
    }

    #[test]
    fn inject_pagination_links_omits_next_on_last_page() {
        let mut value = serde_json::json!({"data": [], "total": 25, "offset": 20, "limit": 10});
        let changed = inject_pagination_links(
            &mut value,
            "https://api.example/api",
            "/v1/agents?offset=20&limit=10",
        );
        assert!(changed);
        assert!(value.get("next_url").is_none());
        assert!(value.get("prev_url").is_some());
    }

    #[test]
    fn inject_pagination_links_omits_prev_on_first_page() {
        let mut value = serde_json::json!({"data": [], "total": 25, "offset": 0, "limit": 10});
        let changed = inject_pagination_links(
            &mut value,
            "https://api.example/api",
            "/v1/agents?offset=0&limit=10",
        );
        assert!(changed);
        assert!(value.get("next_url").is_some());
        assert!(value.get("prev_url").is_none());
    }

    #[test]
    fn inject_pagination_links_preserves_other_query_params() {
        let mut value = serde_json::json!({"data": [], "total": 100, "offset": 10, "limit": 10});
        let changed = inject_pagination_links(
            &mut value,
            "https://api.example/api",
            "/v1/agents?search=foo&offset=10&limit=10&include_archived=true",
        );
        assert!(changed);
        assert_eq!(
            value["next_url"],
            "https://api.example/api/v1/agents?search=foo&offset=20&limit=10&include_archived=true"
        );
    }

    #[test]
    fn inject_pagination_links_appends_offset_when_missing_from_query() {
        let mut value = serde_json::json!({"data": [], "total": 100, "offset": 0, "limit": 10});
        let changed = inject_pagination_links(
            &mut value,
            "https://api.example/api",
            "/v1/agents?search=foo",
        );
        assert!(changed);
        assert_eq!(
            value["next_url"],
            "https://api.example/api/v1/agents?search=foo&offset=10"
        );
    }

    #[test]
    fn inject_pagination_links_skips_objects_without_pagination_fields() {
        let mut value = serde_json::json!({"data": [1,2,3]});
        let changed = inject_pagination_links(&mut value, "https://x", "/y");
        assert!(!changed);
        assert!(value.get("next_url").is_none());
    }

    #[test]
    fn inject_pagination_links_preserves_handler_supplied_next_url() {
        let mut value = serde_json::json!({
            "data": [],
            "total": 100,
            "offset": 20,
            "limit": 10,
            "next_url": "https://custom/cursor-based-link"
        });
        let changed = inject_pagination_links(
            &mut value,
            "https://api.example/api",
            "/v1/agents?offset=20&limit=10",
        );
        assert!(changed);
        // Handler-supplied next_url is preserved.
        assert_eq!(value["next_url"], "https://custom/cursor-based-link");
        // prev_url is still filled in by the middleware.
        assert_eq!(
            value["prev_url"],
            "https://api.example/api/v1/agents?offset=10&limit=10"
        );
    }

    #[test]
    fn inject_pagination_links_preserves_handler_supplied_prev_url() {
        let mut value = serde_json::json!({
            "data": [],
            "total": 100,
            "offset": 20,
            "limit": 10,
            "prev_url": "https://custom/cursor-based-link"
        });
        let changed = inject_pagination_links(
            &mut value,
            "https://api.example/api",
            "/v1/agents?offset=20&limit=10",
        );
        assert!(changed);
        // Handler-supplied prev_url is preserved.
        assert_eq!(value["prev_url"], "https://custom/cursor-based-link");
        // next_url is still filled in by the middleware.
        assert_eq!(
            value["next_url"],
            "https://api.example/api/v1/agents?offset=30&limit=10"
        );
    }

    #[test]
    fn test_url_builder_wrap_includes_ui_link_alias() {
        let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
        let wrapped = builder.wrap(TestResource {
            id: "resource_1".to_string(),
        });

        assert_eq!(
            wrapped.self_url,
            "https://api.example/api/v1/test-resources/resource_1"
        );
        assert_eq!(
            wrapped.view_url,
            "https://app.example/test-resources/resource_1"
        );
        assert_eq!(wrapped.ui_link, wrapped.view_url);
    }

    #[test]
    fn test_link_decoration_requires_bounded_content_length() {
        let missing_length = Response::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::empty())
            .unwrap();
        assert!(!response_within_link_decoration_limit(&missing_length));

        let large_response = Response::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                header::CONTENT_LENGTH,
                (LINK_DECORATION_MAX_RESPONSE_BYTES + 1).to_string(),
            )
            .body(Body::empty())
            .unwrap();
        assert!(!response_within_link_decoration_limit(&large_response));

        let bounded_response = Response::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, "128")
            .body(Body::empty())
            .unwrap();
        assert!(response_within_link_decoration_limit(&bounded_response));
    }

    #[test]
    fn test_decorate_value_links_adds_command_session_link() {
        let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
        let mut value = serde_json::json!({
            "session_id": "session_00000000000000000000000000000001",
            "message_id": "message_00000000000000000000000000000002"
        });

        assert!(builder.decorate_value_links(&mut value));
        assert!(value.get("self_url").is_none());
        assert_eq!(
            value["ui_link"],
            "https://app.example/sessions/session_00000000000000000000000000000001/chat"
        );
    }

    #[test]
    fn test_decorate_value_links_adds_nested_capability_link() {
        let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
        let mut value = serde_json::json!({
            "data": [{
                "id": "platform_management",
                "name": "Platform Management",
                "type": "builtin"
            }]
        });

        assert!(builder.decorate_value_links(&mut value));
        assert_eq!(
            value["data"][0]["view_url"],
            "https://app.example/capabilities/platform_management"
        );
    }

    #[test]
    fn test_decorate_value_links_aligns_budget_ui_link() {
        let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
        let mut value = serde_json::json!({
            "id": "bdgt_00000000000000000000000000000001",
            "subject_type": "session"
        });

        assert!(builder.decorate_value_links(&mut value));
        assert_eq!(value["view_url"], "https://app.example/budgets");
        assert_eq!(value["ui_link"], value["view_url"]);
    }

    #[test]
    fn test_decorate_value_links_preserves_existing_links() {
        let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
        let mut value = serde_json::json!({
            "id": "agent_00000000000000000000000000000001",
            "self_url": "https://custom.example/api",
            "view_url": "https://custom.example/view"
        });

        assert!(builder.decorate_value_links(&mut value));
        assert_eq!(value["self_url"], "https://custom.example/api");
        assert_eq!(value["view_url"], "https://custom.example/view");
        assert_eq!(value["ui_link"], "https://custom.example/view");
    }

    #[test]
    fn test_decorate_value_links_aliases_existing_view_url() {
        let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
        let mut value = serde_json::json!({
            "id": "compaction",
            "name": "Compaction",
            "view_url": "https://app.example/capabilities/compaction"
        });

        assert!(builder.decorate_value_links(&mut value));
        assert_eq!(
            value["ui_link"],
            "https://app.example/capabilities/compaction"
        );
    }

    #[test]
    fn problem_details_constructors_set_status_title_code() {
        let cases = [
            (
                ErrorResponse::not_found("Agent"),
                StatusCode::NOT_FOUND,
                "Not Found",
                "not_found",
                "Agent not found",
            ),
            (
                ErrorResponse::conflict("agent already exists"),
                StatusCode::CONFLICT,
                "Conflict",
                "conflict",
                "agent already exists",
            ),
            (
                ErrorResponse::internal_error(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Server Error",
                "internal_error",
                "Internal server error",
            ),
            (
                ErrorResponse::bad_gateway(),
                StatusCode::BAD_GATEWAY,
                "Bad Gateway",
                "bad_gateway",
                "Bad gateway",
            ),
        ];
        for (i, (response, expected_status, title, code, detail)) in cases.iter().enumerate() {
            let (status, Json(body)) = response;
            assert_eq!(status, expected_status, "case {i} status");
            assert_eq!(body.title, *title, "case {i} title");
            assert_eq!(
                body.status,
                expected_status.as_u16(),
                "case {i} status code"
            );
            assert_eq!(body.code.as_deref(), Some(*code), "case {i} code");
            assert_eq!(body.detail.as_deref(), Some(*detail), "case {i} detail");
            assert!(body.allowed_actions.is_empty(), "case {i} actions");
            assert!(body.retry_after_seconds.is_none(), "case {i} retry");
        }
    }

    #[test]
    fn problem_details_builders_add_extensions() {
        let response = ErrorResponse::new("Shorten 'name'")
            .with_title("Validation failed")
            .with_code("agent_name_too_long")
            .with_type("https://errors.everruns.com/validation/agent-name-too-long")
            .with_instance("/v1/agents")
            .with_action(
                AllowedAction::new("retry")
                    .with_operation_id("create_agent")
                    .with_hint("Shorten 'name' to <= 200 chars."),
            )
            .into_response(StatusCode::UNPROCESSABLE_ENTITY);

        let value = serde_json::to_value(&response.1.0).unwrap();
        assert_eq!(value["title"], "Validation failed");
        assert_eq!(value["status"], 422);
        assert_eq!(value["detail"], "Shorten 'name'");
        assert_eq!(value["code"], "agent_name_too_long");
        assert_eq!(value["instance"], "/v1/agents");
        assert_eq!(
            value["type"],
            "https://errors.everruns.com/validation/agent-name-too-long"
        );
        let actions = value["allowed_actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["rel"], "retry");
        assert_eq!(actions[0]["operation_id"], "create_agent");
    }

    #[test]
    fn problem_details_omits_unset_optional_fields() {
        let (_, body) = ErrorResponse::not_found("Agent");
        let value = serde_json::to_value(&body.0).unwrap();
        assert!(value.get("type").is_none());
        assert!(value.get("instance").is_none());
        assert!(value.get("retry_after_seconds").is_none());
        assert!(value.get("allowed_actions").is_none());
    }

    #[test]
    fn problem_details_rate_limit_carries_retry_after() {
        let body = ErrorResponse::new("Too many requests.")
            .with_code("rate_limited")
            .with_retry_after(60)
            .with_action(
                AllowedAction::new("retry-later")
                    .with_hint("Wait retry_after_seconds before retrying."),
            )
            .into_response(StatusCode::TOO_MANY_REQUESTS);
        let value = serde_json::to_value(&body.1.0).unwrap();
        assert_eq!(value["status"], 429);
        assert_eq!(value["code"], "rate_limited");
        assert_eq!(value["retry_after_seconds"], 60);
        assert_eq!(value["allowed_actions"][0]["rel"], "retry-later");
    }

    #[test]
    fn session_actions_include_cancel_when_turn_is_active() {
        use everruns_platform::SessionStatus;
        let actions = session_allowed_actions(
            "session_01",
            &SessionStatus::Active,
            false,
            "https://api.example",
        );
        let rels: Vec<&str> = actions.iter().map(|a| a.rel.as_str()).collect();
        assert!(
            rels.contains(&"cancel"),
            "active turn should expose cancel: {rels:?}"
        );
        let cancel = actions.iter().find(|a| a.rel == "cancel").unwrap();
        assert_eq!(cancel.method.as_deref(), Some("POST"));
        assert_eq!(cancel.operation_id.as_deref(), Some("cancel_turn"));
        assert_eq!(
            cancel.href.as_deref(),
            Some("https://api.example/v1/sessions/session_01/cancel")
        );
    }

    #[test]
    fn session_actions_omit_cancel_in_idle_or_paused_states() {
        use everruns_platform::SessionStatus;
        for status in [
            SessionStatus::Started,
            SessionStatus::Idle,
            SessionStatus::WaitingForToolResults,
            SessionStatus::Paused,
        ] {
            let actions =
                session_allowed_actions("session_01", &status, false, "https://api.example");
            assert!(
                actions.iter().all(|a| a.rel != "cancel"),
                "{status:?}: cancel must not appear when no turn is mid-flight"
            );
        }
    }

    #[test]
    fn session_actions_flip_pin_rel_on_is_pinned() {
        use everruns_platform::SessionStatus;
        let unpinned = session_allowed_actions(
            "session_01",
            &SessionStatus::Idle,
            false,
            "https://api.example",
        );
        let pinned = session_allowed_actions(
            "session_01",
            &SessionStatus::Idle,
            true,
            "https://api.example",
        );
        let unpinned_rels: Vec<&str> = unpinned.iter().map(|a| a.rel.as_str()).collect();
        let pinned_rels: Vec<&str> = pinned.iter().map(|a| a.rel.as_str()).collect();
        assert!(
            unpinned_rels.contains(&"pin") && !unpinned_rels.contains(&"unpin"),
            "unpinned session offers pin, not unpin: {unpinned_rels:?}"
        );
        assert!(
            pinned_rels.contains(&"unpin") && !pinned_rels.contains(&"pin"),
            "pinned session offers unpin, not pin: {pinned_rels:?}"
        );
        let pin = unpinned.iter().find(|a| a.rel == "pin").unwrap();
        assert_eq!(pin.method.as_deref(), Some("PUT"));
        let unpin = pinned.iter().find(|a| a.rel == "unpin").unwrap();
        assert_eq!(unpin.method.as_deref(), Some("DELETE"));
    }

    #[test]
    fn session_actions_always_include_self_events_stream_update_delete() {
        use everruns_platform::SessionStatus;
        let actions = session_allowed_actions(
            "session_xyz",
            &SessionStatus::Idle,
            false,
            "https://api.example",
        );
        let rels: Vec<&str> = actions.iter().map(|a| a.rel.as_str()).collect();
        for expected in ["self", "events", "stream", "update", "delete"] {
            assert!(
                rels.contains(&expected),
                "missing rel `{expected}` in {rels:?}"
            );
        }
        let update = actions.iter().find(|a| a.rel == "update").unwrap();
        assert_eq!(
            update.schema_ref.as_deref(),
            Some("#/components/schemas/UpdateSessionRequest"),
            "update action must reference its request body schema"
        );
        // `events` is JSON polling (operationId list_events); `stream` is SSE
        // (operationId stream_sse, /sse path).
        let events = actions.iter().find(|a| a.rel == "events").unwrap();
        assert_eq!(events.operation_id.as_deref(), Some("list_events"));
        assert!(events.href.as_deref().unwrap().ends_with("/events"));
        let stream = actions.iter().find(|a| a.rel == "stream").unwrap();
        assert_eq!(stream.operation_id.as_deref(), Some("stream_sse"));
        assert!(stream.href.as_deref().unwrap().ends_with("/sse"));
    }

    #[test]
    fn with_urls_preserves_allowed_actions_for_resource_with_counts() {
        let builder = UrlBuilder::new("https://api.example", "https://app.example");
        let wrapped = builder.wrap(ResourceWithCounts {
            session_count: 1,
            app_count: 2,
            inner: TestResourceWithActions {
                id: "resource_1".to_string(),
            },
        });
        assert_eq!(wrapped.allowed_actions.len(), 1);
        assert_eq!(wrapped.allowed_actions[0].rel, "update");
    }

    #[test]
    fn with_urls_skips_empty_allowed_actions() {
        let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
        let wrapped = builder.wrap(TestResource {
            id: "resource_1".to_string(),
        });
        let value = serde_json::to_value(&wrapped).unwrap();
        assert!(
            value.get("allowed_actions").is_none(),
            "resources that didn't opt in must not emit allowed_actions: {value}"
        );
    }

    // ----------------------------------------------------------------
    // Hypermedia rollout — EVE-494
    // ----------------------------------------------------------------

    #[test]
    fn agent_actions_offer_copy_only_when_active() {
        use everruns_platform::AgentStatus;
        let active = agent_allowed_actions("agent_01", &AgentStatus::Active, "https://api.example");
        let archived =
            agent_allowed_actions("agent_01", &AgentStatus::Archived, "https://api.example");
        assert!(
            active.iter().any(|a| a.rel == "copy"),
            "active agent should expose copy"
        );
        assert!(
            !archived.iter().any(|a| a.rel == "copy"),
            "archived agent should not expose copy"
        );
        let rels: Vec<&str> = active.iter().map(|a| a.rel.as_str()).collect();
        for expected in ["self", "update", "versions", "delete"] {
            assert!(
                rels.contains(&expected),
                "missing rel `{expected}` for active agent: {rels:?}"
            );
        }
        let update = active.iter().find(|a| a.rel == "update").unwrap();
        assert_eq!(
            update.schema_ref.as_deref(),
            Some("#/components/schemas/UpdateAgentRequest")
        );
    }

    #[test]
    fn harness_actions_offer_copy_only_when_active() {
        use everruns_platform::HarnessStatus;
        let active =
            harness_allowed_actions("harness_01", &HarnessStatus::Active, "https://api.example");
        let archived = harness_allowed_actions(
            "harness_01",
            &HarnessStatus::Archived,
            "https://api.example",
        );
        assert!(active.iter().any(|a| a.rel == "copy"));
        assert!(!archived.iter().any(|a| a.rel == "copy"));
    }

    #[test]
    fn app_actions_flip_publish_unpublish_on_status() {
        use everruns_platform::AppStatus;
        let draft = app_allowed_actions("app_01", &AppStatus::Draft, "https://api.example");
        let published = app_allowed_actions("app_01", &AppStatus::Published, "https://api.example");
        let archived = app_allowed_actions("app_01", &AppStatus::Archived, "https://api.example");

        let draft_rels: Vec<&str> = draft.iter().map(|a| a.rel.as_str()).collect();
        let published_rels: Vec<&str> = published.iter().map(|a| a.rel.as_str()).collect();
        let archived_rels: Vec<&str> = archived.iter().map(|a| a.rel.as_str()).collect();

        assert!(
            draft_rels.contains(&"publish") && !draft_rels.contains(&"unpublish"),
            "draft app offers publish, not unpublish: {draft_rels:?}"
        );
        assert!(
            published_rels.contains(&"unpublish") && !published_rels.contains(&"publish"),
            "published app offers unpublish, not publish: {published_rels:?}"
        );
        assert!(
            !archived_rels.contains(&"publish") && !archived_rels.contains(&"unpublish"),
            "archived app offers neither: {archived_rels:?}"
        );
        // `runs` is always present.
        for rels in [&draft_rels, &published_rels, &archived_rels] {
            assert!(
                rels.contains(&"runs"),
                "expected runs rel in every state: {rels:?}"
            );
        }
    }

    #[test]
    fn skill_actions_omit_delete_for_terminal_states() {
        use everruns_core::SkillStatus;
        for status in [SkillStatus::Active, SkillStatus::Disabled] {
            let actions = skill_allowed_actions("skill_01", &status, "https://api.example");
            assert!(
                actions.iter().any(|a| a.rel == "delete"),
                "{status:?}: expected delete"
            );
        }
        for status in [SkillStatus::Archived, SkillStatus::Deleted] {
            let actions = skill_allowed_actions("skill_01", &status, "https://api.example");
            assert!(
                !actions.iter().any(|a| a.rel == "delete"),
                "{status:?}: delete should not appear on already-terminal states"
            );
        }
    }
}
