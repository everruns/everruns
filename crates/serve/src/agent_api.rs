//! The Agent Execution API: one agent's sessions, rooted at its agent base URL
//! `/v1/channels/{agent}`.
//!
//! Decisions:
//! - Same routes and shapes as the everruns server's API channel, from the
//!   shared `everruns::execution_api` types, so the SDK's agent client works
//!   against either host.
//! - The handlers are the `/v1/sessions` ones behind a confinement check: a
//!   session is reachable only under the agent that runs it, and any other
//!   agent's base URL answers 404, the same answer as for a session that does
//!   not exist.
//! - A subagent has no base URL; it only runs inside its parent's turns.
//! - `POST /v1/channels/{agent}` stays the channel webhook; `GET` is the card.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::Query;
use everruns::approval::ApprovalDecision;
use everruns::execution_api::{
    AgentCard, AgentCardInput, AgentCardLinks, CreateAgentSessionRequest,
    SubmitToolApprovalsRequest, SubmitToolApprovalsResponse, ToolApprovalDecision,
    ToolApprovalResolution,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AgentEntry;
use crate::host::{ApiError, Host, NewSession};
use crate::server::{
    ApiResult, CreateMessageBody, ListQuery, QuestionAnswersBody, SseQuery, bad_request,
};

/// Upper bound on decisions in one submission, as on the server.
const MAX_DECISIONS: usize = 100;
/// Default and largest page of `GET …/sessions`.
const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 200;

/// The agent base URL of `agent`.
pub(crate) fn base(agent: &str) -> String {
    format!("/v1/channels/{agent}")
}

pub(crate) fn routes(router: Router<Arc<Host>>) -> Router<Arc<Host>> {
    router
        .route(
            "/v1/channels/{name}/sessions",
            get(list_sessions).post(create_session),
        )
        .route("/v1/channels/{name}/sessions/{id}", get(get_session))
        .route(
            "/v1/channels/{name}/sessions/{id}/messages",
            post(create_message),
        )
        .route("/v1/channels/{name}/sessions/{id}/cancel", post(cancel))
        .route("/v1/channels/{name}/sessions/{id}/sse", get(sse))
        .route("/v1/channels/{name}/sessions/{id}/events", get(list_events))
        .route(
            "/v1/channels/{name}/sessions/{id}/question-answers",
            post(question_answers),
        )
        .route(
            "/v1/channels/{name}/sessions/{id}/tool-approvals",
            post(tool_approvals),
        )
        .route("/v1/sessions/{id}/tool-approvals", post(any_tool_approvals))
}

/// The top-level agent `name`, or 404.
fn top_level<'a>(host: &'a Host, name: &str) -> ApiResult<&'a AgentEntry> {
    host.app
        .agent(name)
        .filter(|entry| !entry.sub)
        .ok_or_else(|| ApiError::NotFound(format!("agent {name}")).into())
}

/// 404 unless session `id` runs agent `name`.
fn confine(host: &Host, name: &str, id: &str) -> ApiResult<()> {
    top_level(host, name)?;
    if host.session_row(id)?.agent != name {
        return Err(ApiError::NotFound(format!("session {id}")).into());
    }
    Ok(())
}

/// `GET /v1/channels/{agent}`: what a caller needs to start talking to it.
pub(crate) async fn card(
    State(host): State<Arc<Host>>,
    Path(name): Path<String>,
) -> ApiResult<Json<AgentCard>> {
    let entry = top_level(&host, &name)?;
    // As the app manifest describes it.
    let description = entry
        .spec
        .description
        .clone()
        .or_else(|| (!entry.doc.is_empty()).then(|| entry.doc.clone()));
    #[cfg(feature = "ag-ui")]
    let ag_ui = Some(crate::ag_ui::route(&name));
    #[cfg(not(feature = "ag-ui"))]
    let ag_ui = None;
    #[cfg(feature = "a2a")]
    let a2a = Some(format!(
        "{}/.well-known/agent-card.json",
        crate::a2a::route(&name)
    ));
    #[cfg(not(feature = "a2a"))]
    let a2a = None;
    Ok(Json(AgentCard {
        name: name.clone(),
        description,
        streaming: true,
        input: AgentCardInput::TEXT,
        // serve's API is open; an auth hook comes with the channel verifier.
        auth: Vec::new(),
        conversation_starters: Vec::new(),
        links: AgentCardLinks {
            sessions: format!("{}/sessions", base(&name)),
            ag_ui,
            a2a,
        },
    }))
}

async fn create_session(
    State(host): State<Arc<Host>>,
    Path(name): Path<String>,
    body: Option<Json<CreateAgentSessionRequest>>,
) -> ApiResult<Response> {
    top_level(&host, &name)?;
    let body = body.map(|b| b.0).unwrap_or_default();
    let id = host
        .create_session(NewSession {
            agent: Some(name.clone()),
            title: body.title,
            tags: Vec::new(),
            hints: None,
            metadata: body.metadata,
        })
        .await?;
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, format!("{}/sessions/{id}", base(&name)))],
        Json(crate::server::session_json(&host, &id)?),
    )
        .into_response())
}

#[derive(Deserialize, Default)]
struct ListSessionsQuery {
    #[serde(default)]
    limit: Option<usize>,
}

/// `GET /v1/channels/{agent}/sessions`: the agent's sessions, most recently
/// active first. serve has one caller, so that is every session of the agent.
async fn list_sessions(
    State(host): State<Arc<Host>>,
    Path(name): Path<String>,
    Query(query): Query<ListSessionsQuery>,
) -> ApiResult<Json<Value>> {
    top_level(&host, &name)?;
    let limit = query.limit.unwrap_or(DEFAULT_LIST_LIMIT);
    if !(1..=MAX_LIST_LIMIT).contains(&limit) {
        return Err(bad_request(format!(
            "limit must be between 1 and {MAX_LIST_LIMIT}"
        )));
    }
    let data = host
        .session_ids_for_agent(&name, limit)?
        .iter()
        .map(|id| crate::server::session_json(&host, id))
        .collect::<crate::Result<Vec<_>>>()?;
    Ok(Json(json!({ "data": data })))
}

async fn get_session(
    State(host): State<Arc<Host>>,
    Path((name, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    confine(&host, &name, &id)?;
    crate::server::get_session(State(host), Path(id)).await
}

async fn create_message(
    State(host): State<Arc<Host>>,
    Path((name, id)): Path<(String, String)>,
    body: Json<CreateMessageBody>,
) -> ApiResult<Response> {
    confine(&host, &name, &id)?;
    crate::server::create_message(State(host), Path(id), body).await
}

async fn cancel(
    State(host): State<Arc<Host>>,
    Path((name, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    confine(&host, &name, &id)?;
    crate::server::cancel(State(host), Path(id)).await
}

async fn sse(
    State(host): State<Arc<Host>>,
    Path((name, id)): Path<(String, String)>,
    query: Query<SseQuery>,
) -> ApiResult<Response> {
    confine(&host, &name, &id)?;
    Ok(crate::server::sse(State(host), Path(id), query)
        .await?
        .into_response())
}

async fn list_events(
    State(host): State<Arc<Host>>,
    Path((name, id)): Path<(String, String)>,
    query: Query<ListQuery>,
) -> ApiResult<Response> {
    confine(&host, &name, &id)?;
    crate::server::list_events(State(host), Path(id), query).await
}

async fn question_answers(
    State(host): State<Arc<Host>>,
    Path((name, id)): Path<(String, String)>,
    body: Json<QuestionAnswersBody>,
) -> ApiResult<Json<Value>> {
    confine(&host, &name, &id)?;
    crate::server::question_answers(State(host), Path(id), body).await
}

async fn tool_approvals(
    State(host): State<Arc<Host>>,
    Path((name, id)): Path<(String, String)>,
    body: Json<SubmitToolApprovalsRequest>,
) -> ApiResult<Json<SubmitToolApprovalsResponse>> {
    confine(&host, &name, &id)?;
    any_tool_approvals(State(host), Path(id), body).await
}

/// `POST /v1/sessions/{id}/tool-approvals`, the server's batch shape.
///
/// A serve session runs one turn at a time, so every approval pending on the
/// session belongs to the turn being answered. One left out of the
/// submission is settled as not approved, as on the server: the turn resumes
/// once, and silence never approves.
pub(crate) async fn any_tool_approvals(
    State(host): State<Arc<Host>>,
    Path(id): Path<String>,
    Json(body): Json<SubmitToolApprovalsRequest>,
) -> ApiResult<Json<SubmitToolApprovalsResponse>> {
    host.session_row(&id)?;
    // After a restart, the approvals are pending again once the session is.
    host.session(&id).await?;
    if body.decisions.is_empty() {
        return Err(bad_request("decisions must not be empty"));
    }
    if body.decisions.len() > MAX_DECISIONS {
        return Err(bad_request(format!(
            "at most {MAX_DECISIONS} decisions per submission"
        )));
    }
    let pending = host.pending_approvals(&id);
    let mut seen = std::collections::HashSet::new();
    for answer in &body.decisions {
        if !seen.insert(answer.tool_call_id.as_str()) {
            return Err(bad_request(format!(
                "tool_call_id {} appears more than once",
                answer.tool_call_id
            )));
        }
        if !pending
            .iter()
            .any(|view| view.tool_call_id == answer.tool_call_id)
        {
            return Err(
                ApiError::NotFound(format!("pending approval {}", answer.tool_call_id)).into(),
            );
        }
    }
    let mut resolved = Vec::with_capacity(pending.len());
    for view in pending {
        let answer = body
            .decisions
            .iter()
            .find(|answer| answer.tool_call_id == view.tool_call_id);
        let (decision, outcome) = match answer.map(|answer| answer.decision) {
            Some(ToolApprovalDecision::Allow) => (ApprovalDecision::Allow, "allow"),
            Some(ToolApprovalDecision::AllowAlways) => {
                (ApprovalDecision::AllowAlways, "allow_always")
            }
            Some(ToolApprovalDecision::Reject) => (ApprovalDecision::Reject, "reject"),
            Some(ToolApprovalDecision::RejectAlways) => {
                (ApprovalDecision::RejectAlways, "reject_always")
            }
            None => (ApprovalDecision::Reject, "not_approved"),
        };
        // Settled meanwhile by another caller: that answer stands.
        if host
            .decide_approval(&id, &view.tool_call_id, decision)
            .is_ok()
        {
            resolved.push(ToolApprovalResolution {
                tool_call_id: view.tool_call_id,
                tool: view.tool_name,
                outcome: outcome.to_string(),
            });
        }
    }
    Ok(Json(SubmitToolApprovalsResponse {
        resolved,
        status: host.status(&id).to_string(),
    }))
}
