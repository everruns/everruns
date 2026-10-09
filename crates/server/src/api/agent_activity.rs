// GET /v1/agents/activity: the read model behind the Agents home page.
//
// Decision: one org-wide read instead of a stats call per agent. The page shows
// every agent's load, 24-hour run bars, and every channel's 7-day traffic at
// once; N+1 calls would make the busiest orgs the slowest page.
//
// Decision: buckets are dense arrays, oldest first, so a client draws bars
// without knowing the window arithmetic. Agents and channels with no activity
// are omitted; the client already has the full agent and channel lists.

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use chrono::{DateTime, Utc};
use everruns_core::Caller;
use serde::Serialize;
use std::collections::BTreeMap;
use utoipa::ToSchema;

use super::agents::AppState;
use super::common::{ApiResult, ApiResultExt, ErrorResponse};
use crate::auth::ResolvedOrg;
use crate::domains::agents::AGENT_VIEW;
use crate::domains::sessions::limits::OrgCaps;
use crate::storage::{AGENT_ACTIVITY_HOURS, AgentActivityRows, CHANNEL_ACTIVITY_DAYS};

pub fn routes() -> Router<AppState> {
    Router::new().route("/v1/agents/activity", get(get_agent_activity))
}

/// Turns started and failed in one bucket.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, ToSchema)]
pub struct RunBucket {
    pub runs: u64,
    pub failed: u64,
}

/// A trigger: a run the agent starts itself (schedule, webhook, GitHub, MCP event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AgentTriggerSummary {
    #[schema(example = "github")]
    pub trigger_type: String,
    pub enabled: bool,
}

/// What one agent is doing now and how its last day went.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct AgentActivity {
    /// Public agent identifier.
    #[schema(example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: String,
    /// Sessions executing a turn right now.
    pub running_sessions: u64,
    /// When the agent's most recent turn ended or started.
    pub last_turn_at: Option<DateTime<Utc>>,
    /// Turns started in the window.
    pub runs: u64,
    /// Turns that failed in the window.
    pub failed: u64,
    /// One bucket per hour, oldest first; the last bucket ends at `generated_at`.
    pub hourly: Vec<RunBucket>,
    /// Active triggers, oldest first.
    pub triggers: Vec<AgentTriggerSummary>,
}

/// Traffic one channel brought in over the last week.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct ChannelActivity {
    /// Public channel identifier.
    #[schema(example = "appchan_01933b5a000070008000000000000001")]
    pub channel_id: String,
    /// Sessions the channel started in the window.
    pub sessions: u64,
    /// Sessions per day, oldest first; the last day ends at `generated_at`.
    pub daily: Vec<u64>,
    /// When the channel last started a session inside the window.
    pub last_session_at: Option<DateTime<Utc>>,
    /// Distinct end users behind the channel's sessions in the window.
    pub people: u64,
    /// Sessions that carried an end-user identity. Zero means the channel
    /// cannot tell people apart (webhook, API, anonymous callers), so
    /// `people` says nothing about its audience.
    pub identified_sessions: u64,
    /// Median milliseconds from a session's first user message to the agent's
    /// first completed reply, over sessions that got one.
    pub median_first_reply_ms: Option<u64>,
}

/// Org-wide totals for the page masthead.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct AgentActivityTotals {
    /// Sessions executing a turn right now, across every agent.
    pub running_sessions: u64,
    /// The organization's active-turn limit (`ORG_MAX_ACTIVE_TURNS`).
    pub max_active_turns: u64,
    /// Turns started in the last 24 hours.
    pub runs: u64,
    /// Turns that failed in the last 24 hours.
    pub failed: u64,
    /// Sessions channels started in the last 7 days.
    pub channel_sessions: u64,
    /// Distinct end users across every channel in the last 7 days.
    pub people_reached: u64,
}

/// Activity for every agent and channel in the organization.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct AgentActivityOverview {
    pub generated_at: DateTime<Utc>,
    /// Hours covered by `AgentActivity.hourly`.
    pub hours: u32,
    /// Days covered by `ChannelActivity.daily`.
    pub days: u32,
    pub totals: AgentActivityTotals,
    pub agents: Vec<AgentActivity>,
    pub channels: Vec<ChannelActivity>,
}

/// Fold the storage rows into dense, oldest-first series.
pub fn build_overview(
    rows: AgentActivityRows,
    now: DateTime<Utc>,
    max_active_turns: u64,
) -> AgentActivityOverview {
    let hours = AGENT_ACTIVITY_HOURS as usize;
    let days = CHANNEL_ACTIVITY_DAYS as usize;
    let mut agents: BTreeMap<String, AgentActivity> = BTreeMap::new();
    fn agent<'a>(
        agents: &'a mut BTreeMap<String, AgentActivity>,
        id: &str,
        hours: usize,
    ) -> &'a mut AgentActivity {
        agents
            .entry(id.to_string())
            .or_insert_with(|| AgentActivity {
                agent_id: id.to_string(),
                running_sessions: 0,
                last_turn_at: None,
                runs: 0,
                failed: 0,
                hourly: vec![RunBucket::default(); hours],
                triggers: Vec::new(),
            })
    }

    for load in rows.loads {
        let agent = agent(&mut agents, &load.agent_id, hours);
        agent.running_sessions = load.running_sessions.max(0) as u64;
        agent.last_turn_at = load.last_turn_at;
    }
    for trigger in rows.triggers {
        agent(&mut agents, &trigger.agent_id, hours)
            .triggers
            .push(AgentTriggerSummary {
                trigger_type: trigger.trigger_type,
                enabled: trigger.enabled,
            });
    }
    for bucket in rows.run_buckets {
        let Some(index) = oldest_first_index(bucket.hours_ago, hours) else {
            continue;
        };
        let agent = agent(&mut agents, &bucket.agent_id, hours);
        let runs = bucket.runs.max(0) as u64;
        let failed = bucket.failed.max(0) as u64;
        agent.hourly[index].runs += runs;
        agent.hourly[index].failed += failed;
        agent.runs += runs;
        agent.failed += failed;
    }

    let mut channels: BTreeMap<String, ChannelActivity> = BTreeMap::new();
    for bucket in rows.channel_buckets {
        let Some(index) = oldest_first_index(bucket.days_ago, days) else {
            continue;
        };
        let channel = channels
            .entry(bucket.channel_id.clone())
            .or_insert_with(|| ChannelActivity {
                channel_id: bucket.channel_id.clone(),
                sessions: 0,
                daily: vec![0; days],
                last_session_at: None,
                people: 0,
                identified_sessions: 0,
                median_first_reply_ms: None,
            });
        channel.last_session_at = channel.last_session_at.max(Some(bucket.last_session_at));
        let sessions = bucket.sessions.max(0) as u64;
        channel.daily[index] += sessions;
        channel.sessions += sessions;
    }

    for audience in rows.channel_audience {
        // Audience rows cover the same window as the buckets, so a channel
        // without buckets had no sessions there and has nothing to report.
        let Some(channel) = channels.get_mut(&audience.channel_id) else {
            continue;
        };
        channel.people = audience.people.max(0) as u64;
        channel.identified_sessions = audience.identified_sessions.max(0) as u64;
        channel.median_first_reply_ms = audience
            .median_first_reply_ms
            .filter(|ms| ms.is_finite() && *ms >= 0.0)
            .map(|ms| ms.round() as u64);
    }

    // An agent whose sessions never ran a turn and that has no trigger carries
    // no signal for this page.
    let agents: Vec<AgentActivity> = agents
        .into_values()
        .filter(|agent| {
            agent.running_sessions > 0
                || agent.runs > 0
                || agent.last_turn_at.is_some()
                || !agent.triggers.is_empty()
        })
        .collect();
    let channels: Vec<ChannelActivity> = channels.into_values().collect();
    let totals = AgentActivityTotals {
        running_sessions: agents.iter().map(|agent| agent.running_sessions).sum(),
        max_active_turns,
        runs: agents.iter().map(|agent| agent.runs).sum(),
        failed: agents.iter().map(|agent| agent.failed).sum(),
        channel_sessions: channels.iter().map(|channel| channel.sessions).sum(),
        people_reached: rows.people_reached.max(0) as u64,
    };

    AgentActivityOverview {
        generated_at: now,
        hours: hours as u32,
        days: days as u32,
        totals,
        agents,
        channels,
    }
}

/// `ago` counts back from now (0 = newest); series are stored oldest first.
fn oldest_first_index(ago: i32, len: usize) -> Option<usize> {
    let ago = usize::try_from(ago).ok()?;
    (ago < len).then(|| len - 1 - ago)
}

/// GET /v1/agents/activity - Org-wide agent and channel activity
#[utoipa::path(
    get,
    path = "/v1/agents/activity",
    responses(
        (status = 200, description = "Agent and channel activity", body = AgentActivityOverview),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn get_agent_activity(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<AgentActivityOverview> {
    AGENT_VIEW
        .evaluate_with(state.auth.permission_resolver.as_ref(), &Caller::from(&org))
        .map_err(|error| ErrorResponse::new(error.message).into_response(StatusCode::FORBIDDEN))?;
    let now = Utc::now();
    let rows = state
        .db
        .agent_activity(org.org_id, now)
        .await
        .log_internal_error_json("load agent activity")?;
    let max_active_turns = OrgCaps::from_env().max_active_turns as u64;
    Ok(Json(build_overview(rows, now, max_active_turns)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{
        AgentLoadRow, AgentRunBucketRow, AgentTriggerSummaryRow, ChannelAudienceRow,
        ChannelSessionBucketRow,
    };

    fn rows() -> AgentActivityRows {
        AgentActivityRows {
            loads: vec![
                AgentLoadRow {
                    agent_id: "agent_busy".into(),
                    running_sessions: 2,
                    last_turn_at: Some(Utc::now()),
                },
                AgentLoadRow {
                    agent_id: "agent_never".into(),
                    running_sessions: 0,
                    last_turn_at: None,
                },
            ],
            triggers: vec![AgentTriggerSummaryRow {
                agent_id: "agent_scheduled".into(),
                trigger_type: "schedule".into(),
                enabled: true,
            }],
            run_buckets: vec![
                AgentRunBucketRow {
                    agent_id: "agent_busy".into(),
                    hours_ago: 0,
                    runs: 5,
                    failed: 1,
                },
                AgentRunBucketRow {
                    agent_id: "agent_busy".into(),
                    hours_ago: 23,
                    runs: 2,
                    failed: 0,
                },
                // Out of range is dropped rather than wrapping into a bucket.
                AgentRunBucketRow {
                    agent_id: "agent_busy".into(),
                    hours_ago: 24,
                    runs: 9,
                    failed: 9,
                },
            ],
            channel_buckets: vec![
                ChannelSessionBucketRow {
                    channel_id: "appchan_a".into(),
                    days_ago: 0,
                    sessions: 3,
                    last_session_at: recent(),
                },
                ChannelSessionBucketRow {
                    channel_id: "appchan_a".into(),
                    days_ago: 6,
                    sessions: 1,
                    last_session_at: recent() - chrono::Duration::days(6),
                },
            ],
            channel_audience: vec![
                ChannelAudienceRow {
                    channel_id: "appchan_a".into(),
                    people: 2,
                    identified_sessions: 4,
                    median_first_reply_ms: Some(1499.6),
                    replied_sessions: 4,
                },
                // No sessions in the window: dropped rather than invented.
                ChannelAudienceRow {
                    channel_id: "appchan_quiet".into(),
                    people: 1,
                    identified_sessions: 1,
                    median_first_reply_ms: None,
                    replied_sessions: 0,
                },
            ],
            people_reached: 2,
        }
    }

    fn recent() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-09T08:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn series_are_dense_and_oldest_first() {
        let overview = build_overview(rows(), Utc::now(), 1000);
        let busy = &overview.agents[0];
        assert_eq!(busy.agent_id, "agent_busy");
        assert_eq!(busy.hourly.len(), 24);
        assert_eq!(busy.hourly[23], RunBucket { runs: 5, failed: 1 });
        assert_eq!(busy.hourly[0], RunBucket { runs: 2, failed: 0 });
        assert_eq!((busy.runs, busy.failed), (7, 1));

        let channel = &overview.channels[0];
        assert_eq!(channel.daily, vec![1, 0, 0, 0, 0, 0, 3]);
        assert_eq!(channel.sessions, 4);
        assert_eq!(
            channel.last_session_at,
            Some(recent()),
            "newest bucket wins"
        );
        assert_eq!((channel.people, channel.identified_sessions), (2, 4));
        assert_eq!(channel.median_first_reply_ms, Some(1500));
        assert_eq!(overview.channels.len(), 1, "appchan_quiet had no sessions");
        assert_eq!(overview.totals.people_reached, 2);
    }

    #[test]
    fn idle_agents_with_no_turns_are_omitted_and_totals_add_up() {
        let overview = build_overview(rows(), Utc::now(), 1000);
        let ids: Vec<&str> = overview
            .agents
            .iter()
            .map(|a| a.agent_id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["agent_busy", "agent_scheduled"],
            "agent_never has no signal"
        );
        assert_eq!(overview.agents[1].triggers[0].trigger_type, "schedule");
        assert_eq!(overview.totals.running_sessions, 2);
        assert_eq!(overview.totals.runs, 7);
        assert_eq!(overview.totals.failed, 1);
        assert_eq!(overview.totals.channel_sessions, 4);
        assert_eq!(overview.totals.max_active_turns, 1000);
    }

    #[test]
    fn empty_org_has_empty_series() {
        let overview = build_overview(AgentActivityRows::default(), Utc::now(), 50);
        assert!(overview.agents.is_empty());
        assert!(overview.channels.is_empty());
        assert_eq!(overview.totals.runs, 0);
        assert_eq!((overview.hours, overview.days), (24, 7));
    }
}
