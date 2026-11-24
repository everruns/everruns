// Common DTOs for public API
//
// These types are shared across multiple API endpoints.
// ApiResult: standard return type for API handlers
// impl_auth_state!: macro to eliminate repeated FromRef<AppState> for AuthState impls

pub use crate::common_dto::{
    AllowedAction, ErrorResponse, ListResponse, Pagination, deserialize_nullable_update_field,
};
pub use crate::resource_links::UrlBuilder;
use axum::Json;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::SessionId;
use serde::{Deserialize, Serialize};
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

fn classify_anyhow_error(error: &anyhow::Error) -> Option<(StatusCode, Json<ErrorResponse>)> {
    let message = error.to_string();
    let lowered = message.to_ascii_lowercase();

    if crate::errors::is_database_unique_violation(error) {
        // THREAT[TM-API-005]: Keep storage diagnostics in server logs only.
        tracing::warn!(error = ?error, "database uniqueness conflict");
        return Some(
            ErrorResponse::new(crate::errors::ALREADY_EXISTS_DETAIL)
                .with_code(crate::errors::ALREADY_EXISTS_CODE)
                .into_response(StatusCode::CONFLICT),
        );
    }
    if crate::errors::is_domain_already_exists_message(&message) {
        return Some(
            ErrorResponse::new(message)
                .with_code(crate::errors::ALREADY_EXISTS_CODE)
                .into_response(StatusCode::CONFLICT),
        );
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
            } else if let Some(response) = classify_anyhow_error(&e) {
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

// ============================================================================
// Resource URL enrichment
// ============================================================================

// `UrlBuilder` itself (construction and JSON link decoration) lives in
// `crate::resource_links`; the typed hypermedia wrapping below is HTTP-only.
impl UrlBuilder {
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

/// Trait for resources that can have `url` and `view_url` generated.
pub trait ResourceUrlable {
    /// API path segment (e.g. `"v1/platform/agents"`).
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
    /// responsible for the versioned path.
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

/// Hypermedia actions for an `Agent`.
pub fn agent_allowed_actions(
    id: &str,
    status: &crate::domains::agents::record::AgentStatus,
    api_base: &str,
) -> Vec<AllowedAction> {
    use crate::domains::agents::record::AgentStatus;
    let mut actions = vec![
        AllowedAction::new("self")
            .with_method("GET")
            .with_operation_id("get_agent")
            .with_href(format!("{api_base}/v1/platform/agents/{id}"))
            .with_hint("Fetch the latest representation of this agent."),
        AllowedAction::new("update")
            .with_method("PATCH")
            .with_operation_id("update_agent")
            .with_href(format!("{api_base}/v1/platform/agents/{id}"))
            .with_schema_ref("#/components/schemas/UpdateAgentRequest")
            .with_hint("Edit agent metadata (name, system prompt, capabilities, etc.)."),
        AllowedAction::new("history")
            .with_method("GET")
            .with_operation_id("list_entity_history")
            .with_href(format!("{api_base}/v1/platform/history/{id}"))
            .with_hint("List this agent's change history; restore a past revision from it."),
    ];
    if matches!(status, AgentStatus::Active) {
        actions.push(
            AllowedAction::new("copy")
                .with_method("POST")
                .with_operation_id("copy_agent")
                .with_href(format!("{api_base}/v1/platform/agents/{id}/copy"))
                .with_hint("Fork this agent into a new editable copy."),
        );
    }
    actions.push(
        AllowedAction::new("delete")
            .with_method("DELETE")
            .with_operation_id("delete_agent")
            .with_href(format!("{api_base}/v1/platform/agents/{id}"))
            .with_hint("Delete (or tombstone) this agent."),
    );
    actions
}

impl ResourceUrlable for crate::domains::agents::record::Agent {
    fn api_path() -> &'static str {
        "v1/platform/agents"
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

/// Hypermedia actions for a `Harness`.
pub fn harness_allowed_actions(
    id: &str,
    status: &crate::domains::harnesses::record::HarnessStatus,
    api_base: &str,
) -> Vec<AllowedAction> {
    use crate::domains::harnesses::record::HarnessStatus;
    let mut actions = vec![
        AllowedAction::new("self")
            .with_method("GET")
            .with_operation_id("get_harness")
            .with_href(format!("{api_base}/v1/platform/harnesses/{id}"))
            .with_hint("Fetch the latest representation of this harness."),
        AllowedAction::new("update")
            .with_method("PATCH")
            .with_operation_id("update_harness")
            .with_href(format!("{api_base}/v1/platform/harnesses/{id}"))
            .with_schema_ref("#/components/schemas/UpdateHarnessRequest")
            .with_hint("Edit harness metadata and configuration."),
    ];
    if matches!(status, HarnessStatus::Active) {
        actions.push(
            AllowedAction::new("copy")
                .with_method("POST")
                .with_operation_id("copy_harness")
                .with_href(format!("{api_base}/v1/platform/harnesses/{id}/copy"))
                .with_hint("Fork this harness into a new editable copy."),
        );
    }
    actions.push(
        AllowedAction::new("delete")
            .with_method("DELETE")
            .with_operation_id("delete_harness")
            .with_href(format!("{api_base}/v1/platform/harnesses/{id}"))
            .with_hint("Delete (or tombstone) this harness."),
    );
    actions
}

impl ResourceUrlable for crate::domains::harnesses::record::Harness {
    fn api_path() -> &'static str {
        "v1/platform/harnesses"
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

/// Hypermedia actions for an archival `App`.
///
/// Apps are retired (agent-exposure). Only the read endpoint survives, so `self`
/// is the only action that resolves — advertising `update`, `runs`, `publish`,
/// `unpublish` or `delete` here would hand a client four routes that no longer
/// exist. Endpoint management lives under the owning agent.
pub fn app_allowed_actions(id: &str, api_base: &str) -> Vec<AllowedAction> {
    vec![
        AllowedAction::new("self")
            .with_method("GET")
            .with_operation_id("get_app")
            .with_href(format!("{api_base}/v1/platform/apps/{id}"))
            .with_hint("Fetch this archival App record. Read-only and deprecated."),
    ]
}

/// UI destination for an archival `App` record.
///
/// The `/apps` pages were deleted with the App surface, so a link there would
/// 404. Send a reader to the agent that owns its endpoints now, or to the
/// cross-agent exposures view when the row predates agent assignment.
pub fn archival_app_ui_path(agent_id: Option<&str>) -> String {
    match agent_id {
        Some(agent_id) => format!("agents/{agent_id}?tab=integrations"),
        None => "exposures".to_string(),
    }
}

impl ResourceUrlable for crate::domains::apps::record::App {
    fn api_path() -> &'static str {
        "v1/platform/apps"
    }
    /// Unused: `ui_url_path` is overridden, because the `/apps` pages were
    /// deleted with the App surface and a link there would 404.
    fn ui_path() -> &'static str {
        "exposures"
    }
    fn resource_id(&self) -> String {
        self.public_id.to_string()
    }
    /// An archival App record has no page of its own. See
    /// [`archival_app_ui_path`].
    fn ui_url_path(&self) -> String {
        archival_app_ui_path(self.agent_id.as_ref().map(|id| id.to_string()).as_deref())
    }
    fn allowed_actions(&self, api_base: &str) -> Vec<AllowedAction> {
        app_allowed_actions(&self.public_id.to_string(), api_base)
    }
}

impl ResourceUrlable for crate::domains::budgets::record::Budget {
    fn api_path() -> &'static str {
        "v1/platform/budgets"
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
/// pin/unpin flips on `is_pinned`, archive/unarchive on `is_archived`; events
/// streaming, metadata edits, and delete are unconditional because the
/// API exposes them regardless of run state.
pub fn session_allowed_actions(
    id: &str,
    status: &crate::domains::sessions::record::SessionStatus,
    is_pinned: bool,
    is_archived: bool,
    api_base: &str,
) -> Vec<AllowedAction> {
    use crate::domains::sessions::record::SessionStatus;
    let mut actions = Vec::new();
    actions.push(
        AllowedAction::new("self")
            .with_method("GET")
            .with_operation_id("get_session")
            .with_href(format!("{api_base}/v1/platform/sessions/{id}"))
            .with_hint("Fetch the latest representation of this session."),
    );
    actions.push(
        AllowedAction::new("events")
            .with_method("GET")
            .with_operation_id("list_events")
            .with_href(format!("{api_base}/v1/platform/sessions/{id}/events"))
            .with_hint("List session events (JSON polling; supports type filters and pagination)."),
    );
    actions.push(
        AllowedAction::new("stream")
            .with_method("GET")
            .with_operation_id("stream_sse")
            .with_href(format!("{api_base}/v1/platform/sessions/{id}/sse"))
            .with_hint("Subscribe to session events live over Server-Sent Events."),
    );
    if matches!(status, SessionStatus::Active) {
        actions.push(
            AllowedAction::new("cancel")
                .with_method("POST")
                .with_operation_id("cancel_turn")
                .with_href(format!("{api_base}/v1/platform/sessions/{id}/cancel"))
                .with_hint("Cancel the active turn for this session."),
        );
    }
    actions.push(
        AllowedAction::new("update")
            .with_method("PATCH")
            .with_operation_id("update_session")
            .with_href(format!("{api_base}/v1/platform/sessions/{id}"))
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
            .with_href(format!("{api_base}/v1/platform/sessions/{id}/pin"))
            .with_hint(pin_rel.3),
    );
    let archive_rel = if is_archived {
        (
            "unarchive",
            "DELETE",
            "unarchive_session",
            "Restore this session to default lists.",
        )
    } else {
        (
            "archive",
            "PUT",
            "archive_session",
            "Archive this session so it drops out of default lists.",
        )
    };
    actions.push(
        AllowedAction::new(archive_rel.0)
            .with_method(archive_rel.1)
            .with_operation_id(archive_rel.2)
            .with_href(format!("{api_base}/v1/platform/sessions/{id}/archive"))
            .with_hint(archive_rel.3),
    );
    actions.push(
        AllowedAction::new("delete")
            .with_method("DELETE")
            .with_operation_id("delete_session")
            .with_href(format!("{api_base}/v1/platform/sessions/{id}"))
            .with_hint("Delete this session."),
    );
    actions
}

impl ResourceUrlable for crate::domains::sessions::record::Session {
    fn api_path() -> &'static str {
        "v1/platform/sessions"
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
    /// testable independently of the full `Session` struct.
    fn allowed_actions(&self, api_base: &str) -> Vec<AllowedAction> {
        session_allowed_actions(
            &self.id.to_string(),
            &self.status,
            self.is_pinned.unwrap_or(false),
            self.archived_at.is_some(),
            api_base,
        )
    }
}

impl ResourceUrlable for crate::domains::virtual_users::record::VirtualUser {
    fn api_path() -> &'static str {
        "v1/platform/virtual-users"
    }
    fn ui_path() -> &'static str {
        "virtual-users"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
}

impl ResourceUrlable for crate::domains::evals::record::Eval {
    fn api_path() -> &'static str {
        "v1/platform/evals"
    }
    fn ui_path() -> &'static str {
        "evals"
    }
    fn resource_id(&self) -> String {
        self.public_id.to_string()
    }
}

impl ResourceUrlable for crate::domains::mcp_servers::record::McpServer {
    fn api_path() -> &'static str {
        "v1/platform/mcp-servers"
    }
    fn ui_path() -> &'static str {
        "settings/mcp-catalog"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
    fn ui_url_path(&self) -> String {
        "settings/mcp-catalog".to_string()
    }
}

/// Hypermedia actions for a `Skill`.
pub fn skill_allowed_actions(
    id: &str,
    status: &crate::domains::skills::record::SkillStatus,
    api_base: &str,
) -> Vec<AllowedAction> {
    use crate::domains::skills::record::SkillStatus;
    let mut actions = vec![
        AllowedAction::new("self")
            .with_method("GET")
            .with_operation_id("get_skill")
            .with_href(format!("{api_base}/v1/platform/skills/{id}"))
            .with_hint("Fetch the latest representation of this skill."),
        AllowedAction::new("update")
            .with_method("PATCH")
            .with_operation_id("update_skill")
            .with_href(format!("{api_base}/v1/platform/skills/{id}"))
            .with_schema_ref("#/components/schemas/UpdateSkillRequest")
            .with_hint("Edit skill metadata or contents."),
        AllowedAction::new("content")
            .with_method("GET")
            .with_operation_id("get_skill_content")
            .with_href(format!("{api_base}/v1/platform/skills/{id}/content"))
            .with_hint("Fetch the raw SKILL.md content."),
    ];
    if matches!(status, SkillStatus::Active | SkillStatus::Disabled) {
        actions.push(
            AllowedAction::new("delete")
                .with_method("DELETE")
                .with_operation_id("delete_skill")
                .with_href(format!("{api_base}/v1/platform/skills/{id}"))
                .with_hint("Delete (or tombstone) this skill."),
        );
    }
    actions
}

impl ResourceUrlable for crate::domains::skills::record::Skill {
    fn api_path() -> &'static str {
        "v1/platform/skills"
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

impl ResourceUrlable for crate::domains::providers::record::Provider {
    fn api_path() -> &'static str {
        "v1/platform/providers"
    }
    fn ui_path() -> &'static str {
        "models/providers"
    }
    fn resource_id(&self) -> String {
        self.id.to_string()
    }
    fn ui_url_path(&self) -> String {
        format!("models/providers/{}", self.id)
    }
}

impl ResourceUrlable for crate::domains::models::record::Model {
    fn api_path() -> &'static str {
        "v1/platform/models"
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

impl ResourceUrlable for crate::domains::models::record::ModelWithProvider {
    fn api_path() -> &'static str {
        "v1/platform/models"
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
        "v1/platform/sessions"
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
        "v1/platform/capabilities"
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
        "v1/platform/capabilities/declarative"
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
mod tests;
