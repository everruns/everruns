//! A v2 session's SQL databases, through the control plane's API.
//!
//! Decisions:
//! - A v2 session's databases live on its shard (`hosting_sqldb.rs`); the
//!   control plane's copy of the v1 store holds nothing for it. When the
//!   edge is down and requests fall back to the control plane, the
//!   database API must answer from the shard or not at all: reading or
//!   writing the v1 store for a v2 session would show an empty list and
//!   create databases the session's agent never sees.
//! - So every database command asks the session's shard with the `sqldb`
//!   session command, and an unreachable shard answers 503, never the v1
//!   store.

use everruns_contracts::session_commands;
use everruns_contracts::session_sqldb::TableSchema;
use everruns_contracts::typed_id::SessionId;
use serde::Deserialize;
use serde_json::{Value, json};

use super::types::DatabaseInfoResponse;
use crate::domains::common::*;
use crate::domains::data_plane::session_plane::SessionPlane;

/// What the shard answered, inside its reply (`sqldb.rs` on the shard).
#[derive(Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum SqlResult {
    Done { value: Value },
    Failed { status: u16, message: String },
}

/// The session's databases on its data plane.
pub struct PlaneDatabases(SessionPlane);

impl PlaneDatabases {
    /// `session`'s databases when it runs on a data plane.
    pub async fn of(ctx: &Ctx, session: SessionId) -> Result<Option<Self>, CommandError> {
        Ok(SessionPlane::of(&ctx.db, ctx.org_id(), session)
            .await
            .map_err(CommandError::internal)?
            .map(Self))
    }

    async fn call(&self, op: Value) -> Result<Value, CommandError> {
        let reply = self
            .0
            .command(&session_commands::SQLDB, &op)
            .await
            .map_err(|error| {
                tracing::warn!(error = %error, session = %self.0.session, "session databases: data plane unreachable");
                CommandError::unavailable(
                    "The session's data plane is unavailable; try again in a moment",
                )
            })?;
        match serde_json::from_value::<SqlResult>(reply) {
            Ok(SqlResult::Done { value }) => Ok(value),
            Ok(SqlResult::Failed { status, message }) => Err(match status {
                400 => CommandError::bad_request(message),
                404 => CommandError::not_found("Database"),
                409 => CommandError::conflict(message),
                422 => CommandError::unprocessable(message),
                503 => CommandError::unavailable(message),
                _ => CommandError::internal(anyhow::anyhow!("session databases: {message}")),
            }),
            Err(error) => Err(CommandError::internal(error.into())),
        }
    }

    pub async fn list(&self) -> Result<Vec<DatabaseInfoResponse>, CommandError> {
        #[derive(Deserialize)]
        struct Listed {
            data: Vec<DatabaseInfoResponse>,
        }
        let value = self.call(json!({ "op": "list" })).await?;
        Ok(parse::<Listed>(value)?.data)
    }

    pub async fn get(&self, name: &str) -> Result<DatabaseInfoResponse, CommandError> {
        parse(self.call(json!({ "op": "get", "name": name })).await?)
    }

    pub async fn create(&self, name: &str) -> Result<DatabaseInfoResponse, CommandError> {
        parse(self.call(json!({ "op": "create", "name": name })).await?)
    }

    pub async fn delete(&self, name: &str) -> Result<(), CommandError> {
        self.call(json!({ "op": "delete", "name": name }))
            .await
            .map(drop)
    }

    pub async fn schema(&self, name: &str) -> Result<Vec<TableSchema>, CommandError> {
        #[derive(Deserialize)]
        struct Schema {
            tables: Vec<TableSchema>,
        }
        let value = self.call(json!({ "op": "schema", "name": name })).await?;
        Ok(parse::<Schema>(value)?.tables)
    }
}

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, CommandError> {
    serde_json::from_value(value).map_err(|e| CommandError::internal(e.into()))
}
