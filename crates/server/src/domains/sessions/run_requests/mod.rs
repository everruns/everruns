//! Requests from the other agents in a run reach the run's first session.
//!
//! Spec: `knowledge/runtime-resources/agents-using-agents.md` (Approvals and
//! questions).

// Decision: a question or approval from any agent the first session handed
// work to is shown and answered on the first session, the one the person talks
// to. The session page is for debugging and never asks. Answering on the
// asking agent's own session keeps working; the first answer wins.
// Decision: requests are read, not copied. A waiting session's last
// `tool.call_requested` is the only record of what it waits on, so the first
// session lists its waiting descendants (`root_session_id`) and reads theirs.
// Nothing is mirrored into the first session's events, where the runtime
// would read a copy as its own pending batch.
// Decision: only the first session sees the run's requests. An agent in the
// middle of the run is not where a person talks.
// THREAT[TM-AGENT-039]: an answer reaches a descendant only through the same
// validation as on that session: the decision binds to what the descendant's
// own event asked, never to the body. An agent cannot answer for the person,
// because these surfaces take a person's (or a key holder's) answer only.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use everruns_contracts::openai_hosted_tools::OPENAI_MCP_APPROVAL_TOOL;
use everruns_contracts::tool_types::{
    ASK_USER_TOOL_NAME, CONFIRM_URL_ELICITATION_TOOL, ToolApprovalRequired,
};
use everruns_contracts::typed_id::{AgentId, SessionId};
use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

use crate::storage::{EventRow, StorageBackend};

/// Tool calls that wait on the person rather than on a client.
const PERSON_FACING_TOOLS: [&str; 4] = [
    ASK_USER_TOOL_NAME,
    OPENAI_MCP_APPROVAL_TOOL,
    CONFIRM_URL_ELICITATION_TOOL,
    "setup_connection",
];

/// The agent a request came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct RequestingAgent {
    /// The agent's id, as `/v1/agents/{agent_id}` takes it.
    #[schema(example = "agent_01933b5a00007000800000000000001")]
    pub id: String,
    /// The agent's display name.
    pub name: String,
}

/// A tool call another agent in the run waits on the person for.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RunRequestToolCall {
    pub id: String,
    /// `ask_user`, `approve_tool_call`, `openai_mcp_approval`,
    /// `confirm_url_elicitation` or `setup_connection`.
    pub name: String,
    #[schema(value_type = Object)]
    pub arguments: Value,
}

/// One request from another agent in the run.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RunRequest {
    /// The asking agent's session.
    #[schema(value_type = String)]
    pub session_id: SessionId,
    /// The asking agent. Absent when the agent was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<RequestingAgent>,
    pub requested_at: DateTime<Utc>,
    pub tool_call: RunRequestToolCall,
}

/// What the other agents in a run wait on the person for.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct RunRequestsResponse {
    pub requests: Vec<RunRequest>,
}

/// A session in the run that waits, with what it waits on.
pub(crate) struct WaitingAgent {
    pub(crate) session_id: SessionId,
    pub(crate) agent: Option<RequestingAgent>,
    pub(crate) requested: EventRow,
}

impl WaitingAgent {
    fn tool_calls(&self) -> impl Iterator<Item = &Value> {
        self.requested.data["tool_calls"]
            .as_array()
            .into_iter()
            .flatten()
    }

    fn waits_on(&self, tool_call_id: &str) -> bool {
        self.tool_calls()
            .any(|call| call["id"].as_str() == Some(tool_call_id))
    }
}

/// Sessions under the first session `session_id` that wait, oldest first.
/// Empty for a session that is not a run's first session.
pub(crate) async fn waiting_agents(
    db: &StorageBackend,
    org_id: i64,
    session_id: SessionId,
    is_first_session: bool,
) -> anyhow::Result<Vec<WaitingAgent>> {
    if !is_first_session {
        return Ok(Vec::new());
    }
    let sessions = db.list_waiting_run_sessions(org_id, session_id).await?;
    if sessions.is_empty() {
        return Ok(Vec::new());
    }
    let agent_ids: Vec<AgentId> = sessions.iter().filter_map(|s| s.agent_id).collect();
    let agents: HashMap<AgentId, RequestingAgent> = db
        .get_agents_by_ids(org_id, &agent_ids)
        .await?
        .into_iter()
        .map(|agent| {
            let requesting = RequestingAgent {
                id: agent.public_id,
                name: agent.display_name.unwrap_or(agent.name),
            };
            (agent.id, requesting)
        })
        .collect();
    let mut waiting = Vec::with_capacity(sessions.len());
    for session in sessions {
        let Some(requested) = last_tool_call_requested(db, session.id).await? else {
            continue;
        };
        let agent = session.agent_id.and_then(|id| agents.get(&id).cloned());
        waiting.push(WaitingAgent {
            session_id: session.id,
            agent,
            requested,
        });
    }
    Ok(waiting)
}

async fn last_tool_call_requested(
    db: &StorageBackend,
    session_id: SessionId,
) -> anyhow::Result<Option<EventRow>> {
    Ok(db
        .list_events(
            session_id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(1),
        )
        .await?
        .pop())
}

/// How many person-facing requests `sessions` wait on, each read from its last
/// `tool.call_requested`. For the whole-run view, which counts the first
/// session's own requests too.
pub(crate) async fn count_person_requests(
    db: &StorageBackend,
    sessions: &[SessionId],
) -> anyhow::Result<usize> {
    let mut waiting = Vec::with_capacity(sessions.len());
    for session_id in sessions {
        if let Some(requested) = last_tool_call_requested(db, *session_id).await? {
            waiting.push(WaitingAgent {
                session_id: *session_id,
                agent: None,
                requested,
            });
        }
    }
    Ok(run_requests(&waiting).len())
}

/// The person-facing requests of the other agents in the run.
pub(crate) fn run_requests(waiting: &[WaitingAgent]) -> Vec<RunRequest> {
    let mut requests = Vec::new();
    for agent in waiting {
        for call in agent.tool_calls() {
            let (Some(id), Some(name)) = (call["id"].as_str(), call["name"].as_str()) else {
                continue;
            };
            // THREAT[TM-TOOL-008]: an approval counts only when it is the
            // engine's own request, never a model tool that shares the name.
            let person_facing = PERSON_FACING_TOOLS.contains(&name)
                || ToolApprovalRequired::from_request_call(id, name, &call["arguments"]).is_some();
            if !person_facing {
                continue;
            }
            requests.push(RunRequest {
                session_id: agent.session_id,
                agent: agent.agent.clone(),
                requested_at: agent.requested.ts,
                tool_call: RunRequestToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    arguments: call["arguments"].clone(),
                },
            });
        }
    }
    requests
}

/// Why an answer cannot be routed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnswerTargetError {
    /// The answers name requests of two different agents.
    SpansAgents,
    Internal(String),
}

/// The session that waits on `tool_call_ids`: `session_id` itself, or the
/// agent in its run that asked. Falls back to `session_id`, so a request
/// nobody waits on gets that session's own "not found".
pub(crate) async fn answer_target(
    db: &StorageBackend,
    org_id: i64,
    session_id: SessionId,
    is_first_session: bool,
    tool_call_ids: &[&str],
) -> Result<SessionId, AnswerTargetError> {
    let internal = |error: anyhow::Error| AnswerTargetError::Internal(error.to_string());
    let Some(first) = tool_call_ids.first() else {
        return Ok(session_id);
    };
    if !is_first_session {
        return Ok(session_id);
    }
    let own = last_tool_call_requested(db, session_id)
        .await
        .map_err(internal)?;
    let own_waits_on = |id: &str| {
        own.as_ref().is_some_and(|event| {
            event.data["tool_calls"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|call| call["id"].as_str() == Some(id))
        })
    };
    if own_waits_on(first) {
        return Ok(session_id);
    }
    let waiting = waiting_agents(db, org_id, session_id, true)
        .await
        .map_err(internal)?;
    let Some(asker) = waiting.iter().find(|agent| agent.waits_on(first)) else {
        return Ok(session_id);
    };
    if tool_call_ids.iter().any(|id| !asker.waits_on(id)) {
        return Err(AnswerTargetError::SpansAgents);
    }
    Ok(asker.session_id)
}

#[cfg(test)]
mod tests;
