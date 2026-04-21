// Session Storage HTTP routes
// Routes for listing session key-value storage and secrets
// Secrets are returned without their values for security

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::session_storage::{
    BatchSetSessionSecrets, ListSessionSecrets, ListSessionStorage,
};
use crate::storage::StorageBackend;
use crate::storage::encryption::EncryptionService;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::{Caller, SessionId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::{ApiResult, ErrorResponse, ListResponse, impl_auth_state};

/// Key-value entry info (key and timestamps, no value)
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct KeyValueInfo {
    /// The key name
    pub key: String,
    /// The stored value
    pub value: String,
    /// When the key was created
    pub created_at: String,
    /// When the key was last updated
    pub updated_at: String,
}

/// Secret entry info (name and timestamps only, no value)
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SecretInfo {
    /// The secret name
    pub name: String,
    /// When the secret was created
    pub created_at: String,
    /// When the secret was last updated
    pub updated_at: String,
}

/// Batch secret set request
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchSetSecretsRequest {
    /// Map of secret names to values
    pub secrets: HashMap<String, String>,
}

/// Batch secret set response
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchSetSecretsResponse {
    /// Number of secrets stored
    pub count: usize,
}

/// App state for session storage routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            encryption,
            auth,
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(Caller::from(org), self.db.clone(), self.encryption.clone())
    }
}

impl_auth_state!(AppState);

/// Create session storage routes (nested under sessions)
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/sessions/{session_id}/storage/keys", get(list_keys))
        .route(
            "/v1/sessions/{session_id}/storage/secrets",
            get(list_secrets).put(batch_set_secrets),
        )
        .with_state(state)
}

/// GET /v1/sessions/{session_id}/storage/keys - List all key-value pairs
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/storage/keys",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "List of key-value pairs", body = ListResponse<KeyValueInfo>),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "session-storage"
)]
pub async fn list_keys(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<ListResponse<KeyValueInfo>>, StatusCode> {
    let session_id: SessionId = session_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let items = ListSessionStorage {
        session_id: session_id.to_string(),
    }
    .execute(&state.ctx(&org))
    .await
    .map_err(|e| e.status())?;
    Ok(Json(ListResponse::new(items)))
}

/// GET /v1/sessions/{session_id}/storage/secrets - List all secrets (names only)
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/storage/secrets",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "List of secrets (names only, no values)", body = ListResponse<SecretInfo>),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "session-storage"
)]
pub async fn list_secrets(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<ListResponse<SecretInfo>>, StatusCode> {
    let session_id: SessionId = session_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let items = ListSessionSecrets {
        session_id: session_id.to_string(),
    }
    .execute(&state.ctx(&org))
    .await
    .map_err(|e| e.status())?;
    Ok(Json(ListResponse::new(items)))
}

/// PUT /v1/sessions/{session_id}/storage/secrets - Batch set secrets
///
/// Encrypts and stores multiple secrets in a single request.
/// Existing secrets with the same name are overwritten.
#[utoipa::path(
    put,
    path = "/v1/sessions/{session_id}/storage/secrets",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = BatchSetSecretsRequest,
    responses(
        (status = 200, description = "Secrets stored", body = BatchSetSecretsResponse),
        (status = 400, description = "Bad request (encryption not configured or invalid input)"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "session-storage"
)]
pub async fn batch_set_secrets(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<BatchSetSecretsRequest>,
) -> ApiResult<BatchSetSecretsResponse> {
    let session_id: SessionId = session_id.parse().map_err(|_| {
        ErrorResponse::new("Invalid session ID").into_response(StatusCode::BAD_REQUEST)
    })?;
    Ok(Json(
        BatchSetSessionSecrets {
            session_id: session_id.to_string(),
            secrets: body.secrets,
        }
        .execute(&state.ctx(&org))
        .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_value_info_serialization() {
        let info = KeyValueInfo {
            key: "test_key".to_string(),
            value: "test_value".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("test_key"));
        assert!(json.contains("test_value"));
    }

    #[test]
    fn test_secret_info_serialization() {
        let info = SecretInfo {
            name: "api_key".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("api_key"));
        // Ensure no value field exists
        assert!(!json.contains("value"));
    }

    #[test]
    fn test_batch_set_secrets_request_deserialization() {
        let json = r#"{"secrets":{"KEY1":"value1","KEY2":"value2"}}"#;
        let req: BatchSetSecretsRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.secrets.len(), 2);
        assert_eq!(req.secrets["KEY1"], "value1");
        assert_eq!(req.secrets["KEY2"], "value2");
    }

    #[test]
    fn test_batch_set_secrets_response_serialization() {
        let resp = BatchSetSecretsResponse { count: 3 };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"count\":3"));
    }

    #[test]
    fn test_batch_set_secrets_request_empty() {
        let json = r#"{"secrets":{}}"#;
        let req: BatchSetSecretsRequest = serde_json::from_str(json).unwrap();
        assert!(req.secrets.is_empty());
    }
}
