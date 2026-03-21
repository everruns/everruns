// Session SQL Database HTTP routes
//
// CRUD for session-scoped databases + schema introspection.
// Routes under /v1/sessions/{session_id}/databases.

use crate::auth::{AuthState, ResolvedOrg};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::session_sqldb::{DatabaseInfo, SessionSqlDbStore};
use everruns_core::typed_id::SessionId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use crate::storage::StorageBackend;

use super::common::{ListResponse, impl_auth_state, verify_session_ownership};

/// Request body for creating a database.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateDatabaseRequest {
    /// Database name (alphanumeric + underscores, max 64 chars)
    pub name: String,
}

/// Database info response.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DatabaseInfoResponse {
    pub name: String,
    pub size_bytes: i64,
    pub page_count: i32,
    pub created_at: String,
    pub updated_at: String,
}

impl From<DatabaseInfo> for DatabaseInfoResponse {
    fn from(info: DatabaseInfo) -> Self {
        Self {
            name: info.name,
            size_bytes: info.size_bytes,
            page_count: info.page_count,
            created_at: info.created_at.to_rfc3339(),
            updated_at: info.updated_at.to_rfc3339(),
        }
    }
}

/// Schema response for a database.
#[derive(Debug, Serialize, ToSchema)]
pub struct SchemaResponse {
    pub database: String,
    pub tables: Vec<serde_json::Value>,
}

/// App state for session database routes.
#[derive(Clone)]
pub struct AppState {
    pub sqldb_store: Arc<dyn SessionSqlDbStore>,
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        sqldb_store: Arc<dyn SessionSqlDbStore>,
        db: Arc<StorageBackend>,
        auth: AuthState,
    ) -> Self {
        Self {
            sqldb_store,
            db,
            auth,
        }
    }
}

impl_auth_state!(AppState);

/// Create session database routes.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/sessions/{session_id}/databases",
            get(list_databases).post(create_database),
        )
        .route(
            "/v1/sessions/{session_id}/databases/{name}",
            get(get_database).delete(delete_database),
        )
        .route(
            "/v1/sessions/{session_id}/databases/{name}/schema",
            get(get_schema),
        )
        .with_state(state)
}

/// GET /v1/sessions/{session_id}/databases
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/databases",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "List of databases", body = ListResponse<DatabaseInfoResponse>),
        (status = 400, description = "Invalid session ID"),
    ),
    tag = "session-databases"
)]
pub async fn list_databases(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<ListResponse<DatabaseInfoResponse>>, StatusCode> {
    let session_id: SessionId = session_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    verify_session_ownership(&state.db, org.org_id, session_id).await?;

    let databases = state
        .sqldb_store
        .list_databases(session_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to list databases");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let items: Vec<DatabaseInfoResponse> = databases.into_iter().map(Into::into).collect();
    Ok(Json(ListResponse::new(items)))
}

/// POST /v1/sessions/{session_id}/databases
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/databases",
    params(("session_id" = String, Path, description = "Session ID")),
    request_body = CreateDatabaseRequest,
    responses(
        (status = 201, description = "Database created", body = DatabaseInfoResponse),
        (status = 400, description = "Invalid name or session ID"),
        (status = 409, description = "Database already exists"),
    ),
    tag = "session-databases"
)]
pub async fn create_database(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<CreateDatabaseRequest>,
) -> Result<(StatusCode, Json<DatabaseInfoResponse>), StatusCode> {
    let session_id: SessionId = session_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    verify_session_ownership(&state.db, org.org_id, session_id).await?;

    let info = state
        .sqldb_store
        .create_database(session_id, &req.name)
        .await
        .map_err(|e| {
            use everruns_core::session_sqldb::SessionSqlDbError;
            match &e {
                SessionSqlDbError::InvalidDatabaseName(_) => StatusCode::BAD_REQUEST,
                SessionSqlDbError::DatabaseAlreadyExists(_) => StatusCode::CONFLICT,
                SessionSqlDbError::LimitExceeded(_) => StatusCode::UNPROCESSABLE_ENTITY,
                _ => {
                    tracing::error!(error = %e, "Failed to create database");
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
        })?;

    Ok((StatusCode::CREATED, Json(info.into())))
}

/// GET /v1/sessions/{session_id}/databases/{name}
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/databases/{name}",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("name" = String, Path, description = "Database name"),
    ),
    responses(
        (status = 200, description = "Database info", body = DatabaseInfoResponse),
        (status = 404, description = "Database not found"),
    ),
    tag = "session-databases"
)]
pub async fn get_database(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, name)): Path<(String, String)>,
) -> Result<Json<DatabaseInfoResponse>, StatusCode> {
    let session_id: SessionId = session_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    verify_session_ownership(&state.db, org.org_id, session_id).await?;

    let info = state
        .sqldb_store
        .get_database(session_id, &name)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to get database");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(info.into()))
}

/// DELETE /v1/sessions/{session_id}/databases/{name}
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/databases/{name}",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("name" = String, Path, description = "Database name"),
    ),
    responses(
        (status = 204, description = "Database deleted"),
        (status = 404, description = "Database not found"),
    ),
    tag = "session-databases"
)]
pub async fn delete_database(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, name)): Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    let session_id: SessionId = session_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    verify_session_ownership(&state.db, org.org_id, session_id).await?;

    let deleted = state
        .sqldb_store
        .delete_database(session_id, &name)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to delete database");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// GET /v1/sessions/{session_id}/databases/{name}/schema
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/databases/{name}/schema",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("name" = String, Path, description = "Database name"),
    ),
    responses(
        (status = 200, description = "Database schema", body = SchemaResponse),
        (status = 404, description = "Database not found"),
    ),
    tag = "session-databases"
)]
pub async fn get_schema(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, name)): Path<(String, String)>,
) -> Result<Json<SchemaResponse>, StatusCode> {
    let session_id: SessionId = session_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    verify_session_ownership(&state.db, org.org_id, session_id).await?;

    let tables = state
        .sqldb_store
        .sql_schema(session_id, &name, None)
        .await
        .map_err(|e| {
            use everruns_core::session_sqldb::SessionSqlDbError;
            match &e {
                SessionSqlDbError::DatabaseNotFound(_) => StatusCode::NOT_FOUND,
                _ => {
                    tracing::error!(error = %e, "Failed to get schema");
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
        })?;

    let tables_json: Vec<serde_json::Value> = tables
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "columns": t.columns.into_iter().map(|c| serde_json::json!({
                    "name": c.name,
                    "type": c.column_type,
                    "notnull": c.notnull,
                    "pk": c.pk,
                    "default_value": c.default_value
                })).collect::<Vec<_>>(),
                "row_count": t.row_count
            })
        })
        .collect();

    Ok(Json(SchemaResponse {
        database: name,
        tables: tables_json,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_info_response_serialization() {
        let info = DatabaseInfoResponse {
            name: "test_db".to_string(),
            size_bytes: 4096,
            page_count: 1,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("test_db"));
        assert!(json.contains("4096"));
    }

    #[test]
    fn test_create_request_deserialization() {
        let json = r#"{"name": "analytics"}"#;
        let req: CreateDatabaseRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "analytics");
    }
}
