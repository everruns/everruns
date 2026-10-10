use super::queries as q;
use super::types::{DatabaseInfoResponse, SchemaResponse};
use crate::domains::common::*;
use everruns_contracts::session_sqldb::SessionSqlDbError;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

fn sqldb_store(
    ctx: &Ctx,
) -> Result<&std::sync::Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore>, CommandError>
{
    ctx.sqldb_store.as_ref().ok_or_else(|| {
        CommandError::internal(anyhow::anyhow!("Session SQL DB store not configured"))
    })
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListSessionDatabases {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "list_session_databases",
    category = "session_databases",
    description = "List all SQL databases created inside a session.",
    method = "GET",
    cli = CliRoute::new(&["sessions", "databases"], "list").with_args(&[CliArg::new("session_id").at(1)]).with_examples(&[CliExample::new("See which databases a session has created", "everruns sessions databases list session_01h9")]),
    path = "/v1/sessions/{session_id}/databases",
    positional = "session_id"
)]
impl Command for ListSessionDatabases {
    type Output = Vec<DatabaseInfoResponse>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<DatabaseInfoResponse>, CommandError> {
        let session_id = q::parse_owned_session_id(&self.session_id)?;
        q::verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;

        let items = sqldb_store(ctx)?
            .list_databases(session_id)
            .await
            .map_err(|e| CommandError::internal(e.into()))?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(items)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateSessionDatabaseCmd {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
}

#[command(
    name = "create_session_database",
    category = "session_databases",
    description = "Create a new SQL database inside a session.",
    method = "POST",
    cli = CliRoute::new(&["sessions", "databases"], "create").with_examples(&[CliExample::new("Give a session a scratch SQL database to work with", "everruns sessions databases create --session-id session_01h9 --name analytics --reason 'Store intermediate results'")]),
    path = "/v1/sessions/{session_id}/databases",
    policy = crate::domains::sessions::SESSION_MANAGE,
)]
impl Command for CreateSessionDatabaseCmd {
    type Output = DatabaseInfoResponse;

    async fn execute(self, ctx: &Ctx) -> Result<DatabaseInfoResponse, CommandError> {
        let session_id = q::parse_owned_session_id(&self.session_id)?;
        q::verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;

        let info = sqldb_store(ctx)?
            .create_database(session_id, &self.name)
            .await
            .map_err(|e| match e {
                SessionSqlDbError::InvalidDatabaseName(_) => {
                    CommandError::bad_request(e.to_string())
                }
                SessionSqlDbError::LimitExceeded(_) => CommandError::unprocessable(e.to_string()),
                SessionSqlDbError::DatabaseAlreadyExists(_) => {
                    CommandError::conflict(e.to_string())
                }
                other => CommandError::internal(other.into()),
            })?;

        Ok(info.into())
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetSessionDatabase {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
}

#[command(
    name = "get_session_database",
    category = "session_databases",
    description = "Get metadata for a session SQL database.",
    method = "GET",
    cli = CliRoute::new(&["sessions", "databases"], "get").with_examples(&[CliExample::new("Check a session database's size and metadata", "everruns sessions databases get --session-id session_01h9 --name analytics")]),
    path = "/v1/sessions/{session_id}/databases/{name}"
)]
impl Command for GetSessionDatabase {
    type Output = DatabaseInfoResponse;

    async fn execute(self, ctx: &Ctx) -> Result<DatabaseInfoResponse, CommandError> {
        let session_id = q::parse_owned_session_id(&self.session_id)?;
        q::verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;

        sqldb_store(ctx)?
            .get_database(session_id, &self.name)
            .await
            .map_err(|e| CommandError::internal(e.into()))?
            .map(Into::into)
            .ok_or_else(|| CommandError::not_found("Database"))
    }
}

#[derive(Debug, Serialize)]
pub struct DeleteSessionDatabaseResult {
    pub deleted: bool,
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteSessionDatabase {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
}

#[command(
    name = "delete_session_database",
    category = "session_databases",
    description = "Delete a session SQL database.",
    method = "DELETE",
    cli = CliRoute::new(&["sessions", "databases"], "delete").with_examples(&[CliExample::new("Drop a session database once its data is exported", "everruns sessions databases delete --session-id session_01h9 --name analytics --reason 'No longer needed'")]),
    path = "/v1/sessions/{session_id}/databases/{name}",
    policy = crate::domains::sessions::SESSION_MANAGE,
)]
impl Command for DeleteSessionDatabase {
    type Output = DeleteSessionDatabaseResult;

    async fn execute(self, ctx: &Ctx) -> Result<DeleteSessionDatabaseResult, CommandError> {
        let session_id = q::parse_owned_session_id(&self.session_id)?;
        q::verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;

        let deleted = sqldb_store(ctx)?
            .delete_database(session_id, &self.name)
            .await
            .map_err(|e| CommandError::internal(e.into()))?;
        if !deleted {
            return Err(CommandError::not_found("Database"));
        }
        Ok(DeleteSessionDatabaseResult { deleted })
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetSessionDatabaseSchema {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
}

#[command(
    name = "get_session_database_schema",
    category = "session_databases",
    description = "Inspect the schema of a session SQL database.",
    method = "GET",
    cli = CliRoute::new(&["sessions", "databases", "schema"], "get").with_examples(&[CliExample::new("Inspect a session database's tables before writing queries", "everruns sessions databases schema get --session-id session_01h9 --name analytics")]),
    path = "/v1/sessions/{session_id}/databases/{name}/schema"
)]
impl Command for GetSessionDatabaseSchema {
    type Output = SchemaResponse;

    async fn execute(self, ctx: &Ctx) -> Result<SchemaResponse, CommandError> {
        let session_id = q::parse_owned_session_id(&self.session_id)?;
        q::verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;

        let tables = sqldb_store(ctx)?
            .sql_schema(session_id, &self.name, None)
            .await
            .map_err(|e| match e {
                SessionSqlDbError::DatabaseNotFound(_) => CommandError::not_found("Database"),
                other => CommandError::internal(other.into()),
            })?;

        Ok(SchemaResponse {
            database: self.name,
            tables: tables
                .into_iter()
                .map(|table| {
                    serde_json::json!({
                        "name": table.name,
                        "columns": table.columns.into_iter().map(|column| serde_json::json!({
                            "name": column.name,
                            "type": column.column_type,
                            "notnull": column.notnull,
                            "pk": column.pk,
                            "default_value": column.default_value
                        })).collect::<Vec<_>>(),
                        "row_count": table.row_count,
                    })
                })
                .collect(),
        })
    }
}
