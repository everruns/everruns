//! The session SQL database store, over the generic command transport.
//!
//! Split out of `grpc_adapters` so that file stops growing: the adapter here is
//! self-contained, and the CRUD half of it is JSON-over-`ExecuteCommand` rather
//! than bespoke RPCs, so it shares nothing with the typed adapters next door but
//! the client.

use crate::grpc_adapters::GrpcAdapter;
use crate::grpc_adapters::{proto_value_to_json, uuid_to_proto};
use async_trait::async_trait;
use everruns_internal_protocol::proto;
use everruns_provider::typed_id::SessionId;

use everruns_platform::session_sqldb::{
    ColumnSchema, DatabaseInfo, SessionSqlDbError, SessionSqlDbStore, SqlExecuteResult,
    SqlQueryResult, TableSchema,
};
/// Alias std::result::Result to avoid shadowing by everruns_provider::error::Result.
type SqlDbResult<T> = std::result::Result<T, SessionSqlDbError>;

/// Convert a gRPC status to a SessionSqlDbError, preserving error semantics.
fn grpc_status_to_sqldb_error(status: tonic::Status) -> SessionSqlDbError {
    let msg = status.message().to_string();
    match status.code() {
        tonic::Code::NotFound => SessionSqlDbError::DatabaseNotFound(msg),
        tonic::Code::AlreadyExists => SessionSqlDbError::DatabaseAlreadyExists(msg),
        tonic::Code::InvalidArgument => SessionSqlDbError::InvalidDatabaseName(msg),
        tonic::Code::ResourceExhausted => SessionSqlDbError::LimitExceeded(msg),
        tonic::Code::DeadlineExceeded => SessionSqlDbError::QueryTimeout(0),
        tonic::Code::PermissionDenied => SessionSqlDbError::AuthorizerBlocked(msg),
        tonic::Code::FailedPrecondition => SessionSqlDbError::QueryError(msg),
        _ => SessionSqlDbError::Internal(msg),
    }
}

impl GrpcAdapter {
    /// `execute_session_command` with both failure channels folded into the
    /// sqldb error vocabulary.
    pub(crate) async fn sqldb_command(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> SqlDbResult<serde_json::Value> {
        match self
            .execute_session_command("Session SQL database", name, params)
            .await
        {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(command_error_to_sqldb_error(error)),
            Err(error) => Err(SessionSqlDbError::Internal(error.to_string())),
        }
    }
}

/// Map a domain command failure onto the sqldb error vocabulary.
///
/// The command transport carries the coarse `CommandError::Kind` set, so this is
/// the lossy step the typed RPCs used to avoid. It is lossless for the CRUD
/// operations that go through it: those only ever raise not-found, conflict,
/// bad-request and internal. `sql_execute`/`sql_query` keep their typed RPC
/// precisely because their errors (query timeout, authorizer blocked) have no
/// `CommandError` kind to survive in.
fn command_error_to_sqldb_error(error: proto::CommandError) -> SessionSqlDbError {
    let message = error.message;
    match proto::command_error::Kind::try_from(error.kind) {
        Ok(proto::command_error::Kind::NotFound) => SessionSqlDbError::DatabaseNotFound(message),
        Ok(proto::command_error::Kind::Conflict) => {
            SessionSqlDbError::DatabaseAlreadyExists(message)
        }
        Ok(proto::command_error::Kind::BadRequest) => {
            SessionSqlDbError::InvalidDatabaseName(message)
        }
        Ok(proto::command_error::Kind::Forbidden) => SessionSqlDbError::AuthorizerBlocked(message),
        _ => SessionSqlDbError::Internal(message),
    }
}

fn parse_command_timestamp(value: &serde_json::Value) -> chrono::DateTime<chrono::Utc> {
    value
        .as_str()
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .map(|stamp| stamp.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now)
}

fn command_db_info_to_core(value: &serde_json::Value) -> DatabaseInfo {
    DatabaseInfo {
        name: value["name"].as_str().unwrap_or_default().to_string(),
        size_bytes: value["size_bytes"].as_i64().unwrap_or_default(),
        page_count: i32::try_from(value["page_count"].as_i64().unwrap_or_default())
            .unwrap_or_default(),
        created_at: parse_command_timestamp(&value["created_at"]),
        updated_at: parse_command_timestamp(&value["updated_at"]),
    }
}

#[async_trait]
impl SessionSqlDbStore for GrpcAdapter {
    async fn create_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> SqlDbResult<DatabaseInfo> {
        let value = self
            .sqldb_command(
                "create_session_database",
                serde_json::json!({ "session_id": session_id.to_string(), "name": name }),
            )
            .await?;
        Ok(command_db_info_to_core(&value))
    }

    async fn list_databases(&self, session_id: SessionId) -> SqlDbResult<Vec<DatabaseInfo>> {
        let value = self
            .sqldb_command(
                "list_session_databases",
                serde_json::json!({ "session_id": session_id.to_string() }),
            )
            .await?;
        Ok(value
            .as_array()
            .map(|items| items.iter().map(command_db_info_to_core).collect())
            .unwrap_or_default())
    }

    async fn get_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> SqlDbResult<Option<DatabaseInfo>> {
        // The command answers a missing database with NotFound; the store
        // contract answers with `None`.
        match self
            .sqldb_command(
                "get_session_database",
                serde_json::json!({ "session_id": session_id.to_string(), "name": name }),
            )
            .await
        {
            Ok(value) => Ok(Some(command_db_info_to_core(&value))),
            Err(SessionSqlDbError::DatabaseNotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn delete_database(&self, session_id: SessionId, name: &str) -> SqlDbResult<bool> {
        let value = self
            .sqldb_command(
                "delete_session_database",
                serde_json::json!({ "session_id": session_id.to_string(), "name": name }),
            )
            .await?;
        Ok(value["deleted"].as_bool().unwrap_or(false))
    }

    async fn sql_execute(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> SqlDbResult<SqlExecuteResult> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionSqlDbExecuteRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            db_name: db_name.to_string(),
            sql: sql.to_string(),
        };
        let response = client
            .session_sql_db_execute(request)
            .await
            .map_err(grpc_status_to_sqldb_error)?;
        Ok(SqlExecuteResult {
            rows_affected: response.into_inner().rows_affected,
        })
    }

    async fn sql_query(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> SqlDbResult<SqlQueryResult> {
        let mut client = self.client.inner.lock().await;
        let request = proto::SessionSqlDbQueryRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            db_name: db_name.to_string(),
            sql: sql.to_string(),
        };
        let response = client
            .session_sql_db_query(request)
            .await
            .map_err(grpc_status_to_sqldb_error)?;
        let inner = response.into_inner();
        let rows: Vec<Vec<serde_json::Value>> = inner
            .rows
            .into_iter()
            .map(|list_value| {
                list_value
                    .values
                    .into_iter()
                    .map(proto_value_to_json)
                    .collect()
            })
            .collect();
        Ok(SqlQueryResult {
            columns: inner.columns,
            rows,
            row_count: inner.row_count as usize,
            truncated: inner.truncated,
        })
    }

    async fn sql_schema(
        &self,
        session_id: SessionId,
        db_name: &str,
        table: Option<&str>,
    ) -> SqlDbResult<Vec<TableSchema>> {
        let value = self
            .sqldb_command(
                "get_session_database_schema",
                serde_json::json!({
                    "session_id": session_id.to_string(),
                    "name": db_name,
                    "table": table,
                }),
            )
            .await?;

        Ok(value["tables"]
            .as_array()
            .map(|tables| {
                tables
                    .iter()
                    .map(|table| TableSchema {
                        name: table["name"].as_str().unwrap_or_default().to_string(),
                        columns: table["columns"]
                            .as_array()
                            .map(|columns| {
                                columns
                                    .iter()
                                    .map(|column| ColumnSchema {
                                        name: column["name"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_string(),
                                        column_type: column["type"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_string(),
                                        notnull: column["notnull"].as_bool().unwrap_or(false),
                                        pk: column["pk"].as_bool().unwrap_or(false),
                                        default_value: column["default_value"]
                                            .as_str()
                                            .map(|text| text.to_string()),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        row_count: table["row_count"].as_i64().unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_status_to_sqldb_error_unmapped_code_falls_to_internal() {
        let status = tonic::Status::unimplemented("not implemented");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::Internal(_)));
    }

    #[test]
    fn command_db_info_decodes_the_command_json_shape() {
        // Pins this against `DatabaseInfoResponse` in
        // `crates/server/src/api/session_databases.rs`: RFC 3339 strings, not
        // proto timestamps. A change there that this misses is a silent
        // now()-for-created_at on every database the agent lists.
        let value = serde_json::json!({
            "name": "test",
            "size_bytes": 4096,
            "page_count": 1,
            "created_at": "2023-11-14T22:13:20Z",
            "updated_at": "2023-11-14T22:13:20Z",
        });

        let info = command_db_info_to_core(&value);

        assert_eq!(info.name, "test");
        assert_eq!(info.size_bytes, 4096);
        assert_eq!(info.page_count, 1);
        assert_eq!(info.created_at.timestamp(), 1700000000);
        assert_eq!(info.updated_at.timestamp(), 1700000000);
    }

    #[test]
    fn command_errors_keep_their_sqldb_meaning() {
        for (kind, matches) in [
            (
                proto::command_error::Kind::NotFound,
                matches!(
                    command_error_to_sqldb_error(proto::CommandError {
                        kind: proto::command_error::Kind::NotFound as i32,
                        message: "missing".into(),
                    }),
                    SessionSqlDbError::DatabaseNotFound(_)
                ),
            ),
            (
                proto::command_error::Kind::Conflict,
                matches!(
                    command_error_to_sqldb_error(proto::CommandError {
                        kind: proto::command_error::Kind::Conflict as i32,
                        message: "exists".into(),
                    }),
                    SessionSqlDbError::DatabaseAlreadyExists(_)
                ),
            ),
            (
                proto::command_error::Kind::BadRequest,
                matches!(
                    command_error_to_sqldb_error(proto::CommandError {
                        kind: proto::command_error::Kind::BadRequest as i32,
                        message: "bad name".into(),
                    }),
                    SessionSqlDbError::InvalidDatabaseName(_)
                ),
            ),
        ] {
            assert!(matches, "{kind:?} lost its sqldb meaning");
        }
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_not_found() {
        let status = tonic::Status::not_found("db not found");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::DatabaseNotFound(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_already_exists() {
        let status = tonic::Status::already_exists("db exists");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::DatabaseAlreadyExists(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_invalid_argument() {
        let status = tonic::Status::invalid_argument("bad name");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::InvalidDatabaseName(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_resource_exhausted() {
        let status = tonic::Status::resource_exhausted("too many");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::LimitExceeded(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_deadline_exceeded() {
        let status = tonic::Status::deadline_exceeded("timeout");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::QueryTimeout(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_permission_denied() {
        let status = tonic::Status::permission_denied("blocked");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::AuthorizerBlocked(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_failed_precondition() {
        let status = tonic::Status::failed_precondition("syntax error");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::QueryError(_)));
    }

    #[test]
    fn test_grpc_status_to_sqldb_error_internal() {
        let status = tonic::Status::internal("unexpected");
        let err = grpc_status_to_sqldb_error(status);
        assert!(matches!(err, SessionSqlDbError::Internal(_)));
    }
}
