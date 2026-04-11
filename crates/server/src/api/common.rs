// Common DTOs for public API
//
// These types are shared across multiple API endpoints.
// ApiResult: standard return type for API handlers
// impl_auth_state!: macro to eliminate repeated FromRef<AppState> for AuthState impls

use axum::Json;
use axum::http::StatusCode;
use everruns_core::typed_id::SessionId;
use everruns_durable::UpdateField;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{DeserializeOwned, Error as DeError},
};
use std::sync::Arc;
use utoipa::ToSchema;

use crate::storage::StorageBackend;

/// Standard return type for API handlers that return JSON with error responses.
///
/// Replaces the repeated pattern:
/// ```ignore
/// Result<Json<T>, (StatusCode, Json<ErrorResponse>)>
/// ```
pub type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

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

    /// Wrap a single resource with `url` and `view_url`.
    pub fn wrap<T: ResourceUrlable + Serialize>(&self, item: T) -> WithUrls<T> {
        let id = item.resource_id();
        let api_path = T::api_path();
        let ui_path = T::ui_path();
        WithUrls {
            url: format!("{}/{}/{}", self.api_base, api_path, id),
            view_url: format!("{}/{}/{}", self.ui_base, ui_path, id),
            inner: item,
        }
    }

    /// Wrap a vec of resources.
    pub fn wrap_vec<T: ResourceUrlable + Serialize>(&self, items: Vec<T>) -> Vec<WithUrls<T>> {
        items.into_iter().map(|item| self.wrap(item)).collect()
    }
}

/// Trait for resources that can have `url` and `view_url` generated.
pub trait ResourceUrlable {
    /// API path segment (e.g. `"v1/agents"`).
    fn api_path() -> &'static str;
    /// UI path segment (e.g. `"agents"`).
    fn ui_path() -> &'static str;
    /// The resource's public ID as a string.
    fn resource_id(&self) -> String;
}

/// Wrapper that adds `url` and `view_url` to a serialized resource.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WithUrls<T: Serialize> {
    /// Full API endpoint URL for this resource.
    pub url: String,
    /// Full UI URL for viewing this resource.
    pub view_url: String,
    /// The resource itself (fields are flattened into the parent object).
    #[serde(flatten)]
    pub inner: T,
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
impl_resource_urlable!(everruns_core::Session, "v1/sessions", "sessions", id);
impl_resource_urlable!(everruns_core::Harness, "v1/harnesses", "harnesses", id);
impl_resource_urlable!(everruns_core::Skill, "v1/skills", "skills", id);
impl_resource_urlable!(everruns_core::budget::Budget, "v1/budgets", "budgets", id);
impl_resource_urlable!(
    everruns_core::McpServer,
    "v1/mcp-servers",
    "mcp-servers",
    id
);
impl_resource_urlable!(everruns_core::App, "v1/apps", "apps", public_id);
impl_resource_urlable!(
    everruns_core::llm_models::LlmProvider,
    "v1/llm-providers",
    "llm-providers",
    id
);
impl_resource_urlable!(
    everruns_core::llm_models::LlmModel,
    "v1/llm-models",
    "llm-models",
    id
);
impl_resource_urlable!(
    everruns_core::LlmModelWithProvider,
    "v1/llm-models",
    "llm-models",
    id
);
impl_resource_urlable!(
    everruns_core::session_schedule::SessionSchedule,
    "v1/schedules",
    "durable/schedules",
    id
);

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
