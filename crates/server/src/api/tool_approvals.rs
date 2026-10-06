// Answering a hard tool-approval request (EVE-1140).
//
// Spec: knowledge/execution/tool-approval.md.
//
// The `tool_approval` gate deferred a call it had no decision for, the engine
// parked the turn on a synthetic `approve_tool_call` call, and this endpoint
// carries what a person decided back into the run. Same pause-and-resume
// surface as `mcp_url_consent` and `question_answers`, and it follows the URL
// consent shape because the request is engine-authored:
//
//  1. The decision is recorded in session storage under the reserved
//     `tool_approval/` prefix, because the call it unlocks runs later and
//     possibly in another worker process: an "always" rule per (session, tool),
//     or a one-off answer bound to the exact call's fingerprint.
//  2. The synthetic call is completed (closing the card) and the decision goes
//     in as a user turn, which is what reaches the model, and the turn resumes.
//
// Everything that says *what* was approved — tool, fingerprint, deadline — is
// read back out of the emitted `tool.call_requested` event, never taken from
// the request body: the browser posting the decision does not get to say what
// it approved.

use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use chrono::{DateTime, Utc};
use everruns_contracts::tool_types::{APPROVE_TOOL_CALL_TOOL, ToolApprovalRequired};
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::Caller;
use everruns_core::builtins::{
    StoredToolApproval, always_decision_storage_key, one_off_decision_storage_key,
};
use everruns_core::events::{EventContext, EventRequest, InputMessageData, ToolCompletedData};
use everruns_core::message::{ContentPart, RuntimeMessage};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::common::{ApiOptionExt, ApiResult, ApiResultExt, ErrorResponse};
use super::tool_results::AppState;
use crate::services::EventService;
use crate::services::waiting_turn_resolution::execute_waiting_turn_resolution;
use crate::storage::StorageBackend;
use crate::storage::models::{
    ClaimWaitingTurnResult, EventRow, WaitingTurnResolutionPlan, WaitingTurnSessionValue,
};

/// How far back to look for the request being answered. Matches the sibling
/// answer surfaces: the card is emitted by the act that just paused.
const APPROVAL_LOOKBACK_EVENTS: i32 = 200;

/// Upper bound on decisions in one submission. One act cannot gate more calls
/// than it ran, and the scheduler caps those far below this.
const MAX_DECISIONS: usize = 64;

/// What a person decided about one gated call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalDecision {
    /// Run this call, once.
    Allow,
    /// Run this call, and every later call of the same tool in this session.
    AllowAlways,
    /// Do not run this call.
    Reject,
    /// Do not run this call, nor any later call of the same tool in this session.
    RejectAlways,
}

/// One decision in a submission.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ToolApprovalAnswer {
    /// The `approve_tool_call` call being answered.
    #[schema(example = "tool_approval_toolu_01933b5a00007000800000000000001")]
    pub tool_call_id: String,
    /// The person's decision.
    pub decision: ToolApprovalDecision,
}

/// Request to answer pending tool-approval requests.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SubmitToolApprovalsRequest {
    /// Decisions for the pending requests. A pending request in the same batch
    /// that is left out is resolved as not approved: the turn resumes once, so
    /// every request in it is settled now, and silence never approves.
    pub decisions: Vec<ToolApprovalAnswer>,
}

/// How one pending request was settled.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ToolApprovalResolution {
    /// The `approve_tool_call` call that was answered.
    pub tool_call_id: String,
    /// The gated tool.
    pub tool: String,
    /// `allow`, `allow_always`, `reject`, `reject_always`, `not_approved`
    /// (left out of the submission) or `expired`.
    #[schema(example = "allow")]
    pub outcome: String,
}

/// Result of answering tool-approval requests.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SubmitToolApprovalsResponse {
    /// How every pending request in the batch was settled.
    pub resolved: Vec<ToolApprovalResolution>,
    /// Session status after the decision.
    #[schema(example = "active")]
    pub status: String,
}

/// Routes for answering tool-approval requests. Merged into the tool-results
/// router because it is the same pause-and-resume surface.
pub fn routes() -> axum::Router<AppState> {
    axum::Router::new().route(
        "/v1/sessions/{session_id}/tool-approvals",
        post(submit_tool_approvals),
    )
}

/// A pending request, recovered from the event that raised it.
#[derive(Debug, Clone)]
pub(crate) struct PendingApproval {
    /// Id of the synthetic `approve_tool_call` call.
    pub(crate) request_call_id: String,
    pub(crate) request: ToolApprovalRequired,
    pub(crate) expires_at: Option<DateTime<Utc>>,
}

/// Every approval request in the current `tool.call_requested` batch.
///
/// THREAT[TM-TOOL-008]: only engine-authored calls count — the name, the id
/// prefix and the payload discriminator must all match — so a model-authored
/// client tool that happens to be called `approve_tool_call` is never answered
/// as an approval.
pub(crate) fn pending_approvals_from_events(events: &[EventRow]) -> Vec<PendingApproval> {
    let Some(event) = events.last() else {
        return Vec::new();
    };
    let Some(tool_calls) = event.data.get("tool_calls").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    tool_calls
        .iter()
        .filter_map(|call| {
            let id = call.get("id")?.as_str()?;
            let name = call.get("name")?.as_str()?;
            let request =
                ToolApprovalRequired::from_request_call(id, name, call.get("arguments")?)?;
            let expires_at = DateTime::parse_from_rfc3339(&request.expires_at)
                .ok()
                .map(|value| value.with_timezone(&Utc));
            Some(PendingApproval {
                request_call_id: id.to_string(),
                request,
                expires_at,
            })
        })
        .collect()
}

/// Read the approval requests the session is currently parked on.
pub(crate) async fn pending_tool_approvals(
    db: &StorageBackend,
    session_id: SessionId,
) -> anyhow::Result<Vec<PendingApproval>> {
    let requested = db
        .list_events(
            session_id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(1),
        )
        .await?;
    Ok(pending_approvals_from_events(&requested))
}

/// How a pending request ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApprovalOutcome {
    Decided(ToolApprovalDecision),
    /// Left out of a submission that answered its siblings.
    NotApproved,
    /// Nobody answered before the deadline.
    Expired,
}

impl ApprovalOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Decided(ToolApprovalDecision::Allow) => "allow",
            Self::Decided(ToolApprovalDecision::AllowAlways) => "allow_always",
            Self::Decided(ToolApprovalDecision::Reject) => "reject",
            Self::Decided(ToolApprovalDecision::RejectAlways) => "reject_always",
            Self::NotApproved => "not_approved",
            Self::Expired => "expired",
        }
    }

    fn allows(self) -> bool {
        matches!(
            self,
            Self::Decided(ToolApprovalDecision::Allow | ToolApprovalDecision::AllowAlways)
        )
    }

    /// What is recorded for the retried call to find. Silence records nothing:
    /// a retry asks again rather than inheriting a decision nobody made.
    fn stored_record(
        self,
        request: &ToolApprovalRequired,
        now: DateTime<Utc>,
    ) -> Option<(String, StoredToolApproval)> {
        let tool = request.tool.as_str();
        match self {
            Self::Decided(ToolApprovalDecision::Allow) => Some((
                one_off_decision_storage_key(&request.fingerprint),
                StoredToolApproval::one_off(tool, &request.fingerprint, true, now),
            )),
            Self::Decided(ToolApprovalDecision::Reject) => Some((
                one_off_decision_storage_key(&request.fingerprint),
                StoredToolApproval::one_off(tool, &request.fingerprint, false, now),
            )),
            Self::Decided(ToolApprovalDecision::AllowAlways) => Some((
                always_decision_storage_key(tool),
                StoredToolApproval::always(tool, true, now),
            )),
            Self::Decided(ToolApprovalDecision::RejectAlways) => Some((
                always_decision_storage_key(tool),
                StoredToolApproval::always(tool, false, now),
            )),
            Self::NotApproved | Self::Expired => None,
        }
    }

    /// The decision as the person would say it. The request is engine-authored,
    /// so nothing in the transcript claims the synthetic call and its result
    /// never reaches the model; this line is what does.
    fn spoken(self, tool: &str) -> String {
        match self {
            Self::Decided(ToolApprovalDecision::Allow) => format!(
                "I approved `{tool}` this once. Call it again now with exactly the same arguments."
            ),
            Self::Decided(ToolApprovalDecision::AllowAlways) => format!(
                "I approved `{tool}` for the rest of this session. Call it again now with the same arguments."
            ),
            Self::Decided(ToolApprovalDecision::Reject) => format!(
                "I rejected `{tool}`. Don't run it, and don't try to get the same result another way."
            ),
            Self::Decided(ToolApprovalDecision::RejectAlways) => {
                format!("I rejected `{tool}` for the rest of this session. Don't call it again.")
            }
            Self::NotApproved => {
                format!("I didn't approve `{tool}`, so it must not run.")
            }
            Self::Expired => format!(
                "I didn't answer the approval request for `{tool}` in time, so it is not approved. Don't run it."
            ),
        }
    }
}

/// Why a resolution did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ApprovalResolveError {
    /// The session is not parked on a tool call at all.
    NotWaiting(String),
    /// No pending approval request matches.
    NotFound(String),
    /// Something already answered this batch. First writer wins.
    AlreadyResolved,
    /// The request's deadline passed; the sweep owns it now.
    Expired,
    /// The submission is malformed.
    Invalid(String),
    /// Persistence or durable resume failed.
    Internal(String),
}

/// What resolving a batch writes through: storage, the event log, and the
/// durable runner that resumes the turn.
pub(crate) struct ApprovalServices<'a> {
    pub(crate) db: &'a std::sync::Arc<StorageBackend>,
    pub(crate) event_service: &'a EventService,
    pub(crate) runner: &'a std::sync::Arc<dyn everruns_core::host::TurnBackend>,
}

/// Settle every pending approval request on a parked session and resume it.
///
/// The one operation the API and the deadline sweep share, so validation,
/// recording and the single-claim resume cannot drift between them.
/// `outcomes` maps request call ids to how they end; any pending request it
/// does not name ends as `fallback`.
pub(crate) async fn resolve_tool_approvals(
    services: &ApprovalServices<'_>,
    org_id: i64,
    session_id: SessionId,
    pending: &[PendingApproval],
    outcomes: &HashMap<String, ApprovalOutcome>,
    fallback: ApprovalOutcome,
    kind: &str,
) -> Result<Vec<ToolApprovalResolution>, ApprovalResolveError> {
    let ApprovalServices {
        db,
        event_service,
        runner,
    } = services;
    if pending.is_empty() {
        return Err(ApprovalResolveError::NotFound(
            "No pending tool approval request".to_string(),
        ));
    }
    let now = Utc::now();
    let turn_id = TurnId::from_uuid(session_id.uuid());
    let event_message_id = MessageId::from_uuid(session_id.uuid());

    let mut session_values = Vec::new();
    let mut completions = Vec::new();
    let mut spoken = Vec::new();
    let mut resolved = Vec::new();
    for approval in pending {
        let outcome = outcomes
            .get(&approval.request_call_id)
            .copied()
            .unwrap_or(fallback);
        let request = &approval.request;
        if let Some((key, record)) = outcome.stored_record(request, now) {
            session_values.push(WaitingTurnSessionValue {
                key,
                value: serde_json::to_string(&record)
                    .map_err(|error| ApprovalResolveError::Internal(error.to_string()))?,
            });
        }
        spoken.push(outcome.spoken(&request.tool));
        let summary = serde_json::json!({
            "outcome": outcome.label(),
            "approved": outcome.allows(),
            "tool": request.tool,
            "tool_call_id": request.tool_call_id,
        });
        completions.push(EventRequest::new(
            session_id,
            EventContext::turn(turn_id, event_message_id),
            ToolCompletedData::success(
                approval.request_call_id.clone(),
                APPROVE_TOOL_CALL_TOOL.to_string(),
                vec![ContentPart::tool_result_text(&summary)],
                None,
            ),
        ));
        resolved.push(ToolApprovalResolution {
            tool_call_id: approval.request_call_id.clone(),
            tool: request.tool.clone(),
            outcome: outcome.label().to_string(),
        });
    }

    // Inherit the run's controls so the resumed turn stays on the model the
    // person was talking to (same reasoning as `mcp_url_consent`).
    let mut message = RuntimeMessage::user(spoken.join("\n"));
    message.controls = latest_user_controls(db, session_id).await;
    let mut events = vec![EventRequest::new(
        session_id,
        EventContext::empty(),
        InputMessageData::new(message),
    )];
    events.extend(completions);

    let plan = WaitingTurnResolutionPlan {
        kind: kind.to_string(),
        events,
        session_values,
        response: serde_json::to_value(
            resolved
                .iter()
                .map(|r| (r.tool_call_id.clone(), r.outcome.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default(),
    };
    let claim = match db
        .claim_waiting_turn(org_id, session_id, plan)
        .await
        .map_err(|error| ApprovalResolveError::Internal(error.to_string()))?
    {
        ClaimWaitingTurnResult::Claimed(claim) => claim,
        ClaimWaitingTurnResult::Conflict { current_status } => {
            let completed = db
                .list_events(
                    session_id,
                    None,
                    None,
                    &["tool.completed".to_string()],
                    &[],
                    None,
                    Some(APPROVAL_LOOKBACK_EVENTS),
                )
                .await
                .map_err(|error| ApprovalResolveError::Internal(error.to_string()))?;
            if completed.iter().any(|event| {
                event.data.get("tool_call_id").and_then(|v| v.as_str())
                    == Some(pending[0].request_call_id.as_str())
            }) {
                return Err(ApprovalResolveError::AlreadyResolved);
            }
            return Err(ApprovalResolveError::NotWaiting(current_status));
        }
        ClaimWaitingTurnResult::SessionNotFound => {
            return Err(ApprovalResolveError::NotFound("Session".to_string()));
        }
    };
    execute_waiting_turn_resolution(db, event_service, runner, org_id, session_id, &claim)
        .await
        .map_err(|error| ApprovalResolveError::Internal(error.to_string()))?;

    // A recovered claim replays a plan another request already stored; report
    // what that plan settled rather than what this caller asked for.
    if claim.recovered
        && let Ok(stored) =
            serde_json::from_value::<Vec<(String, String)>>(claim.plan.response.clone())
    {
        for resolution in &mut resolved {
            if let Some((_, outcome)) = stored.iter().find(|(id, _)| *id == resolution.tool_call_id)
            {
                resolution.outcome = outcome.clone();
            }
        }
    }

    tracing::info!(
        session_id = %session_id,
        resolution_kind = kind,
        requests = resolved.len(),
        outcomes = ?resolved.iter().map(|r| r.outcome.as_str()).collect::<Vec<_>>(),
        "tool approval requests resolved"
    );
    Ok(resolved)
}

/// Controls from the most recent user message in this session, if any.
async fn latest_user_controls(
    db: &StorageBackend,
    session_id: SessionId,
) -> Option<everruns_core::message::Controls> {
    let events = db
        .list_events(
            session_id,
            None,
            None,
            &["input.message".to_string()],
            &[],
            None,
            Some(APPROVAL_LOOKBACK_EVENTS),
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

/// Check a submission against what is pending and turn it into outcomes.
pub(crate) fn validate_decisions(
    pending: &[PendingApproval],
    decisions: &[ToolApprovalAnswer],
    now: DateTime<Utc>,
) -> Result<HashMap<String, ApprovalOutcome>, ApprovalResolveError> {
    if decisions.is_empty() {
        return Err(ApprovalResolveError::Invalid(
            "decisions must not be empty".to_string(),
        ));
    }
    if decisions.len() > MAX_DECISIONS {
        return Err(ApprovalResolveError::Invalid(format!(
            "at most {MAX_DECISIONS} decisions per submission"
        )));
    }
    let mut seen = HashSet::new();
    let mut outcomes = HashMap::new();
    for answer in decisions {
        if !seen.insert(answer.tool_call_id.as_str()) {
            return Err(ApprovalResolveError::Invalid(format!(
                "tool call {:?} is answered more than once",
                answer.tool_call_id
            )));
        }
        let Some(approval) = pending
            .iter()
            .find(|approval| approval.request_call_id == answer.tool_call_id)
        else {
            return Err(ApprovalResolveError::NotFound(format!(
                "No pending tool approval request {:?}",
                answer.tool_call_id
            )));
        };
        // An answer after the deadline would approve a call the model has
        // already been told to treat as rejected. The sweep resolves it.
        if approval.expires_at.is_some_and(|expires| now >= expires) {
            return Err(ApprovalResolveError::Expired);
        }
        outcomes.insert(
            answer.tool_call_id.clone(),
            ApprovalOutcome::Decided(answer.decision),
        );
    }
    Ok(outcomes)
}

/// POST /v1/sessions/{session_id}/tool-approvals
///
/// Records a person's decisions about tool calls an agent's `tool_approval`
/// gate held back, and resumes the paused turn.
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/tool-approvals",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    request_body = SubmitToolApprovalsRequest,
    responses(
        (status = 200, description = "Decisions recorded and workflow resumed", body = SubmitToolApprovalsResponse),
        (status = 400, description = "Invalid session ID or decisions"),
        (status = 403, description = "Caller may not manage this session"),
        (status = 404, description = "Session or pending approval request not found"),
        (status = 409, description = "Session is not waiting, the request expired, or it was already answered"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn submit_tool_approvals(
    org: crate::auth::ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<SubmitToolApprovalsRequest>,
) -> ApiResult<SubmitToolApprovalsResponse> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid session ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    // Approving lets a held-back action run, so it takes the same authority as
    // any other session write; attribution comes from the principal, never the
    // payload.
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
        // THREAT[TM-AGENT-017]: approvals resume Platform Chat like any other
        // answer and must bind to its persisted owner too.
        return Err(ErrorResponse::not_found("Session"));
    }

    let pending = pending_tool_approvals(&state.db, session_id)
        .await
        .log_internal_error_json("read session events")?;
    let to_response = |error: ApprovalResolveError| match error {
        ApprovalResolveError::NotWaiting(detail) => ErrorResponse::new(format!(
            "Session is not waiting for tool results (current status: {detail})"
        ))
        .into_response(StatusCode::CONFLICT),
        ApprovalResolveError::NotFound(detail) => {
            ErrorResponse::new(detail).into_response(StatusCode::NOT_FOUND)
        }
        ApprovalResolveError::AlreadyResolved => {
            ErrorResponse::new("These approval requests were already answered".to_string())
                .into_response(StatusCode::CONFLICT)
        }
        ApprovalResolveError::Expired => {
            ErrorResponse::new("The approval request expired and counts as rejected".to_string())
                .into_response(StatusCode::CONFLICT)
        }
        ApprovalResolveError::Invalid(detail) => {
            ErrorResponse::new(detail).into_response(StatusCode::BAD_REQUEST)
        }
        ApprovalResolveError::Internal(detail) => {
            tracing::error!(error = %detail, "Failed to resolve tool approvals");
            ErrorResponse::new("Internal server error".to_string())
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        }
    };
    let outcomes = validate_decisions(&pending, &req.decisions, Utc::now()).map_err(to_response)?;
    let resolved = resolve_tool_approvals(
        &ApprovalServices {
            db: &state.db,
            event_service: &state.event_service,
            runner: &state.runner,
        },
        org.org_id,
        session_id,
        &pending,
        &outcomes,
        ApprovalOutcome::NotApproved,
        "tool_approvals",
    )
    .await
    .map_err(to_response)?;

    Ok(Json(SubmitToolApprovalsResponse {
        resolved,
        status: "active".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pending(id: &str, expires_at: &str) -> PendingApproval {
        let request: ToolApprovalRequired = serde_json::from_value(json!({
            "code": "tool_approval_required", "error": "Waiting", "tool_call_id": id,
            "tool": "send_email", "arguments": {"to": "a@example.com"},
            "fingerprint": "sha256:ab", "risk": "open_world", "mode": "normal",
            "asked_at": "2026-10-01T00:00:00Z", "expires_at": expires_at,
        }))
        .unwrap();
        PendingApproval {
            request_call_id: format!("tool_approval_{id}"),
            expires_at: DateTime::parse_from_rfc3339(expires_at)
                .ok()
                .map(|v| v.with_timezone(&Utc)),
            request,
        }
    }

    fn answer(id: &str, decision: ToolApprovalDecision) -> ToolApprovalAnswer {
        ToolApprovalAnswer {
            tool_call_id: id.to_string(),
            decision,
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-01T00:05:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn decisions_must_name_pending_requests_once() {
        let pending = vec![pending("call_1", "2026-10-01T00:15:00Z")];
        assert!(matches!(
            validate_decisions(&pending, &[], now()),
            Err(ApprovalResolveError::Invalid(_))
        ));
        assert!(matches!(
            validate_decisions(
                &pending,
                &[answer("tool_approval_other", ToolApprovalDecision::Allow)],
                now()
            ),
            Err(ApprovalResolveError::NotFound(_))
        ));
        assert!(matches!(
            validate_decisions(
                &pending,
                &[
                    answer("tool_approval_call_1", ToolApprovalDecision::Allow),
                    answer("tool_approval_call_1", ToolApprovalDecision::Reject),
                ],
                now()
            ),
            Err(ApprovalResolveError::Invalid(_))
        ));
        let outcomes = validate_decisions(
            &pending,
            &[answer("tool_approval_call_1", ToolApprovalDecision::Allow)],
            now(),
        )
        .unwrap();
        assert_eq!(
            outcomes["tool_approval_call_1"],
            ApprovalOutcome::Decided(ToolApprovalDecision::Allow)
        );
    }

    #[test]
    fn an_expired_request_cannot_be_approved() {
        let pending = vec![pending("call_1", "2026-10-01T00:01:00Z")];
        assert_eq!(
            validate_decisions(
                &pending,
                &[answer("tool_approval_call_1", ToolApprovalDecision::Allow)],
                now()
            ),
            Err(ApprovalResolveError::Expired)
        );
    }

    #[test]
    fn only_decisions_are_recorded_and_silence_never_approves() {
        let request = pending("call_1", "2026-10-01T00:15:00Z").request;
        let (key, record) = ApprovalOutcome::Decided(ToolApprovalDecision::Allow)
            .stored_record(&request, now())
            .unwrap();
        assert_eq!(key, one_off_decision_storage_key("sha256:ab"));
        assert!(record.allow);
        assert_eq!(record.fingerprint.as_deref(), Some("sha256:ab"));

        let (key, record) = ApprovalOutcome::Decided(ToolApprovalDecision::RejectAlways)
            .stored_record(&request, now())
            .unwrap();
        assert_eq!(key, always_decision_storage_key("send_email"));
        assert!(!record.allow);
        assert!(record.fingerprint.is_none());

        assert!(
            ApprovalOutcome::NotApproved
                .stored_record(&request, now())
                .is_none()
        );
        assert!(
            ApprovalOutcome::Expired
                .stored_record(&request, now())
                .is_none()
        );
        assert!(!ApprovalOutcome::NotApproved.allows());
        assert!(!ApprovalOutcome::Expired.allows());
    }

    #[test]
    fn only_engine_authored_requests_are_pending() {
        let request = pending("call_1", "2026-10-01T00:15:00Z").request;
        let engine = request.request_call();
        let row = |calls: serde_json::Value| {
            let now = Utc::now();
            EventRow {
                id: everruns_contracts::typed_id::EventId::new(),
                session_id: SessionId::new(),
                sequence: 1,
                event_type: "tool.call_requested".to_string(),
                ts: now,
                context: json!({}),
                data: json!({ "tool_calls": calls }),
                metadata: None,
                tags: None,
                created_at: now,
            }
        };
        let found = pending_approvals_from_events(&[row(json!([engine]))]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].request.tool, "send_email");

        // Same name, model-authored id: not an approval request.
        let forged = json!([{ "id": "call_9", "name": "approve_tool_call",
            "arguments": engine.arguments }]);
        assert!(pending_approvals_from_events(&[row(forged)]).is_empty());
    }
}
