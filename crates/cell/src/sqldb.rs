//! A session's SQL databases on the data plane (data plane v2, step 11.6b):
//! the `sql_database` tools' store.
//!
//! Decisions:
//! - The databases stay in the control plane's store, as for a v1 session,
//!   so the console's Databases panel reads the same ones a cell session's
//!   tools write. Each operation goes over the lookups channel
//!   ([`SESSION_SQL_DB`], `lookups.rs`); the control plane checks the
//!   session is the org's before it touches the store.
//! - The store is bound to one session: a call naming another session is
//!   refused here, before it leaves the executor.
//! - Errors keep their kind (a timeout, a blocked statement, a missing
//!   database) so the tools tell the model what they tell it on v1; a
//!   lookup that cannot reach the control plane is internal.

use async_trait::async_trait;
use everruns_contracts::host_calls::SESSION_SQL_DB;
use everruns_contracts::session_sqldb::{
    DatabaseInfo, SessionSqlDbError, SessionSqlDbStore, SqlExecuteResult, SqlQueryResult,
    TableSchema,
};
use everruns_contracts::typed_id::SessionId;
use everruns_internal_protocol::commands::InternalCommandTransport;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::lookups::OrgLookups;

type SqlResult<T> = std::result::Result<T, SessionSqlDbError>;

/// One session's SQL databases, in the control plane.
pub(crate) struct CellSqlDb {
    lookups: OrgLookups,
    session: SessionId,
}

impl CellSqlDb {
    pub(crate) fn new(lookups: OrgLookups, session: SessionId) -> Self {
        Self { lookups, session }
    }

    /// Run `op` on `session`'s databases with `args`; its answer as `T`.
    async fn call<T: DeserializeOwned>(
        &self,
        session: SessionId,
        op: &str,
        args: Value,
    ) -> SqlResult<T> {
        if session != self.session {
            return Err(SessionSqlDbError::Internal(
                "a session's tools reach only its own databases".into(),
            ));
        }
        let mut params = json!({ "session_id": session.to_string(), "op": op });
        if let (Some(params), Value::Object(args)) = (params.as_object_mut(), args) {
            params.extend(args);
        }
        let answer = match self
            .lookups
            .execute_internal_command(SESSION_SQL_DB, params)
            .await
        {
            Ok(Ok(answer)) => answer,
            Ok(Err(error)) => return Err(SessionSqlDbError::Internal(error.message)),
            Err(error) => return Err(SessionSqlDbError::Internal(error.to_string())),
        };
        let internal = |error: serde_json::Error| SessionSqlDbError::Internal(error.to_string());
        if let Some(error) = answer.get("error") {
            return Err(serde_json::from_value(error.clone()).map_err(internal)?);
        }
        serde_json::from_value(answer["ok"].clone()).map_err(internal)
    }
}

#[async_trait]
impl SessionSqlDbStore for CellSqlDb {
    async fn create_database(&self, session_id: SessionId, name: &str) -> SqlResult<DatabaseInfo> {
        self.call(session_id, "create", json!({ "name": name }))
            .await
    }

    async fn list_databases(&self, session_id: SessionId) -> SqlResult<Vec<DatabaseInfo>> {
        self.call(session_id, "list", json!({})).await
    }

    async fn get_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> SqlResult<Option<DatabaseInfo>> {
        self.call(session_id, "get", json!({ "name": name })).await
    }

    async fn delete_database(&self, session_id: SessionId, name: &str) -> SqlResult<bool> {
        self.call(session_id, "delete", json!({ "name": name }))
            .await
    }

    async fn sql_execute(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> SqlResult<SqlExecuteResult> {
        self.call(
            session_id,
            "execute",
            json!({ "name": db_name, "sql": sql }),
        )
        .await
    }

    async fn sql_query(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> SqlResult<SqlQueryResult> {
        self.call(session_id, "query", json!({ "name": db_name, "sql": sql }))
            .await
    }

    async fn sql_schema(
        &self,
        session_id: SessionId,
        db_name: &str,
        table: Option<&str>,
    ) -> SqlResult<Vec<TableSchema>> {
        self.call(
            session_id,
            "schema",
            json!({ "name": db_name, "table": table }),
        )
        .await
    }
}
