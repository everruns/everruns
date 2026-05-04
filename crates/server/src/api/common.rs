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

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ResourceStatsResponse {
    pub session_count: u64,
    pub active_session_count: u64,
    pub idle_session_count: u64,
    pub started_session_count: u64,
    pub waiting_for_tool_results_session_count: u64,
    pub execution_count: u64,
    pub total_session_duration_ms: u64,
    pub avg_session_duration_ms: Option<u64>,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub first_session_at: Option<DateTime<Utc>>,
    pub last_session_at: Option<DateTime<Utc>>,
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

/// Standard error response for API endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorResponse {
    /// Error message describing what went wrong.
    pub error: String,
}

impl ErrorResponse {
    pub fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
        }
    }

    /// Convert to axum response tuple
    pub fn into_response(self, status: StatusCode) -> (StatusCode, Json<Self>) {
        (status, Json(self))
    }

    /// Create an internal server error response
    pub fn internal_error() -> (StatusCode, Json<Self>) {
        Self::new("Internal server error").into_response(StatusCode::INTERNAL_SERVER_ERROR)
    }

    /// Create a not found error response
    pub fn not_found(resource: &str) -> (StatusCode, Json<Self>) {
        Self::new(format!("{} not found", resource)).into_response(StatusCode::NOT_FOUND)
    }

    /// Create a conflict error response (409)
    pub fn conflict(message: &str) -> (StatusCode, Json<Self>) {
        Self::new(message).into_response(StatusCode::CONFLICT)
    }

    /// Create a bad gateway error response (502)
    pub fn bad_gateway() -> (StatusCode, Json<Self>) {
        Self::new("Bad gateway").into_response(StatusCode::BAD_GATEWAY)
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
            } else if let Some(policy_err) = e.downcast_ref::<everruns_core::PolicyError>() {
                (
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse::new(&policy_err.message)),
                )
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
}

impl<T> PaginatedResponse<T> {
    pub fn new(data: Vec<T>, total: u32, offset: u32, limit: u32) -> Self {
        Self {
            data,
            total,
            offset,
            limit,
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

    /// Wrap a single resource with API and UI links.
    pub fn wrap<T: ResourceUrlable + Serialize>(&self, item: T) -> WithUrls<T> {
        let api_path = item.api_url_path();
        let ui_path = item.ui_url_path();
        let view_url = format!("{}/{}", self.ui_base, ui_path);
        WithUrls {
            self_url: format!("{}/{}", self.api_base, api_path),
            view_url: view_url.clone(),
            ui_link: view_url,
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
        "provider" => ("v1/llm-providers", "settings/providers".to_string()),
        "model" => ("v1/llm-models", "models".to_string()),
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
}

/// Wrapper that adds API and UI links to a serialized resource.
///
/// Uses `self_url` (not `url`) for the API link to avoid collision with
/// resources that already have a `url` field (e.g. McpServer).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WithUrls<T: Serialize> {
    /// Full API endpoint URL for this resource.
    pub self_url: String,
    /// Full UI URL for viewing this resource.
    pub view_url: String,
    /// Alias for `view_url`, used by command and MCP outputs.
    pub ui_link: String,
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
}

impl<T: ResourceUrlable + Serialize> PaginatedResponse<T> {
    /// Map all items through `UrlBuilder::wrap`.
    pub fn with_urls(self, builder: &UrlBuilder) -> PaginatedResponse<WithUrls<T>> {
        PaginatedResponse {
            data: builder.wrap_vec(self.data),
            total: self.total,
            offset: self.offset,
            limit: self.limit,
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

macro_rules! impl_resource_urlable {
    ($ty:ty, $api:expr, $ui:expr, $id_field:ident) => {
        impl ResourceUrlable for $ty {
            fn api_path() -> &'static str {
                $api
            }
            fn ui_path() -> &'static str {
                $ui
            }
            fn resource_id(&self) -> String {
                self.$id_field.to_string()
            }
        }
    };
}

impl_resource_urlable!(everruns_core::Agent, "v1/agents", "agents", public_id);
impl_resource_urlable!(everruns_core::Harness, "v1/harnesses", "harnesses", id);
impl_resource_urlable!(everruns_core::App, "v1/apps", "apps", public_id);

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

impl ResourceUrlable for everruns_core::Session {
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
}

impl ResourceUrlable for everruns_core::llm_models::LlmProvider {
    fn api_path() -> &'static str {
        "v1/llm-providers"
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

impl ResourceUrlable for everruns_core::llm_models::LlmModel {
    fn api_path() -> &'static str {
        "v1/llm-models"
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

impl ResourceUrlable for everruns_core::LlmModelWithProvider {
    fn api_path() -> &'static str {
        "v1/llm-models"
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

    #[test]
    fn test_error_response_new() {
        let error = ErrorResponse::new("Test error");
        assert_eq!(error.error, "Test error");
    }

    #[test]
    fn test_error_response_into_response() {
        let (status, json) = ErrorResponse::new("Test").into_response(StatusCode::BAD_REQUEST);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.0.error, "Test");
    }

    #[test]
    fn test_error_response_internal_error() {
        let (status, json) = ErrorResponse::internal_error();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json.0.error, "Internal server error");
    }

    #[test]
    fn test_error_response_not_found() {
        let (status, json) = ErrorResponse::not_found("Agent");
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json.0.error, "Agent not found");
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
        assert_eq!(json.0.error, "Internal server error");
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
        assert_eq!(json.0.error, "Agent not found");
    }

    #[test]
    fn test_list_response_new() {
        let list = ListResponse::new(vec![1, 2, 3]);
        assert_eq!(list.data, vec![1, 2, 3]);
    }

    #[test]
    fn test_list_response_from_vec() {
        let list: ListResponse<i32> = vec![1, 2, 3].into();
        assert_eq!(list.data, vec![1, 2, 3]);
    }

    #[test]
    fn test_paginated_response_new() {
        let response = PaginatedResponse::new(vec![1, 2, 3], 100, 0, 20);
        assert_eq!(response.data, vec![1, 2, 3]);
        assert_eq!(response.total, 100);
        assert_eq!(response.offset, 0);
        assert_eq!(response.limit, 20);
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
    fn test_pagination_new() {
        let pagination = Pagination::new(10, 20);
        assert_eq!(pagination.offset, 10);
        assert_eq!(pagination.limit, 20);
    }

    #[test]
    fn test_pagination_default() {
        let pagination = Pagination::default();
        assert_eq!(pagination.offset, 0);
        assert_eq!(pagination.limit, 0);
    }
}
