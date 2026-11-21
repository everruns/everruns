//! A session's SQL databases on the data plane (data plane v2 parity, step 9):
//! the `sql_database` tools' databases and the Databases API, in the cell.
//!
//! Decisions:
//! - A session's SQL databases are session state, so they live with the
//!   session (design decision 2A): each database is one SQLite file, stored
//!   in the object store by its SHA-256 like any other blob, and the
//!   session's replicated state holds the index, name to file, changed only
//!   by [`Command::SqlDb`] commits. A database moves and restores with its
//!   session, and the control plane holds none of it:
//!   the `session_sql_db` host call is gone.
//! - A write makes a new file (a new generation); nothing is overwritten,
//!   so a reader never sees a torn database and a cached copy is never
//!   stale. Old generations stay, as workspace blobs do (fs.md, "Deleting
//!   bytes").
//! - The SQLite work runs on a local working copy (`hosting_sqldb.rs`),
//!   with the same executor, authorizer and limits as every other host
//!   (`everruns::database`). A write commits its new file over the one it
//!   started from (compare-and-set on the file's hash), so two writers can
//!   never both win; the loser's copy is dropped and its call fails.
//! - Writers are fenced as workspace writers are: an executor commits under
//!   its epoch; the Databases API commits unfenced, as the gateway.
//! - The index enforces the database count and the session's total size
//!   too, so a working copy that missed another writer's change cannot
//!   grow the session past its limits.
//! - The Databases API (`/v1/sessions/{id}/databases`) is served here, in
//!   the control plane's shapes: list, create, get, delete and schema.
//!
//! [`Command::SqlDb`]: crate::state::Command::SqlDb

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use everruns::database::runtime::limits::{
    MAX_DATABASES_PER_SESSION, MAX_SESSION_TOTAL_SIZE_BYTES, PAGE_SIZE,
};
use everruns::database::runtime::validation::validate_database_name;
use everruns_contracts::error::AgentLoopError;
use everruns_contracts::session_sqldb::{DatabaseInfo, SessionSqlDbError, SessionSqlDbStore as _};
use everruns_contracts::typed_id::SessionId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::{CellReply, reply_of};
use crate::gateway::{Gateway, parse};
use crate::node::Node;

type ApiError = (StatusCode, String);

/// Commits whose replies are kept for retries.
const KEPT_OPS: usize = 64;

/// The session's databases: the index of their current files.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Databases {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entries: BTreeMap<String, Database>,
    /// Recent commits' replies by operation id, oldest first.
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub ops: VecDeque<(String, Value)>,
}

/// One database: its current file in the object store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Database {
    /// The file's SHA-256 (hex), which names its blob.
    pub sha256: String,
    pub size: u64,
    pub created_ms: i64,
    pub updated_ms: i64,
}

impl Database {
    pub fn info(&self, name: &str) -> DatabaseInfo {
        let time = |ms| chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default();
        DatabaseInfo {
            name: name.to_owned(),
            size_bytes: self.size as i64,
            page_count: (self.size / PAGE_SIZE as u64) as i32,
            created_at: time(self.created_ms),
            updated_at: time(self.updated_ms),
        }
    }
}

/// One change to the index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum SqlChange {
    /// Point `name` at a new file. `expected` is the file the writer started
    /// from (`None`: the database must not exist yet).
    Put {
        name: String,
        sha256: String,
        size: u64,
        #[serde(default)]
        expected: Option<String>,
    },
    /// Drop `name` from the index.
    Delete { name: String },
}

/// Why a commit was refused, as its reply says.
pub const CONFLICT: &str = "conflict";
pub const LIMIT: &str = "limit";
pub const INVALID: &str = "invalid";

fn refusal(kind: &str, reason: impl Into<String>) -> Value {
    json!({ "refused": kind, "reason": reason.into() })
}

impl Databases {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.ops.is_empty()
    }

    /// The reply of an earlier commit with this operation id.
    pub fn replay(&self, op: &str) -> Option<Value> {
        self.ops
            .iter()
            .rev()
            .find(|(id, _)| id == op)
            .map(|(_, reply)| reply.clone())
    }

    /// Apply a commit and remember its reply.
    pub fn commit(&mut self, op: String, change: SqlChange, now_ms: i64) -> Value {
        if let Some(reply) = self.replay(&op) {
            return reply;
        }
        let reply = self.apply(change, now_ms);
        self.ops.push_back((op, reply.clone()));
        while self.ops.len() > KEPT_OPS {
            self.ops.pop_front();
        }
        reply
    }

    fn apply(&mut self, change: SqlChange, now_ms: i64) -> Value {
        match change {
            SqlChange::Put {
                name,
                sha256,
                size,
                expected,
            } => {
                if let Err(error) = validate_database_name(&name) {
                    return refusal(INVALID, error.to_string());
                }
                if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return refusal(INVALID, "not a file hash");
                }
                let current = self.entries.get(&name);
                if current.map(|d| &d.sha256) != expected.as_ref() {
                    return refusal(
                        CONFLICT,
                        format!("database changed since it was read: {name}"),
                    );
                }
                if current.is_none() && self.entries.len() >= MAX_DATABASES_PER_SESSION {
                    return refusal(LIMIT, "too many databases");
                }
                let others: u64 = self
                    .entries
                    .iter()
                    .filter(|(n, _)| **n != name)
                    .map(|(_, d)| d.size)
                    .sum();
                if others + size > MAX_SESSION_TOTAL_SIZE_BYTES as u64 {
                    return refusal(LIMIT, "session database storage exhausted");
                }
                let created_ms = current.map_or(now_ms, |d| d.created_ms);
                self.entries.insert(
                    name,
                    Database {
                        sha256,
                        size,
                        created_ms,
                        updated_ms: now_ms,
                    },
                );
                json!({ "changed": true })
            }
            SqlChange::Delete { name } => {
                json!({ "changed": self.entries.remove(&name).is_some() })
            }
        }
    }
}

impl crate::state::SessionState {
    /// Why a write under `fence` (`None`: the gateway's) may not change the
    /// session now, if it may not: an epoch that ended, or a session that
    /// is not live. Workspace and database commits share it.
    pub fn unwritable(&self, fence: Option<u64>, now_ms: i64) -> Option<crate::state::Response> {
        use crate::state::{Placement, Response};
        match fence {
            Some(fence) if !self.authority.current(fence, now_ms) => Some(Response::Fenced),
            None if self.placement != Placement::Live => Some(Response::Frozen),
            _ => None,
        }
    }

    /// Apply [`Command::SqlDb`](crate::state::Command::SqlDb).
    pub fn sqldb_commit(
        &mut self,
        fence: Option<u64>,
        op: String,
        change: SqlChange,
        now_ms: i64,
    ) -> crate::state::Response {
        if let Some(value) = self.sqldb.replay(&op) {
            return crate::state::Response::Value { value };
        }
        if let Some(refused) = self.unwritable(fence, now_ms) {
            return refused;
        }
        let value = self.sqldb.commit(op, change, now_ms);
        crate::state::Response::Value { value }
    }
}

// ---------------------------------------------------------------------------
// The Databases API: a node's side, then the gateway's.

pub fn router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/cell/sessions/{session}/sqldb", post(sqldb))
        // An import carries a whole database; the cell is a trusted
        // network, and the gateway bounds what reaches it.
        .layer(axum::extract::DefaultBodyLimit::disable())
        .with_state(node)
}

/// One Databases API call, as the gateway sends it.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SqlOp {
    List,
    Get {
        name: String,
    },
    Create {
        name: String,
    },
    Delete {
        name: String,
    },
    Schema {
        name: String,
    },
    /// A moved v1 session's database, whole: `file` is the SQLite file,
    /// base64 encoded. Only the control plane sends it (`sqldb` session
    /// command), while it imports the session.
    Import {
        name: String,
        file: String,
    },
}

/// What a call came to, inside [`CellReply::Ok`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum SqlResult {
    Done { value: Value },
    Failed { status: u16, message: String },
}

fn failed(status: StatusCode, message: impl Into<String>) -> SqlResult {
    SqlResult::Failed {
        status: status.as_u16(),
        message: message.into(),
    }
}

/// A database as the control plane's API shows it.
pub fn shown(info: &DatabaseInfo) -> Value {
    json!({
        "name": info.name,
        "size_bytes": info.size_bytes,
        "page_count": info.page_count,
        "created_at": info.created_at.to_rfc3339(),
        "updated_at": info.updated_at.to_rfc3339(),
    })
}

/// The status the control plane's API answers for a store error.
fn status_of(error: &SessionSqlDbError) -> StatusCode {
    match error {
        SessionSqlDbError::DatabaseNotFound(_) => StatusCode::NOT_FOUND,
        SessionSqlDbError::DatabaseAlreadyExists(_) => StatusCode::CONFLICT,
        SessionSqlDbError::InvalidDatabaseName(_) => StatusCode::BAD_REQUEST,
        SessionSqlDbError::LimitExceeded(_) => StatusCode::UNPROCESSABLE_ENTITY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn sqldb(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(op): Json<SqlOp>,
) -> Result<Json<CellReply>, ApiError> {
    let id = SessionId::parse(&session).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    // Reads are linearizable, as the workspace API's are (`api_files.rs`).
    crate::api_files::read_barrier(&node)
        .await
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
    let store = node.hosting.databases_api();
    let reply = match run(&store, id, op).await {
        Ok(result) => CellReply::Ok {
            value: serde_json::to_value(result).unwrap_or(Value::Null),
        },
        Err(error) => match error {
            AgentLoopError::SessionNotFound(_) => CellReply::NotFound,
            other => match reply_of(&other) {
                CellReply::Refused { reason } => CellReply::Ok {
                    value: serde_json::to_value(failed(StatusCode::SERVICE_UNAVAILABLE, reason))
                        .unwrap_or(Value::Null),
                },
                CellReply::NotFound => CellReply::Ok {
                    value: serde_json::to_value(failed(
                        StatusCode::SERVICE_UNAVAILABLE,
                        other.to_string(),
                    ))
                    .unwrap_or(Value::Null),
                },
                reply => reply,
            },
        },
    };
    Ok(Json(reply))
}

async fn run(
    store: &crate::hosting::sqldb::CellSqlDb,
    session: SessionId,
    op: SqlOp,
) -> everruns_contracts::error::Result<SqlResult> {
    // The session's whereabouts first: a missing or moved session answers
    // as one, never as a missing database.
    let entries = store.index(session).await?;
    let outcome = match op {
        SqlOp::List => Ok(json!({
            "data": entries.iter().map(|(n, d)| shown(&d.info(n))).collect::<Vec<_>>(),
        })),
        SqlOp::Get { name } => match entries.get(&name) {
            Some(database) => Ok(shown(&database.info(&name))),
            None => return Ok(failed(StatusCode::NOT_FOUND, "Database not found")),
        },
        SqlOp::Create { name } => store
            .create_database(session, &name)
            .await
            .map(|info| shown(&info)),
        SqlOp::Delete { name } => match store.delete_database(session, &name).await {
            Ok(true) => Ok(json!({ "deleted": true })),
            Ok(false) => return Ok(failed(StatusCode::NOT_FOUND, "Database not found")),
            Err(error) => Err(error),
        },
        SqlOp::Schema { name } => store
            .sql_schema(session, &name, None)
            .await
            .map(|tables| json!({ "database": name, "tables": tables })),
        SqlOp::Import { name, file } => {
            use base64::Engine as _;
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(file) else {
                return Ok(failed(StatusCode::BAD_REQUEST, "file: not base64"));
            };
            store
                .import(session, &name, bytes)
                .await
                .map(|info| shown(&info))
        }
    };
    Ok(match outcome {
        Ok(value) => SqlResult::Done { value },
        Err(error) => failed(status_of(&error), error.to_string()),
    })
}

pub fn gateway_routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route(
            "/v1/sessions/{session}/databases",
            get(list_databases).post(create_database),
        )
        .route(
            "/v1/sessions/{session}/databases/{name}",
            get(get_database).delete(delete_database),
        )
        .route(
            "/v1/sessions/{session}/databases/{name}/schema",
            get(get_schema),
        )
}

/// Run `op` on the session's cell.
async fn call(gateway: &Gateway, session: &str, op: SqlOp) -> Result<Value, ApiError> {
    let session = parse(session)?;
    let body = serde_json::to_value(&op)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let value = gateway.session_command(session, "sqldb", &body).await?;
    match serde_json::from_value::<SqlResult>(value) {
        Ok(SqlResult::Done { value }) => Ok(value),
        Ok(SqlResult::Failed { status, message }) => Err((
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            message,
        )),
        Err(error) => Err((StatusCode::BAD_GATEWAY, error.to_string())),
    }
}

async fn list_databases(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(call(&gateway, &session, SqlOp::List).await?))
}

#[derive(Deserialize)]
struct CreateBody {
    name: String,
}

async fn create_database(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let op = SqlOp::Create { name: body.name };
    Ok((
        StatusCode::CREATED,
        Json(call(&gateway, &session, op).await?),
    ))
}

async fn get_database(
    State(gateway): State<Arc<Gateway>>,
    Path((session, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(call(&gateway, &session, SqlOp::Get { name }).await?))
}

async fn delete_database(
    State(gateway): State<Arc<Gateway>>,
    Path((session, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    call(&gateway, &session, SqlOp::Delete { name }).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_schema(
    State(gateway): State<Arc<Gateway>>,
    Path((session, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        call(&gateway, &session, SqlOp::Schema { name }).await?,
    ))
}

#[cfg(test)]
#[path = "sqldb_tests.rs"]
mod tests;
