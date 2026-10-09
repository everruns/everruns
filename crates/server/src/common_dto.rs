//! Layer-neutral shared DTOs: error body, list wrapper, pagination.
//!
//! Decision: these live outside the HTTP layer so domains, storage and services
//! can use them without importing it; `api::common` re-exports them, so OpenAPI
//! schema names and wire shapes are unchanged. They are not in `records/`
//! because that module is the control-plane record vocabulary that
//! `scripts/check_control_plane_records.py` keeps server-owned.

use crate::storage::UpdateField;
use axum::Json;
use axum::http::StatusCode;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{DeserializeOwned, Error as DeError},
};
use utoipa::ToSchema;
/// Standard error response.
///
/// Wire shape is [RFC 9457 Problem Details](https://www.rfc-editor.org/rfc/rfc9457):
/// every error response includes `title` and `status`, and may include
/// `detail`, `code`, `allowed_actions`, `retry_after_seconds`, `instance`,
/// and `type`. The content type is rewritten to `application/problem+json`
/// with the `application/problem+json` content type.
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
/// `rel` vocabulary distinguishes them.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct AllowedAction {
    /// Link relation describing the action. Examples: `self`, `cancel`, `pause`,
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
