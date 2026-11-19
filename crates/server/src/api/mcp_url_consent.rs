// Consent for a URL mode elicitation raised by an MCP server we are a client of.
//
// The counterpart of `api::mcp_elicitation`, which serves the pages for
// elicitations Everruns *raises* over `/mcp`. Here Everruns is the client: a
// server answered a `tools/call` by asking that a person visit a URL, the turn
// paused on a `confirm_url_elicitation` card, and this endpoint carries what the
// person decided back into the run.
//
// Two things happen on an accept, and the order matters:
//
//  1. The consent is recorded in session storage, because the tool call that
//     will answer the server `accept` runs later and possibly elsewhere.
//  2. The synthetic tool call is completed and the workflow resumes.
//
// The consent names the server, the tool, and the domain the user was actually
// shown, all read back out of the emitted `tool.call_requested` event rather
// than from the request body: the browser posting this decision does not get to
// say what was consented to.

use crate::domains::sessions::record::SessionStatus;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use everruns_contracts::session_commands;
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::{EventRequest, ToolCompletedData};
use everruns_core::mcp::{StoredConsent, consent_storage_key};
use everruns_core::message::ContentPart;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::common::{ApiOptionExt, ApiResult, ApiResultExt, ErrorResponse};
use super::tool_results::AppState;
use crate::domains::tool_results::waiting_turn_resolution::execute_waiting_turn_resolution;
use crate::storage::{ClaimWaitingTurnResult, WaitingTurnResolutionPlan, WaitingTurnSessionValue};
use everruns_contracts::tool_types::CONFIRM_URL_ELICITATION_TOOL;
use everruns_core::Caller;

/// How far back to look for the elicitation being answered. The card is emitted
/// by the act that just paused, so it is within the last handful of events; the
/// window only needs to be generous enough to survive a chatty turn.
const ELICITATION_LOOKBACK_EVENTS: i32 = 200;

/// What the user decided about opening the URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConsentAction {
    /// The user opened the URL and finished whatever the server asked for.
    Accept,
    /// The user refused. Nothing is recorded and the tool is not retried.
    Decline,
}

/// Request to answer a pending URL mode elicitation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ElicitationConsentRequest {
    /// The `confirm_url_elicitation` tool call being answered.
    #[schema(example = "url_elicitation_01933b5a00007000800000000000001")]
    pub tool_call_id: String,
    /// The user's decision.
    pub action: ConsentAction,
}

/// Result of answering a pending URL mode elicitation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ElicitationConsentResponse {
    /// Domain the consent was recorded against, echoed so a client can confirm
    /// it showed the same one.
    pub host: String,
    /// Session status after the decision.
    pub status: String,
}

/// Routes for answering URL mode elicitations. Merged into the tool-results
/// router because it is the same pause-and-resume surface.
pub fn routes() -> axum::Router<AppState> {
    axum::Router::new().route(
        "/v1/sessions/{session_id}/mcp-elicitation-consent",
        post(submit_elicitation_consent),
    )
}

/// The parts of a pending elicitation that only the server may decide.
struct PendingElicitation {
    server: String,
    tool: String,
    retry_tool: String,
    host: String,
    url: String,
}

/// POST /v1/sessions/{session_id}/mcp-elicitation-consent
///
/// Records a user's decision about a URL an MCP server asked them to open, and
/// resumes the paused turn.
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/mcp-elicitation-consent",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    request_body = ElicitationConsentRequest,
    responses(
        (status = 200, description = "Decision recorded and turn made runnable", body = ElicitationConsentResponse),
        (status = 400, description = "Invalid session ID or request"),
        (status = 403, description = "Caller may not manage this session"),
        (status = 404, description = "Session or pending elicitation not found"),
        (status = 409, description = "Session is not waiting for tool results"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn submit_elicitation_consent(
    org: crate::auth::ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<ElicitationConsentRequest>,
) -> ApiResult<ElicitationConsentResponse> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid session ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    // Consenting resumes the turn and stores a consent the retry acts on, so it
    // takes the same authority as any other session write. Both checks run
    // before the pending elicitation is read, so a refusal reveals nothing
    // about it.
    let caller = Caller::from(&org);
    crate::domains::sessions::SESSION_MANAGE
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(|error| {
            ErrorResponse::new(error.to_string()).into_response(StatusCode::FORBIDDEN)
        })?;
    let session = state
        .session_service
        .get(&caller, session_id.uuid(), None)
        .await
        .log_internal_error_json("get session")?
        .ok_or_not_found_json("Session")?;
    if !crate::domains::sessions::platform_chat_owner_matches_session(&state.db, &caller, &session)
        .await
        .log_internal_error_json("authorize session owner")?
    {
        // THREAT[TM-AGENT-017]: a URL consent resumes Platform Chat like any
        // other answer and must bind to its persisted owner too.
        return Err(ErrorResponse::not_found("Session"));
    }

    // A v2 session's elicitation and its parked turn are in its cell, which
    // checks that the call is still pending.
    if let Some(storage) =
        crate::domains::session_storage::plane::PlaneStorage::on(&state.db, org.org_id, session_id)
            .await
            .log_internal_error_json("find the session's data plane")?
    {
        return consent_on_data_plane(&storage, &req).await;
    }

    if session.status != SessionStatus::WaitingForToolResults {
        return Err(ErrorResponse::new(format!(
            "Session is not waiting for tool results (current status: {})",
            session.status
        ))
        .into_response(StatusCode::CONFLICT));
    }

    let pending = find_pending_elicitation(&state, session_id, &req.tool_call_id)
        .await
        .log_internal_error_json("read session events")?
        .ok_or_not_found_json("Pending URL elicitation")?;
    let session_values = if req.action == ConsentAction::Accept {
        let record = StoredConsent::new(
            &pending.server,
            &pending.tool,
            &pending.host,
            chrono::Utc::now(),
        );
        vec![WaitingTurnSessionValue {
            key: consent_storage_key(&pending.server, &pending.tool),
            value: serde_json::to_string(&record).unwrap_or_default(),
        }]
    } else {
        Vec::new()
    };
    // Recorded under the waiting turn, which the worker resumes.
    let context = crate::turns::open_turn_context(&state.db, session_id)
        .await
        .log_internal_error_json("read the waiting turn")?;

    // What the model reads. On an accept it needs to know the retry is now
    // worth making; on a decline it needs to stop asking.
    let summary = consent_summary(req.action, &pending);

    let completed = ToolCompletedData::success(
        req.tool_call_id.clone(),
        CONFIRM_URL_ELICITATION_TOOL.to_string(),
        vec![ContentPart::tool_result_text(&summary)],
        None,
    );

    // The decision also goes in as a turn of the conversation.
    //
    // The tool.completed above is what closes the card in the UI, but it cannot
    // carry the decision to the model: the synthetic call was emitted by the
    // engine, never by the model, so nothing in the transcript claims that tool
    // call and the lone result is dropped before the request is built. Saying it
    // as the user is both what reaches the model and what actually happened —
    // they answered, with a button instead of the keyboard.
    // Written the way the user would say it, without internal tool ids: the
    // model already knows which call it just made, and this line is read by a
    // person in the transcript.
    let spoken = consent_spoken(req.action, &pending);

    // Inherit the run's controls. A turn's model can be pinned per message, and
    // a decision that arrived without them would resume the turn on whatever the
    // org default resolves to — switching provider mid-conversation, carrying
    // the other provider's response ids with it.
    let mut message = everruns_core::message::RuntimeMessage::user(spoken);
    message.controls = latest_user_controls(&state, session_id).await;
    let plan = WaitingTurnResolutionPlan {
        kind: "url_consent".to_string(),
        events: vec![
            EventRequest::new(
                session_id,
                context.clone(),
                everruns_core::events::InputMessageData::new(message),
            ),
            EventRequest::new(session_id, context, completed),
        ],
        session_values,
        response: serde_json::json!({ "host": pending.host }),
    };
    let claim = match state
        .db
        .claim_waiting_turn(org.org_id, session_id, plan)
        .await
        .log_internal_error_json("claim waiting turn")?
    {
        ClaimWaitingTurnResult::Claimed(claim) => claim,
        ClaimWaitingTurnResult::Conflict { current_status } => {
            return Err(ErrorResponse::new(format!(
                "Session is not waiting for tool results (current status: {current_status})"
            ))
            .into_response(StatusCode::CONFLICT));
        }
        ClaimWaitingTurnResult::SessionNotFound => {
            return Err(ErrorResponse::not_found("Session"));
        }
    };
    let result = execute_waiting_turn_resolution(
        &state.db,
        &state.event_service,
        &state.runner,
        org.org_id,
        session_id,
        &claim,
    )
    .await;
    let host = claim.plan.response["host"]
        .as_str()
        .unwrap_or(&pending.host)
        .to_string();
    result.log_internal_error_json("resolve URL elicitation consent")?;

    tracing::info!(
        session_id = %session_id,
        server = %pending.server,
        tool = %pending.tool,
        host = %host,
        url = %pending.url,
        action = ?req.action,
        "URL elicitation consent recorded"
    );

    Ok(Json(ElicitationConsentResponse {
        host,
        status: "active".to_string(),
    }))
}

/// Controls from the most recent user message in this session, if any.
///
/// The decision continues that person's turn, so it has to run under the same
/// model and reasoning settings they were talking to.
async fn latest_user_controls(
    state: &AppState,
    session_id: SessionId,
) -> Option<everruns_core::message::Controls> {
    let events = crate::domains::sessions::events::list(
        &state.db,
        session_id,
        None,
        None,
        &["input.message".to_string()],
        &[],
        None,
        Some(ELICITATION_LOOKBACK_EVENTS),
    )
    .await
    .ok()?;
    events.iter().rev().find_map(|event| {
        serde_json::from_value::<everruns_core::message::Controls>(
            event.data.get("message")?.get("controls")?.clone(),
        )
        .ok()
    })
}

/// Recover what was actually asked, from the event that asked it.
///
/// THREAT[TM-TOOL-035]: the browser posting a decision does not get to say what
/// was consented to.
///
/// The request body names only the tool call; the server, tool, and domain come
/// from the `confirm_url_elicitation` call Everruns itself emitted, so a client
/// cannot record consent for a domain the user was never shown.
async fn find_pending_elicitation(
    state: &AppState,
    session_id: SessionId,
    tool_call_id: &str,
) -> anyhow::Result<Option<PendingElicitation>> {
    let events = crate::domains::sessions::events::list(
        &state.db,
        session_id,
        None,
        None,
        &["tool.call_requested".to_string()],
        &[],
        None,
        Some(ELICITATION_LOOKBACK_EVENTS),
    )
    .await?;

    Ok(pending_in(
        events.iter().rev().map(|e| &e.data),
        tool_call_id,
    ))
}

/// The pending elicitation `tool_call_id` among `tool.call_requested`
/// payloads, newest first.
fn pending_in<'a>(
    payloads: impl Iterator<Item = &'a serde_json::Value>,
    tool_call_id: &str,
) -> Option<PendingElicitation> {
    for data in payloads {
        let Some(tool_calls) = data.get("tool_calls").and_then(|v| v.as_array()) else {
            continue;
        };
        for call in tool_calls {
            if call.get("id").and_then(|v| v.as_str()) != Some(tool_call_id) {
                continue;
            }
            if call.get("name").and_then(|v| v.as_str()) != Some(CONFIRM_URL_ELICITATION_TOOL) {
                return None;
            }
            let arguments = call.get("arguments").cloned().unwrap_or_default();
            let field = |name: &str| {
                arguments
                    .get(name)
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            };
            let (Some(server), Some(tool), Some(host)) =
                (field("server"), field("tool"), field("url_host"))
            else {
                return None;
            };
            return Some(PendingElicitation {
                retry_tool: field("retry_tool").unwrap_or_else(|| tool.clone()),
                server,
                tool,
                host,
                url: field("url").unwrap_or_default(),
            });
        }
    }
    None
}

/// What the model reads: on an accept, that the retry is now worth making;
/// on a decline, to stop asking.
fn consent_summary(action: ConsentAction, pending: &PendingElicitation) -> serde_json::Value {
    match action {
        ConsentAction::Accept => serde_json::json!({
            "action": "accept",
            "server": pending.server,
            "host": pending.host,
            "message": format!(
                "The user opened {} and completed what {} asked for. Call '{}' again now; \
                 do not ask them to open the link again.",
                pending.host, pending.server, pending.retry_tool
            ),
        }),
        ConsentAction::Decline => serde_json::json!({
            "action": "decline",
            "server": pending.server,
            "host": pending.host,
            "message": format!(
                "The user declined to open {}, so '{}' cannot run. Continue without it and \
                 do not ask again.",
                pending.host, pending.retry_tool
            ),
        }),
    }
}

/// The decision as the person would say it, without internal tool ids.
fn consent_spoken(action: ConsentAction, pending: &PendingElicitation) -> String {
    match action {
        ConsentAction::Accept => format!(
            "I opened {} and finished what {} asked for — go ahead.",
            pending.host, pending.server
        ),
        ConsentAction::Decline => format!(
            "I'd rather not open {}. Carry on without it, and don't ask again.",
            pending.host
        ),
    }
}

/// [`submit_elicitation_consent`] for a session on a data plane (step
/// 11.6): the pending elicitation comes from the cell's events, an accept's
/// consent goes into the session's storage on its cell for the retried
/// call, the card's call is answered with the summary, and the decision
/// follows as the person's message (the model never made the synthetic
/// call, so its result alone would not reach it). The cell refuses a call
/// that is not pending any more.
async fn consent_on_data_plane(
    storage: &crate::domains::session_storage::plane::PlaneStorage,
    req: &ElicitationConsentRequest,
) -> ApiResult<ElicitationConsentResponse> {
    let plane = storage.plane();
    let requested = plane
        .events(
            1,
            i32::MAX,
            &["tool.call_requested"],
            Some(ELICITATION_LOOKBACK_EVENTS as usize),
        )
        .await
        .log_internal_error_json("read session events")?;
    let pending = pending_in(requested.iter().rev().map(|e| &e.data), &req.tool_call_id)
        .ok_or_not_found_json("Pending URL elicitation")?;
    if req.action == ConsentAction::Accept {
        let record = StoredConsent::new(
            &pending.server,
            &pending.tool,
            &pending.host,
            chrono::Utc::now(),
        );
        let record = serde_json::to_string(&record).log_internal_error_json("record consent")?;
        storage
            .set_value(
                &consent_storage_key(&pending.server, &pending.tool),
                &record,
            )
            .await
            .map_err(|error| {
                ErrorResponse::new(error.to_string())
                    .into_response(StatusCode::INTERNAL_SERVER_ERROR)
            })?;
    }
    let command = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, req.tool_call_id.as_bytes());
    let body = serde_json::json!({
        "command": command,
        "tool_results": [{
            "tool_call_id": req.tool_call_id,
            "result": consent_summary(req.action, &pending),
        }],
    });
    if let Err(error) = plane.command(&session_commands::TOOL_RESULTS, &body).await {
        let status = error
            .downcast_ref::<reqwest::Error>()
            .and_then(reqwest::Error::status);
        if status == Some(reqwest::StatusCode::CONFLICT) {
            return Err(
                ErrorResponse::new("Session is not waiting for this elicitation")
                    .into_response(StatusCode::CONFLICT),
            );
        }
        return Err(error).log_internal_error_json("resolve URL elicitation consent");
    }
    plane
        .say(
            &format!("consent.{}", req.tool_call_id),
            &consent_spoken(req.action, &pending),
        )
        .await
        .log_internal_error_json("say the decision")?;
    Ok(Json(ElicitationConsentResponse {
        host: pending.host,
        status: "active".to_string(),
    }))
}
