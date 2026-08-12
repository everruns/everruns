use super::queries as q;
use super::types::{DatabaseInfoResponse, SchemaResponse};
use crate::domains::common::*;
use everruns_platform::session_sqldb::SessionSqlDbError;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

fn sqldb_store(
    ctx: &Ctx,
) -> Result<&std::sync::Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore>, CommandError>
{
    ctx.sqldb_store.as_ref().ok_or_else(|| {
        CommandError::internal(anyhow::anyhow!("Session SQL DB store not configured"))
    })
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSessionDatabases {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for ListSessionDatabases {
    type Output = Vec<DatabaseInfoResponse>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_session_databases",
            category: "session_databases",
            description: "List all SQL databases created inside a session.",
            method: "GET",
            path: "/v1/sessions/{session_id}/databases",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

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

inventory::submit! { CommandDescriptor::of::<ListSessionDatabases>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSessionDatabaseCmd {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
}

impl Command for CreateSessionDatabaseCmd {
    type Output = DatabaseInfoResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_session_database",
            category: "session_databases",
            description: "Create a new SQL database inside a session.",
            method: "POST",
            path: "/v1/sessions/{session_id}/databases",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_MANAGE)
    }

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

inventory::submit! { CommandDescriptor::of::<CreateSessionDatabaseCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetSessionDatabase {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
}

impl Command for GetSessionDatabase {
    type Output = DatabaseInfoResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session_database",
            category: "session_databases",
            description: "Get metadata for a session SQL database.",
            method: "GET",
            path: "/v1/sessions/{session_id}/databases/{name}",
        }
    }

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

inventory::submit! { CommandDescriptor::of::<GetSessionDatabase>() }

#[derive(Debug, Serialize)]
pub struct DeleteSessionDatabaseResult {
    pub deleted: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteSessionDatabase {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
}

impl Command for DeleteSessionDatabase {
    type Output = DeleteSessionDatabaseResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_session_database",
            category: "session_databases",
            description: "Delete a session SQL database.",
            method: "DELETE",
            path: "/v1/sessions/{session_id}/databases/{name}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_MANAGE)
    }

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

inventory::submit! { CommandDescriptor::of::<DeleteSessionDatabase>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetSessionDatabaseSchema {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
}

impl Command for GetSessionDatabaseSchema {
    type Output = SchemaResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session_database_schema",
            category: "session_databases",
            description: "Inspect the schema of a session SQL database.",
            method: "GET",
            path: "/v1/sessions/{session_id}/databases/{name}/schema",
        }
    }

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

inventory::submit! { CommandDescriptor::of::<GetSessionDatabaseSchema>() }
