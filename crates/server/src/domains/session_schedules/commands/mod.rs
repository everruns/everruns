//! The worker's session schedule store, as internal commands.
//!
//! Decision: the `schedule_*` tools the agent runtime runs reach these through
//! `ExecuteCommand` (gRPC workers) and `dispatch` (the in-process worker), so
//! one implementation serves both. They used to be five bespoke RPCs plus a
//! direct adapter each. Internal (`INTERNAL_COMMAND_PATH_PREFIX`): the public
//! schedule surface is the `schedules` domain; these exist for the runtime's
//! `SessionScheduleStore` and keep its exact semantics, including create-time
//! limits.
//!
//! Errors keep the store's split: a limit rejection is `unprocessable`, which
//! the worker reads back as `ScheduleLimitError::Rejected` (the tool tells the
//! model why); every other failure stays internal, as it was over the RPCs.

use super::super::session_resources::queries::parse_session_id;
use super::super::session_storage::queries::verify_session_ownership;
use crate::domains::common::*;
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW};
use crate::kernel_imports::{
    contracts::typed_id::ScheduleId, contracts::typed_id::SessionId,
    session_schedule::ScheduleLimitError, session_schedule::SessionSchedule,
    session_services::SessionScheduleStore,
};
use crate::storage::DbSessionScheduleStore;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

fn store(ctx: &Ctx) -> DbSessionScheduleStore {
    DbSessionScheduleStore::new(ctx.db.clone(), ctx.org_id())
}

fn store_failure(error: impl std::fmt::Display) -> CommandError {
    CommandError::internal(anyhow::anyhow!("{error}"))
}

/// The session, after confirming it belongs to the caller's org: the store
/// addresses some rows by session alone.
async fn owned_session(ctx: &Ctx, session_id: &str) -> Result<SessionId, CommandError> {
    let session_id = parse_session_id(session_id)?;
    verify_session_ownership(&ctx.db, ctx.org_id(), session_id).await?;
    Ok(session_id)
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerCreateSessionSchedule {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// What the agent should do when the schedule fires.
    pub description: String,
    /// Cron expression for a recurring schedule.
    #[serde(default)]
    pub cron_expression: Option<String>,
    /// Trigger time for a one-shot schedule.
    #[serde(default)]
    pub scheduled_at: Option<DateTime<Utc>>,
    /// IANA timezone the cron expression is read in.
    pub timezone: String,
}

#[command(
    name = "worker_create_session_schedule",
    category = "session_schedules",
    description = "Internal: create a session schedule within the create-time limits.",
    method = "POST",
    path = "/internal/sessions/{session_id}/schedules",
    policy = SESSION_MANAGE
)]
impl Command for WorkerCreateSessionSchedule {
    type Output = SessionSchedule;

    async fn execute(self, ctx: &Ctx) -> Result<SessionSchedule, CommandError> {
        // The store resolves the session within the org itself.
        let session_id = parse_session_id(&self.session_id)?;
        store(ctx)
            .create_schedule_enforcing_limits(
                session_id,
                self.description,
                self.cron_expression,
                self.scheduled_at,
                self.timezone,
            )
            .await
            .map_err(|error| match error {
                ScheduleLimitError::Rejected(message) => CommandError::unprocessable(message),
                ScheduleLimitError::Store(error) => store_failure(error),
            })
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerCancelSessionSchedule {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// Schedule's prefixed public identifier.
    pub schedule_id: String,
}

#[command(
    name = "worker_cancel_session_schedule",
    category = "session_schedules",
    description = "Internal: disable a session schedule.",
    method = "POST",
    path = "/internal/sessions/{session_id}/schedules/{schedule_id}/cancel",
    policy = SESSION_MANAGE
)]
impl Command for WorkerCancelSessionSchedule {
    type Output = SessionSchedule;

    async fn execute(self, ctx: &Ctx) -> Result<SessionSchedule, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        let schedule_id: ScheduleId = self
            .schedule_id
            .parse()
            .map_err(|error| CommandError::bad_request(format!("Invalid schedule ID: {error}")))?;
        store(ctx)
            .cancel_schedule(session_id, schedule_id)
            .await
            .map_err(store_failure)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerListSessionSchedules {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "worker_list_session_schedules",
    category = "session_schedules",
    description = "Internal: list a session's schedules.",
    method = "GET",
    path = "/internal/sessions/{session_id}/schedules",
    policy = SESSION_VIEW
)]
impl Command for WorkerListSessionSchedules {
    type Output = Vec<SessionSchedule>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<SessionSchedule>, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        store(ctx)
            .list_schedules(session_id)
            .await
            .map_err(store_failure)
    }
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerCountActiveSessionSchedules {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "worker_count_active_session_schedules",
    category = "session_schedules",
    description = "Internal: count a session's active schedules.",
    method = "GET",
    path = "/internal/sessions/{session_id}/schedules/active-count",
    policy = SESSION_VIEW
)]
impl Command for WorkerCountActiveSessionSchedules {
    type Output = u32;

    async fn execute(self, ctx: &Ctx) -> Result<u32, CommandError> {
        let session_id = owned_session(ctx, &self.session_id).await?;
        store(ctx)
            .count_active_schedules(session_id)
            .await
            .map_err(store_failure)
    }
}

#[derive(Debug, Default, Deserialize, ToSchema, Serialize)]
pub struct WorkerCountActiveOrgSessionSchedules {}

#[command(
    name = "worker_count_active_org_session_schedules",
    category = "session_schedules",
    description = "Internal: count the organization's active session schedules.",
    method = "GET",
    path = "/internal/session-schedules/active-count",
    policy = SESSION_VIEW
)]
impl Command for WorkerCountActiveOrgSessionSchedules {
    type Output = u32;

    async fn execute(self, ctx: &Ctx) -> Result<u32, CommandError> {
        store(ctx)
            .count_active_org_schedules()
            .await
            .map_err(store_failure)
    }
}

#[cfg(test)]
mod tests;
