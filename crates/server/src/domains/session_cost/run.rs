// GetSessionRun: the whole run a session belongs to, for the session's Agents
// tab, the Trace "Whole run" timeline and the Cost "Whole run" split.
// Spec: knowledge/runtime-resources/agents-using-agents.md (Whole run).
//
// Decision: a run is the first session and every session started under it
// (`root_session_id`), the same tree run requests and the team folder use. Any
// session of the run answers for the whole run, so a member's own page shows
// the run it is part of.
// Decision: read-only and computed on read from existing records: session
// status and spend (`sessions.total_cost_usd`), turns from the trace index,
// turn spend from `llm_generations`. No new storage.
// Decision: anyone who can view one session of the run sees every agent's
// status, timing and spend in it, and nothing more. Conversations stay behind
// each session's own view.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{AgentId, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::domains::common::*;
use crate::domains::sessions::SESSION_VIEW;
use crate::domains::sessions::run_requests::count_person_requests;

/// Sessions read for one run: its first session and up to 32 agents, each of
/// which can be handed work several times.
pub const MAX_RUN_SESSIONS: i64 = 200;
/// Turns returned at most across the run; the newest when there are more.
pub const MAX_RUN_TURNS: i64 = 2000;

#[derive(Debug, Deserialize, ToSchema, IntoParams, Serialize)]
#[into_params(parameter_in = Query)]
pub struct GetSessionRun {
    /// Session's prefixed public identifier (a path parameter).
    #[param(ignore)]
    pub session_id: String,
}

/// The agent behind a session of the run.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RunAgent {
    /// The agent's id, as `/v1/agents/{agent_id}` takes it.
    #[schema(example = "agent_01h9xk2m")]
    pub id: String,
    /// The agent's display name.
    #[schema(example = "Triage")]
    pub name: String,
}

/// One turn of a session in the run.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RunTurn {
    /// Turn number within its session, from 1.
    #[schema(example = 1)]
    pub turn_no: i32,
    /// Public turn id (`turn_...`).
    #[schema(example = "turn_01h9xk2m")]
    pub turn_id: String,
    #[schema(example = "2026-11-03T09:00:00Z")]
    pub started_at: DateTime<Utc>,
    /// Absent while the turn runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "2026-11-03T09:01:30Z")]
    pub ended_at: Option<DateTime<Utc>>,
    /// Trace status of the turn: `running`, `completed`, `failed`, ...
    #[schema(example = "completed")]
    pub status: String,
    /// Model spend of the turn in USD.
    #[schema(example = 0.21)]
    pub cost_usd: f64,
}

/// One agent session of the run.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RunSession {
    /// Session's prefixed public identifier.
    #[schema(example = "session_01h9xk2m")]
    pub session_id: String,
    /// The session that handed it work; absent for the first session.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "session_01h9xk1a")]
    pub parent_session_id: Option<String>,
    /// The agent that ran it. Absent when it has none or it was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<RunAgent>,
    /// The session's title: for work handed off, the task's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Triage the failing build")]
    pub title: Option<String>,
    /// Session status: `started`, `active`, `idle`, `waiting_for_tool_results`, ...
    #[schema(example = "idle")]
    pub status: String,
    /// Outcome of its last finished turn: `completed`, `failed` or `cancelled`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "completed")]
    pub last_turn_status: Option<String>,
    #[schema(example = "2026-11-03T09:00:00Z")]
    pub created_at: DateTime<Utc>,
    /// When its last turn ended; absent while it works or before it ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "2026-11-03T09:01:30Z")]
    pub last_turn_at: Option<DateTime<Utc>>,
    /// Model spend in USD.
    #[schema(example = 0.21)]
    pub cost_usd: f64,
    /// Its turns, oldest first.
    pub turns: Vec<RunTurn>,
}

/// The run a session belongs to: every agent that took part, with status,
/// timing and spend.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionRun {
    /// The run's first session, the one the person talks to.
    #[schema(example = "session_01h9xk1a")]
    pub root_session_id: String,
    /// Spend of every session in the run, in USD.
    #[schema(example = 5.06)]
    pub cost_usd: f64,
    /// Spend of the first session alone, in USD.
    #[schema(example = 0.25)]
    pub first_session_cost_usd: f64,
    /// Questions and approvals any agent in the run waits on the person for.
    #[schema(example = 1)]
    pub waiting_requests: usize,
    /// The first session, then every session started under it by creation.
    pub sessions: Vec<RunSession>,
    /// More sessions or turns existed than were returned.
    #[schema(example = false)]
    pub truncated: bool,
}

#[command(
    name = "get_session_run",
    category = "sessions",
    description = "Show the run a session belongs to: the first session and every agent session started under it, with status, turns and spend.",
    summary = "Show a session's whole run",
    method = "GET",
    path = "/v1/sessions/{session_id}/run",
    policy = SESSION_VIEW,
    cli = CliRoute::new(&["sessions", "run"], "get").with_args(&[CliArg::new("session_id").at(1)]).with_examples(&[CliExample::new("See every agent in a session's run", "everruns sessions run get session_01h9")]),
    positional = "session_id",
    http = plain,
    params(GetSessionRun),
)]
impl Command for GetSessionRun {
    type Output = SessionRun;

    async fn execute(self, ctx: &Ctx) -> Result<SessionRun, CommandError> {
        let session_id: SessionId = self
            .session_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid session ID: {e}")))?;
        let org = ctx.org_id();
        if ctx.db.get_session(org, session_id).await?.is_none() {
            return Err(CommandError::not_found("Session"));
        }
        // The session itself sorts first; its row carries the run's root.
        let root = ctx
            .db
            .list_run_sessions(org, session_id, 1)
            .await?
            .first()
            .and_then(|row| row.root_session_id)
            .unwrap_or(session_id);
        let rows = ctx
            .db
            .list_run_sessions(org, root, MAX_RUN_SESSIONS + 1)
            .await?;
        let mut truncated = rows.len() as i64 > MAX_RUN_SESSIONS;
        let rows: Vec<_> = rows.into_iter().take(MAX_RUN_SESSIONS as usize).collect();
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id.uuid()).collect();
        // Turns come from the trace index; bring each session's up to date.
        for id in &ids {
            ctx.db.database().catch_up_session_trace_fully(*id).await?;
        }
        let turns = ctx.db.run_turns(&ids, MAX_RUN_TURNS + 1).await?;
        truncated |= turns.len() as i64 > MAX_RUN_TURNS;
        let costs: HashMap<(Uuid, Uuid), f64> = ctx
            .db
            .run_turn_costs(org, &ids)
            .await?
            .into_iter()
            .filter_map(|row| Some(((row.session_id, row.turn_id?), row.cost_usd)))
            .collect();
        let agent_ids: Vec<AgentId> = rows.iter().filter_map(|row| row.agent_id).collect();
        let agents: HashMap<AgentId, RunAgent> = ctx
            .db
            .get_agents_by_ids(org, &agent_ids)
            .await?
            .into_iter()
            .map(|agent| {
                let run_agent = RunAgent {
                    id: agent.public_id,
                    name: agent.display_name.unwrap_or(agent.name),
                };
                (agent.id, run_agent)
            })
            .collect();
        let waiting: Vec<SessionId> = rows
            .iter()
            .filter(|row| row.status == "waiting_for_tool_results")
            .map(|row| row.id)
            .collect();
        let waiting_requests = count_person_requests(&ctx.db, &waiting).await?;

        let mut by_session: HashMap<Uuid, Vec<RunTurn>> = HashMap::new();
        let skip = turns.len().saturating_sub(MAX_RUN_TURNS as usize);
        for turn in turns.into_iter().skip(skip) {
            let cost_usd = costs
                .get(&(turn.session_id, turn.turn_id))
                .copied()
                .unwrap_or(0.0);
            by_session
                .entry(turn.session_id)
                .or_default()
                .push(RunTurn {
                    turn_no: turn.turn_no,
                    turn_id: TurnId::from_uuid(turn.turn_id).to_string(),
                    started_at: turn.started_at,
                    ended_at: turn.ended_at,
                    status: turn.status,
                    cost_usd,
                });
        }
        Ok(assemble(
            root,
            rows,
            &agents,
            by_session,
            waiting_requests,
            truncated,
        ))
    }
}

/// Put the run together; pure so the ordering and totals are tested alone.
pub(crate) fn assemble(
    root: SessionId,
    rows: Vec<crate::storage::SessionRow>,
    agents: &HashMap<AgentId, RunAgent>,
    mut turns: HashMap<Uuid, Vec<RunTurn>>,
    waiting_requests: usize,
    truncated: bool,
) -> SessionRun {
    let sessions: Vec<RunSession> = rows
        .into_iter()
        .map(|row| RunSession {
            session_id: row.id.to_string(),
            parent_session_id: row
                .parent_session_id
                .filter(|_| row.id != root)
                .map(|id| id.to_string()),
            agent: row.agent_id.and_then(|id| agents.get(&id).cloned()),
            title: row.title,
            status: row.status,
            last_turn_status: row.last_turn_status,
            created_at: row.created_at,
            last_turn_at: row.last_turn_at,
            cost_usd: row.total_cost_usd,
            turns: turns.remove(&row.id.uuid()).unwrap_or_default(),
        })
        .collect();
    let root_id = root.to_string();
    SessionRun {
        cost_usd: sessions.iter().map(|session| session.cost_usd).sum(),
        first_session_cost_usd: sessions
            .iter()
            .filter(|session| session.session_id == root_id)
            .map(|session| session.cost_usd)
            .sum(),
        root_session_id: root_id,
        waiting_requests,
        sessions,
        truncated,
    }
}
