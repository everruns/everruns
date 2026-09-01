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

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use everruns_core::events::{EventContext, EventRequest, ToolCompletedData};
use everruns_core::message::ContentPart;
use everruns_mcp::{StoredConsent, consent_storage_key};
use everruns_platform::SessionStatus;
use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::common::{ApiOptionExt, ApiResult, ApiResultExt, ErrorResponse};
use super::tool_results::AppState;
use crate::storage::models::UpsertSessionKeyValue;
use everruns_core::Caller;
use everruns_provider::tool_types::CONFIRM_URL_ELICITATION_TOOL;

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
        (status = 200, description = "Decision recorded and workflow resumed", body = ElicitationConsentResponse),
        (status = 400, description = "Invalid session ID or request"),
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

    let caller = Caller::from(&org);
    let session = state
        .session_service
        .get(&caller, session_id.uuid(), None)
        .await
        .log_internal_error_json("get session")?
        .ok_or_not_found_json("Session")?;

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

    // Record before resuming. The tool call that reads this consent can start
    // as soon as the workflow is running again, so a consent written after the
    // resume could arrive too late to be seen.
    if req.action == ConsentAction::Accept {
        let record = StoredConsent::new(
            &pending.server,
            &pending.tool,
            &pending.host,
            chrono::Utc::now(),
        );
        state
            .db
            .upsert_session_key_value(UpsertSessionKeyValue {
                session_id,
                key: consent_storage_key(&pending.server, &pending.tool),
                value: serde_json::to_string(&record).unwrap_or_default(),
            })
            .await
            .log_internal_error_json("record elicitation consent")?;
    }

    let turn_id = TurnId::from_uuid(session_id.uuid());
    let event_message_id = MessageId::from_uuid(session_id.uuid());

    // What the model reads. On an accept it needs to know the retry is now
    // worth making; on a decline it needs to stop asking.
    let summary = match req.action {
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
    };

    let completed = ToolCompletedData::success(
        req.tool_call_id.clone(),
        CONFIRM_URL_ELICITATION_TOOL.to_string(),
        vec![ContentPart::tool_result_text(&summary)],
        None,
    );

    if let Err(e) = state
        .event_service
        .emit(EventRequest::new(
            session_id,
            EventContext::turn(turn_id, event_message_id),
            completed,
        ))
        .await
    {
        tracing::warn!(
            session_id = %session_id,
            tool_call_id = %req.tool_call_id,
            error = %e,
            "Failed to emit tool.completed event for elicitation consent"
        );
    }

    if let Err(e) = state
        .session_service
        .update_status(&caller, session_id.uuid(), "active".to_string())
        .await
    {
        tracing::warn!(error = %e, "Failed to set session status to active");
    }

    tracing::info!(
        session_id = %session_id,
        server = %pending.server,
        tool = %pending.tool,
        host = %pending.host,
        url = %pending.url,
        action = ?req.action,
        "URL elicitation consent recorded"
    );

    let runner = state.runner.clone();
    tokio::spawn(async move {
        if let Err(e) = runner.resume_after_tool_results(session_id).await {
            tracing::error!(
                session_id = %session_id,
                error = %e,
                "Failed to resume workflow after elicitation consent"
            );
        }
    });

    Ok(Json(ElicitationConsentResponse {
        host: pending.host,
        status: "active".to_string(),
    }))
}

/// Recover what was actually asked, from the event that asked it.
///
/// The request body names only the tool call; the server, tool, and domain come
/// from the `confirm_url_elicitation` call Everruns itself emitted, so a client
/// cannot record consent for a domain the user was never shown.
async fn find_pending_elicitation(
    state: &AppState,
    session_id: SessionId,
    tool_call_id: &str,
) -> anyhow::Result<Option<PendingElicitation>> {
    let events = state
        .db
        .list_events(
            session_id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(ELICITATION_LOOKBACK_EVENTS),
        )
        .await?;

    for event in events.iter().rev() {
        let Some(tool_calls) = event.data.get("tool_calls").and_then(|v| v.as_array()) else {
            continue;
        };
        for call in tool_calls {
            if call.get("id").and_then(|v| v.as_str()) != Some(tool_call_id) {
                continue;
            }
            if call.get("name").and_then(|v| v.as_str()) != Some(CONFIRM_URL_ELICITATION_TOOL) {
                return Ok(None);
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
                return Ok(None);
            };
            return Ok(Some(PendingElicitation {
                retry_tool: field("retry_tool").unwrap_or_else(|| tool.clone()),
                server,
                tool,
                host,
                url: field("url").unwrap_or_default(),
            }));
        }
    }
    Ok(None)
}
