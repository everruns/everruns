//! Public schedule reads share the session API's policy and choose the
//! session's authority: PostgreSQL for v1, its shard for v2.
use super::SessionScheduleService;
use crate::domains::common::*;
use crate::domains::sessions::SESSION_VIEW;
use everruns_contracts::typed_id::{ScheduleId, SessionId};
use everruns_core::session_schedule::SessionSchedule;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub async fn read_schedules(
    ctx: &Ctx,
    session_id: SessionId,
) -> Result<Vec<SessionSchedule>, CommandError> {
    if ctx
        .db
        .get_session(ctx.org_id(), session_id)
        .await?
        .is_none()
    {
        return Err(CommandError::not_found("Session"));
    }
    if let Some(plane) = crate::domains::data_plane::session_plane::SessionPlane::of(
        &ctx.db,
        ctx.org_id(),
        session_id,
    )
    .await
    .map_err(CommandError::internal)?
    {
        let value = plane
            .read(&everruns_contracts::session_commands::SCHEDULES)
            .await
            .map_err(CommandError::internal)?;
        return serde_json::from_value(value).map_err(|e| CommandError::internal(e.into()));
    }
    SessionScheduleService::new(ctx.db.clone())
        .list(ctx.org_id(), session_id)
        .await
        .map_err(CommandError::internal)
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListSessionSchedules {
    /// Session's prefixed public identifier.
    pub session_id: SessionId,
}

#[command(
    name = "list_session_schedules", category = "session_schedules",
    description = "Read a session's follow-up schedules, including purpose, stop conditions, limits, spend and deleted schedules.",
    method = "GET", path = "/v1/platform/sessions/{session_id}/schedules",
    policy = SESSION_VIEW,
    cli = CliRoute::new(&["sessions", "schedules"], "list").with_examples(&[CliExample::new("Inspect a session's follow-ups and their stop conditions", "everruns sessions schedules list --session-id session_01h9")]),
)]
impl Command for ListSessionSchedules {
    type Output = Vec<SessionSchedule>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        read_schedules(ctx, self.session_id).await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetSessionSchedule {
    /// Session's prefixed public identifier.
    pub session_id: SessionId,
    /// Schedule's prefixed public identifier.
    pub schedule_id: ScheduleId,
}

#[command(
    name = "get_session_schedule", category = "session_schedules",
    description = "Read one follow-up schedule in its owning session, with its purpose, stop condition, lifetime, spend and stop reason.",
    method = "GET", path = "/v1/platform/sessions/{session_id}/schedules/{schedule_id}",
    policy = SESSION_VIEW,
    cli = CliRoute::new(&["sessions", "schedules"], "get").with_examples(&[CliExample::new("Read one follow-up's purpose, limits and stop reason", "everruns sessions schedules get --session-id session_01h9 --schedule-id sched_01h9")]),
)]
impl Command for GetSessionSchedule {
    type Output = SessionSchedule;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        read_schedules(ctx, self.session_id)
            .await?
            .into_iter()
            .find(|schedule| schedule.id == self.schedule_id)
            .ok_or_else(|| CommandError::not_found("Schedule"))
    }
}
