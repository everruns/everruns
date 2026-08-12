// Session SQL Database HTTP routes
//
// CRUD for session-scoped databases + schema introspection.
// Routes under /v1/sessions/{session_id}/databases.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::session_databases::{
    CreateSessionDatabaseCmd, DeleteSessionDatabase, GetSessionDatabase, GetSessionDatabaseSchema,
    ListSessionDatabases,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::Caller;
use everruns_core::typed_id::SessionId;
use everruns_platform::session_sqldb::{DatabaseInfo, SessionSqlDbStore};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use crate::storage::StorageBackend;

use super::common::{ListResponse, impl_auth_state};

/// Request body for creating a database.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateDatabaseRequest {
    /// Database name (alphanumeric + underscores, max 64 chars).
    #[schema(example = "refund_history")]
    pub name: String,
}

/// Database info response.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DatabaseInfoResponse {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    pub size_bytes: i64,
    pub page_count: i32,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: String,
    /// Timestamp when this resource was last updated (RFC 3339).
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

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_sqldb_store(self.sqldb_store.clone())
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
    let items = ListSessionDatabases {
        session_id: session_id.to_string(),
    }
    .run(&state.ctx(&org))
    .await
    .map_err(|e| e.status())?;
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
        (status = 422, description = "Session database limit exceeded"),
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
    let info = CreateSessionDatabaseCmd {
        session_id: session_id.to_string(),
        name: req.name,
    }
    .run(&state.ctx(&org))
    .await
    .map_err(|e| e.status())?;

    Ok((StatusCode::CREATED, Json(info)))
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
    Ok(Json(
        GetSessionDatabase {
            session_id: session_id.to_string(),
            name,
        }
        .run(&state.ctx(&org))
        .await
        .map_err(|e| e.status())?,
    ))
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
    DeleteSessionDatabase {
        session_id: session_id.to_string(),
        name,
    }
    .run(&state.ctx(&org))
    .await
    .map_err(|e| e.status())?;
    Ok(StatusCode::NO_CONTENT)
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
    Ok(Json(
        GetSessionDatabaseSchema {
            session_id: session_id.to_string(),
            name,
        }
        .run(&state.ctx(&org))
        .await
        .map_err(|e| e.status())?,
    ))
}

#[cfg(test)]
mod tests {
    // Trivial derive-only serde round-trips removed; covered by the derive + handler tests.
}
