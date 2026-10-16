// An agent's failure count drills into terminal-turn history, independently
// of session creation time and the latest turn's outcome.

use chrono::{DateTime, Duration, Utc};
use everruns_contracts::typed_id::AgentId;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::domains::common::*;
use crate::domains::sessions::{SESSION_VIEW, SessionService, record::Session};

#[derive(Debug, Deserialize, Serialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListAgentFailures {
    #[param(ignore)]
    pub agent_id: String,
    /// Exclusive lower bound on failure time; defaults to 24 hours before `before`.
    pub after: Option<DateTime<Utc>>,
    /// Inclusive upper bound, normally the Agents activity response's generated_at.
    pub before: Option<DateTime<Utc>>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentFailedSession {
    pub session: Session,
    pub failed_turns: i64,
    /// Sequence of the latest terminal failure in the requested window.
    pub failure_sequence: i32,
    pub failed_at: DateTime<Utc>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentFailureSessions {
    pub data: Vec<AgentFailedSession>,
    pub total_sessions: i64,
    pub failed_turns: i64,
    pub after: DateTime<Utc>,
    pub before: DateTime<Utc>,
    pub offset: u32,
    pub limit: u32,
}

fn failure_window(
    after: Option<DateTime<Utc>>,
    before: Option<DateTime<Utc>>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), CommandError> {
    let before = before.unwrap_or_else(Utc::now);
    let after = match after {
        Some(after) => after,
        None => before
            .checked_sub_signed(Duration::hours(24))
            .ok_or_else(|| CommandError::bad_request("Invalid failure window"))?,
    };
    if after >= before || before - after > Duration::days(30) {
        return Err(CommandError::bad_request(
            "Failure window must be positive and no longer than 30 days",
        ));
    }
    Ok((after, before))
}

#[command(
    name = "list_agent_failures",
    category = "agents",
    description = "List sessions with failed turns. Filters by failure time and includes older, recovered, and archived sessions, matching agent activity. Returns the latest matching failure per session.",
    method = "GET",
    path = "/v1/agents/{agent_id}/failures",
    policy = SESSION_VIEW,
    http = plain,
    params(ListAgentFailures),
)]
impl Command for ListAgentFailures {
    type Output = AgentFailureSessions;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        super::AGENT_VIEW
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .map_err(|error| CommandError::forbidden(error.message))?;
        let public_agent = self
            .agent_id
            .parse::<AgentId>()
            .map_err(|_| CommandError::bad_request("Invalid agent ID"))?;
        let agent = ctx
            .db
            .get_agent_by_public_id(ctx.org_id(), &self.agent_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))?;
        let (after, before) = failure_window(self.after, self.before)?;
        let limit = self.limit.unwrap_or(20).clamp(1, 50);
        let offset = self.offset.unwrap_or(0);
        let (rows, totals) = ctx
            .db
            .agent_failure_sessions(ctx.org_id(), agent.id, after, before, offset, limit)
            .await?;
        let fallback = if rows.iter().any(|row| row.session.harness_id.is_none()) {
            Some(crate::setup::org_init::base_harness_id(&ctx.db, ctx.org_id()).await?)
        } else {
            None
        };
        Ok(AgentFailureSessions {
            data: rows
                .into_iter()
                .map(|row| {
                    let mut session = SessionService::row_to_session(
                        row.session,
                        &ctx.caller.org_public_id,
                        fallback,
                    );
                    // Storage FKs use the internal agent UUID; public session
                    // records name the public agent, as SessionService::list does.
                    session.agent_id = Some(public_agent);
                    AgentFailedSession {
                        session,
                        failed_turns: row.failed_turns,
                        failure_sequence: row.failure_sequence,
                        failed_at: row.failed_at,
                        error: row.error,
                    }
                })
                .collect(),
            total_sessions: totals.sessions,
            failed_turns: totals.failed_turns,
            after,
            before,
            offset,
            limit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_window_rejects_reversed_and_unbounded_ranges() {
        let now = Utc::now();
        assert!(failure_window(Some(now), Some(now)).is_err());
        assert!(failure_window(Some(now + Duration::seconds(1)), Some(now)).is_err());
        assert!(failure_window(Some(now - Duration::days(31)), Some(now)).is_err());
        assert_eq!(
            failure_window(None, Some(now)).unwrap(),
            (now - Duration::hours(24), now)
        );
    }
}

#[cfg(test)]
mod history_tests;
