// Answering an `ask_user` question set (EVE-1054).
//
// `ask_user` will be answered from four places — a browser card, `/mcp`, an A2A
// caller, and an in-process host. Each one growing its own implementation is how
// they drift on validation, attribution and idempotency, so they all call
// `resolve_question_answers` and this file is the only place those three live.
//
// The sibling of `tool_results` (raw results, no typed validation) and
// `mcp_url_consent` (a pause Everruns itself raised). One difference from that
// last one is load-bearing: `ask_user` is called by the *model*, so the
// transcript claims the call and the tool result alone carries the answer back.
// URL elicitation needs an extra synthetic user message precisely because its
// call is engine-authored; adding one here would put words in the user's mouth
// that they never said.

use crate::services::waiting_turn_resolution::execute_waiting_turn_resolution;
use crate::storage::models::{ClaimWaitingTurnResult, WaitingTurnResolutionPlan};
use everruns_builtins::ask_user::{
    ASK_USER_TOOL_NAME, AskUserAnswer, AskUserAnsweredBy, AskUserQuestion, AskUserQuestionKind,
    AskUserResult, AskUserStatus, session_secret_ref,
};

/// How far back to look for the question set being answered. The card is emitted
/// by the act that just paused, so it is within the last handful of events.
/// Matches `mcp_url_consent`'s window for the same reason.
pub(crate) const QUESTION_LOOKBACK_EVENTS: i32 = 200;

/// What was actually asked, recovered from the event that asked it.
///
/// THREAT[TM-AGENT-015]: the caller submitting an answer does not get to say what
/// it was asked. Every label an answer selects is checked against the options in
/// the emitted `tool.call_requested`, never against anything in the request body
/// — the same principle `mcp_url_consent` applies to the consented domain.
#[derive(Debug, Clone)]
pub(crate) struct PendingQuestions {
    pub(crate) tool_call_id: String,
    pub(crate) questions: Vec<AskUserQuestion>,
    /// The deadline normalization stamped on this call (EVE-1056).
    ///
    /// `None` only for a call recorded before the server emitted deadlines;
    /// the sweep falls back to the generic timeout for those rather than
    /// resolving them on a deadline nobody wrote down.
    pub(crate) expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Check a submitted answer set against what was asked.
///
/// Returns the answers in the order the questions were asked, so the model reads
/// them in the order it wrote them rather than the order a client happened to
/// serialize them.
pub(crate) fn validate_answers(
    questions: &[AskUserQuestion],
    submitted: &[AskUserAnswer],
) -> Result<Vec<AskUserAnswer>, String> {
    for answer in submitted {
        if !questions
            .iter()
            .any(|question| question.id.as_deref() == Some(answer.id.as_str()))
        {
            return Err(format!("no question with id {:?} was asked", answer.id));
        }
    }

    let mut validated = Vec::with_capacity(questions.len());
    for question in questions {
        let id = question.id.as_deref().unwrap_or_default();
        let matches: Vec<&AskUserAnswer> =
            submitted.iter().filter(|answer| answer.id == id).collect();
        if matches.len() > 1 {
            return Err(format!("question {id:?} was answered more than once"));
        }
        // Every asked question needs an answer. A partial set would reach the
        // model as a question it asked and nobody addressed, which it cannot
        // tell apart from one the person deliberately skipped.
        let answer = matches
            .first()
            .ok_or_else(|| format!("question {id:?} was not answered"))?;

        if question.kind == AskUserQuestionKind::Secret {
            // THREAT[TM-AGENT-016]: the only thing a secret answer may carry is
            // a reference to a secret that was already stored encrypted. Every
            // value-bearing field is refused here rather than ignored, so a
            // client that put the credential in one gets an error instead of a
            // result that quietly persisted it to the event log.
            if !answer.selected.is_empty() || answer.other_text.is_some() {
                return Err(format!(
                    "question {id:?} is a secret; its answer carries only secret_ref"
                ));
            }
            let expected = session_secret_ref(question.secret_name.as_deref().unwrap_or_default());
            match answer.secret_ref.as_deref() {
                Some(reference) if reference == expected => {}
                Some(_) => {
                    return Err(format!("question {id:?} was not asked for that secret"));
                }
                None => return Err(format!("question {id:?} has no secret_ref")),
            }
            validated.push(AskUserAnswer {
                id: id.to_string(),
                selected: Vec::new(),
                other_text: None,
                secret_ref: Some(expected),
            });
            continue;
        }
        // A choice answer cannot mint a handle to a secret nobody asked for.
        if answer.secret_ref.is_some() {
            return Err(format!("question {id:?} is not a secret question"));
        }

        for label in &answer.selected {
            if !question.options.iter().any(|option| &option.label == label) {
                return Err(format!(
                    "question {id:?} was not asked with option {label:?}"
                ));
            }
        }
        if answer.selected.len()
            != answer
                .selected
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
        {
            return Err(format!("question {id:?} selects the same option twice"));
        }

        let other = answer
            .other_text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty());
        if other.is_some() && !question.allow_other {
            return Err(format!("question {id:?} does not allow free text"));
        }

        // An answer has to say something. Free text stands in for a selection,
        // which is the whole point of `allow_other`.
        if answer.selected.is_empty() && other.is_none() {
            return Err(format!("question {id:?} has no selection and no free text"));
        }
        if !question.multi_select && answer.selected.len() > 1 {
            return Err(format!(
                "question {id:?} is single-select and carries {} selections",
                answer.selected.len()
            ));
        }

        validated.push(AskUserAnswer {
            id: id.to_string(),
            selected: answer.selected.clone(),
            other_text: other.map(str::to_string),
            secret_ref: None,
        });
    }
    Ok(validated)
}

/// Build the result the model reads.
///
/// `answered_by` is decided here, never taken from the payload: a model-asserted
/// "a human answered this" would answer nothing. Only an outcome a person
/// actually produced is attributed to `User`.
pub(crate) fn build_result(status: AskUserStatus, answers: Vec<AskUserAnswer>) -> AskUserResult {
    let answered_by = match status {
        AskUserStatus::Answered | AskUserStatus::Declined => AskUserAnsweredBy::User,
        AskUserStatus::TimedOut => AskUserAnsweredBy::Timeout,
        AskUserStatus::Cancelled => AskUserAnsweredBy::Unattended,
    };
    AskUserResult {
        status,
        answered_by,
        // `answered` and `timed_out` carry answers; `declined` and `cancelled`
        // do not. A decline that shipped the options the person refused to
        // choose between would read to the model as a choice, but a timeout
        // *is* the declared defaults being applied (EVE-1056) — dropping them
        // would leave the model with no value at all. `answered_by: timeout`
        // is what tells it no human spoke, so the value is not consent.
        answers: if matches!(status, AskUserStatus::Answered | AskUserStatus::TimedOut) {
            answers
        } else {
            Vec::new()
        },
    }
}

/// Pull the `ask_user` call out of the current `tool.call_requested` batch.
///
/// `tool_call_id` is optional because some answer surfaces rely on the current
/// pending question set instead of carrying a rendered card's call id.
pub(crate) fn pending_from_events(
    events: &[crate::storage::models::EventRow],
    tool_call_id: Option<&str>,
) -> Option<PendingQuestions> {
    let event = events.last()?;
    let tool_calls = event.data.get("tool_calls")?.as_array()?;
    for call in tool_calls {
        let id = call.get("id").and_then(|v| v.as_str())?;
        if let Some(wanted) = tool_call_id
            && id != wanted
        {
            continue;
        }
        if call.get("name").and_then(|v| v.as_str()) != Some(ASK_USER_TOOL_NAME) {
            // A specific id that names some other tool is a client error,
            // not "keep looking": answering it as a question set would
            // complete a call that asked something else entirely.
            if tool_call_id.is_some() {
                return None;
            }
            continue;
        }
        let arguments = call.get("arguments")?;
        let questions: Vec<AskUserQuestion> =
            serde_json::from_value(arguments.get("questions")?.clone()).ok()?;
        let expires_at = arguments
            .get("expires_at")
            .and_then(|value| value.as_str())
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&chrono::Utc));
        return Some(PendingQuestions {
            tool_call_id: id.to_string(),
            questions,
            expires_at,
        });
    }
    None
}

/// Why a resolve did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// The session is not parked on a tool call at all.
    NotWaiting(String),
    /// No pending `ask_user` call matches.
    NoPendingQuestions,
    /// The current parked call is not the submitted `ask_user` question set.
    WrongPendingCall,
    /// Something already answered this call. First writer wins.
    AlreadyResolved,
    /// The submitted answers do not match what was asked.
    Invalid(String),
    /// Persistence or durable resume failed.
    Internal(String),
}

/// Resolve the pending question set on a session with this outcome.
///
/// The one operation every surface calls. It owns validation against what was
/// asked, attribution, idempotency, and resuming the durable turn — so a second
/// surface cannot implement any of them differently.
///
/// `submitted` is ignored for every status but `Answered`, because those
/// outcomes carry no answers by construction (see `build_result`).
pub struct QuestionResolver<'a> {
    pub(crate) db: &'a std::sync::Arc<crate::storage::StorageBackend>,
    pub(crate) session_service: &'a crate::domains::sessions::SessionService,
    pub(crate) event_service: &'a crate::services::EventService,
    pub(crate) runner: std::sync::Arc<dyn everruns_worker::AgentRunner>,
}

pub async fn resolve_question_answers(
    state: &QuestionResolver<'_>,
    caller: &everruns_core::Caller,
    session_id: everruns_provider::typed_id::SessionId,
    tool_call_id: Option<&str>,
    status: AskUserStatus,
    submitted: &[AskUserAnswer],
) -> Result<AskUserResult, ResolveError> {
    state
        .session_service
        .get(caller, session_id.uuid(), None)
        .await
        .map_err(|error| ResolveError::Internal(error.to_string()))?
        .ok_or(ResolveError::NoPendingQuestions)?;

    let requested = state
        .db
        .list_events(
            session_id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(QUESTION_LOOKBACK_EVENTS),
        )
        .await
        .map_err(|error| ResolveError::Internal(error.to_string()))?;
    if requested.is_empty() {
        return Err(ResolveError::NoPendingQuestions);
    }
    let pending =
        pending_from_events(&requested, tool_call_id).ok_or(ResolveError::WrongPendingCall)?;

    // A timeout's defaults are validated on the same path as a human answer.
    // They are generated from the question's own options, so they must pass —
    // and if they ever do not, the sweep has produced something the model
    // would have read as a legal choice.
    let answers = if matches!(status, AskUserStatus::Answered | AskUserStatus::TimedOut) {
        validate_answers(&pending.questions, submitted).map_err(ResolveError::Invalid)?
    } else {
        Vec::new()
    };

    // A `secret_ref` naming nothing is worse than a decline: the model would
    // read the question as answered and hand tools a handle that resolves to
    // empty. The client stores the value first, through the session-secret
    // endpoint that has always encrypted it, and only then answers the card.
    if answers.iter().any(|answer| answer.secret_ref.is_some()) {
        let stored = state
            .db
            .list_session_secrets(session_id.uuid())
            .await
            .map_err(|error| ResolveError::Internal(error.to_string()))?;
        for question in &pending.questions {
            if question.kind != AskUserQuestionKind::Secret {
                continue;
            }
            let name = question.secret_name.as_deref().unwrap_or_default();
            if !stored.iter().any(|row| row.name == name) {
                return Err(ResolveError::Invalid(format!(
                    "secret {name:?} has not been stored on this session"
                )));
            }
        }
    }

    let result = build_result(status, answers);

    let turn_id = everruns_provider::typed_id::TurnId::from_uuid(session_id.uuid());
    let event_message_id = everruns_provider::typed_id::MessageId::from_uuid(session_id.uuid());
    let payload = serde_json::to_value(&result).unwrap_or_default();

    // The tool result alone. `ask_user` is called by the model, so the
    // transcript claims the call and this reaches it; a synthetic user message
    // would put words in the person's mouth they never said.
    let completed_event = everruns_core::events::ToolCompletedData::success(
        pending.tool_call_id.clone(),
        ASK_USER_TOOL_NAME.to_string(),
        vec![everruns_core::message::ContentPart::tool_result_text(
            &payload,
        )],
        None,
    );
    let plan = WaitingTurnResolutionPlan {
        kind: "question_answers".to_string(),
        events: vec![everruns_core::events::EventRequest::new(
            session_id,
            everruns_core::events::EventContext::turn(turn_id, event_message_id),
            completed_event,
        )],
        session_values: Vec::new(),
        response: serde_json::to_value(&result)
            .map_err(|error| ResolveError::Internal(error.to_string()))?,
    };
    let claim = match state
        .db
        .claim_waiting_turn(caller.org_id, session_id, plan)
        .await
        .map_err(|error| ResolveError::Internal(error.to_string()))?
    {
        ClaimWaitingTurnResult::Claimed(claim) => claim,
        ClaimWaitingTurnResult::Conflict { current_status } => {
            let completed = state
                .db
                .list_events(
                    session_id,
                    None,
                    None,
                    &["tool.completed".to_string()],
                    &[],
                    None,
                    Some(QUESTION_LOOKBACK_EVENTS),
                )
                .await
                .map_err(|error| ResolveError::Internal(error.to_string()))?;
            if completed.iter().any(|event| {
                event.data.get("tool_call_id").and_then(|v| v.as_str())
                    == Some(&pending.tool_call_id)
            }) {
                return Err(ResolveError::AlreadyResolved);
            }
            return Err(ResolveError::NotWaiting(current_status));
        }
        ClaimWaitingTurnResult::SessionNotFound => {
            return Err(ResolveError::NoPendingQuestions);
        }
    };
    execute_waiting_turn_resolution(
        state.db,
        state.event_service,
        &state.runner,
        caller.org_id,
        session_id,
        &claim,
    )
    .await
    .map_err(|error| ResolveError::Internal(error.to_string()))?;
    let result = serde_json::from_value(claim.plan.response)
        .map_err(|error| ResolveError::Internal(error.to_string()))?;

    tracing::info!(
        session_id = %session_id,
        tool_call_id = %pending.tool_call_id,
        status = ?status,
        questions = pending.questions.len(),
        recovered = claim.recovered,
        "ask_user question set resolved"
    );

    Ok(result)
}

/// Request to answer a pending `ask_user` question set.
#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct QuestionAnswersRequest {
    /// The `ask_user` tool call being answered. Optional: when omitted the one
    /// pending question set on the session is used.
    #[serde(default)]
    #[schema(example = "toolu_01933b5a00007000800000000000001")]
    pub tool_call_id: Option<String>,
    /// What the person did. `answered` carries answers; `declined` is a finished
    /// decision the model must not re-ask.
    #[serde(default = "default_answer_status")]
    pub status: SubmittedStatus,
    /// One answer per asked question, keyed by question id.
    #[serde(default)]
    #[schema(example = json!([{"id": "target", "selected": ["Staging"], "other_text": null}]))]
    pub answers: Vec<SubmittedAnswer>,
}

fn default_answer_status() -> SubmittedStatus {
    SubmittedStatus::Answered
}

/// The outcomes a caller may submit.
///
/// `cancelled` and `timed_out` are deliberately absent: they are the server's to
/// decide, from a superseding message and from the deadline sweep. A caller that
/// could assert them could claim a person's silence as their answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubmittedStatus {
    Answered,
    Declined,
}

/// One submitted answer.
#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct SubmittedAnswer {
    /// Question id, as carried in the `tool.call_requested` payload.
    pub id: String,
    /// Chosen option labels. Must be labels that were actually offered.
    #[serde(default)]
    pub selected: Vec<String>,
    /// Free text, accepted only when the question allows it.
    #[serde(default)]
    pub other_text: Option<String>,
    /// Handle to the stored credential, on a `secret` question only — the value
    /// itself is never submitted here. Store it with
    /// `PUT /v1/sessions/{session_id}/storage/secrets` first, then answer with
    /// `session:{secret_name}`.
    #[serde(default)]
    #[schema(example = "session:STRIPE_API_KEY")]
    pub secret_ref: Option<String>,
}

/// Result of answering a pending question set.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct QuestionAnswersResponse {
    /// Outcome recorded against the call.
    #[schema(example = "answered")]
    pub status: String,
    /// Who the server attributed the outcome to.
    #[schema(example = "user")]
    pub answered_by: String,
    /// Session status after the answer.
    #[schema(example = "active")]
    pub session_status: String,
}

/// Routes for answering `ask_user` question sets. Merged into the tool-results
/// router because it is the same pause-and-resume surface.
pub fn routes() -> axum::Router<super::tool_results::AppState> {
    axum::Router::new().route(
        "/v1/sessions/{session_id}/question-answers",
        axum::routing::post(submit_question_answers),
    )
}

/// POST /v1/sessions/{session_id}/question-answers
///
/// Answers the question set an agent raised with `ask_user`, and resumes the
/// paused turn.
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/question-answers",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    request_body = QuestionAnswersRequest,
    responses(
        (status = 200, description = "Answer recorded and workflow resumed", body = QuestionAnswersResponse),
        (status = 400, description = "Invalid session ID, or answers that do not match what was asked"),
        (status = 404, description = "Session or pending question set not found"),
        (status = 409, description = "Session is not waiting for tool results, or the question set was already answered"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn submit_question_answers(
    org: crate::auth::ResolvedOrg,
    axum::extract::State(state): axum::extract::State<super::tool_results::AppState>,
    axum::extract::Path(session_id): axum::extract::Path<String>,
    axum::Json(req): axum::Json<QuestionAnswersRequest>,
) -> super::common::ApiResult<QuestionAnswersResponse> {
    let session_id: everruns_provider::typed_id::SessionId =
        session_id.parse().map_err(|error| {
            super::common::ErrorResponse::new(format!("Invalid session ID: {error}"))
                .into_response(axum::http::StatusCode::BAD_REQUEST)
        })?;

    // Attribution comes from the authenticated principal, never the payload.
    let caller = everruns_core::Caller::from(&org);
    let status = match req.status {
        SubmittedStatus::Answered => AskUserStatus::Answered,
        SubmittedStatus::Declined => AskUserStatus::Declined,
    };
    let submitted: Vec<AskUserAnswer> = req
        .answers
        .into_iter()
        .map(|answer| AskUserAnswer {
            id: answer.id,
            selected: answer.selected,
            other_text: answer.other_text,
            secret_ref: answer.secret_ref,
        })
        .collect();

    let resolver = QuestionResolver {
        db: &state.db,
        session_service: &state.session_service,
        event_service: &state.event_service,
        runner: state.runner.clone(),
    };
    let result = resolve_question_answers(
        &resolver,
        &caller,
        session_id,
        req.tool_call_id.as_deref(),
        status,
        &submitted,
    )
    .await
    .map_err(|error| match error {
        ResolveError::NotWaiting(detail) => super::common::ErrorResponse::new(format!(
            "Session is not waiting for tool results (current status: {detail})"
        ))
        .into_response(axum::http::StatusCode::CONFLICT),
        ResolveError::AlreadyResolved => super::common::ErrorResponse::new(
            "This question set has already been answered".to_string(),
        )
        .into_response(axum::http::StatusCode::CONFLICT),
        ResolveError::NoPendingQuestions => {
            super::common::ErrorResponse::new("Pending question set not found".to_string())
                .into_response(axum::http::StatusCode::NOT_FOUND)
        }
        ResolveError::WrongPendingCall => super::common::ErrorResponse::new(
            "Session is waiting on a different tool call".to_string(),
        )
        .into_response(axum::http::StatusCode::CONFLICT),
        ResolveError::Invalid(detail) => super::common::ErrorResponse::new(detail)
            .into_response(axum::http::StatusCode::BAD_REQUEST),
        ResolveError::Internal(detail) => {
            tracing::error!(error = %detail, "Failed to resolve question answers");
            super::common::ErrorResponse::new("Internal server error".to_string())
                .into_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    })?;

    Ok(axum::Json(QuestionAnswersResponse {
        status: serde_json::to_value(result.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        answered_by: serde_json::to_value(result.answered_by)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        session_status: "active".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_builtins::ask_user::{AskUserOption, AskUserQuestionKind};

    /// A `tool.call_requested` row carrying one `ask_user` call, shaped the way
    /// the event stream stores it.
    fn event_with_ask_user_arguments(
        arguments: serde_json::Value,
    ) -> crate::storage::models::EventRow {
        let now = chrono::Utc::now();
        crate::storage::models::EventRow {
            id: everruns_provider::typed_id::EventId::new(),
            session_id: everruns_provider::typed_id::SessionId::new(),
            sequence: 1,
            event_type: "tool.call_requested".to_string(),
            ts: now,
            context: serde_json::json!({}),
            data: serde_json::json!({
                "tool_calls": [{
                    "id": "call_1",
                    "name": ASK_USER_TOOL_NAME,
                    "arguments": arguments,
                }]
            }),
            metadata: None,
            tags: None,
            created_at: now,
        }
    }

    fn option(label: &str) -> AskUserOption {
        AskUserOption {
            label: label.to_string(),
            description: format!("{label} description"),
            is_default: false,
        }
    }

    fn question(id: &str, multi_select: bool, allow_other: bool) -> AskUserQuestion {
        AskUserQuestion {
            kind: AskUserQuestionKind::Choice,
            id: Some(id.to_string()),
            header: "Target".to_string(),
            question: "Which environment?".to_string(),
            multi_select,
            allow_other,
            options: vec![option("Staging"), option("Production")],
            secret_name: None,
            purpose: None,
        }
    }

    fn secret_question(id: &str, secret_name: &str) -> AskUserQuestion {
        AskUserQuestion {
            kind: AskUserQuestionKind::Secret,
            id: Some(id.to_string()),
            header: "Stripe key".to_string(),
            question: "Which Stripe restricted key should I use?".to_string(),
            multi_select: false,
            allow_other: false,
            options: Vec::new(),
            secret_name: Some(secret_name.to_string()),
            purpose: Some("Read-only charge lookups.".to_string()),
        }
    }

    fn answer(id: &str, selected: &[&str], other: Option<&str>) -> AskUserAnswer {
        AskUserAnswer {
            id: id.to_string(),
            selected: selected.iter().map(|s| s.to_string()).collect(),
            other_text: other.map(str::to_string),
            secret_ref: None,
        }
    }

    fn secret_answer(id: &str, secret_ref: Option<&str>) -> AskUserAnswer {
        AskUserAnswer {
            id: id.to_string(),
            selected: Vec::new(),
            other_text: None,
            secret_ref: secret_ref.map(str::to_string),
        }
    }

    #[test]
    fn a_selection_that_was_offered_is_accepted() {
        let questions = vec![question("target", false, true)];
        let validated =
            validate_answers(&questions, &[answer("target", &["Staging"], None)]).expect("valid");
        assert_eq!(validated.len(), 1);
        assert_eq!(validated[0].selected, vec!["Staging".to_string()]);
    }

    /// THREAT[TM-AGENT-015]: the caller does not get to say what it was asked.
    #[test]
    fn a_label_that_was_never_offered_is_refused() {
        let questions = vec![question("target", false, true)];
        let error = validate_answers(&questions, &[answer("target", &["Production "], None)])
            .expect_err("a near-miss label is still not an option");
        assert!(error.contains("not asked with option"), "{error}");

        let error = validate_answers(&questions, &[answer("target", &["rm -rf /"], None)])
            .expect_err("an invented label");
        assert!(error.contains("not asked with option"), "{error}");
    }

    #[test]
    fn a_question_that_was_never_asked_is_refused() {
        let questions = vec![question("target", false, true)];
        let error = validate_answers(&questions, &[answer("other_question", &["Staging"], None)])
            .expect_err("unknown question id");
        assert!(error.contains("no question with id"), "{error}");
    }

    #[test]
    fn a_single_select_carries_exactly_one() {
        let questions = vec![question("target", false, true)];
        let error = validate_answers(
            &questions,
            &[answer("target", &["Staging", "Production"], None)],
        )
        .expect_err("two selections on a single-select");
        assert!(error.contains("single-select"), "{error}");
    }

    #[test]
    fn a_multi_select_carries_several() {
        let questions = vec![question("target", true, true)];
        let validated = validate_answers(
            &questions,
            &[answer("target", &["Staging", "Production"], None)],
        )
        .expect("multi-select accepts both");
        assert_eq!(validated[0].selected.len(), 2);
    }

    #[test]
    fn the_same_option_twice_is_refused() {
        let questions = vec![question("target", true, true)];
        let error = validate_answers(
            &questions,
            &[answer("target", &["Staging", "Staging"], None)],
        )
        .expect_err("duplicate selection");
        assert!(error.contains("same option twice"), "{error}");
    }

    #[test]
    fn free_text_needs_allow_other() {
        let allowed = vec![question("target", false, true)];
        validate_answers(&allowed, &[answer("target", &[], Some("Somewhere else"))])
            .expect("free text stands in for a selection when allowed");

        let refused = vec![question("target", false, false)];
        let error = validate_answers(&refused, &[answer("target", &[], Some("Somewhere else"))])
            .expect_err("free text on a closed question");
        assert!(error.contains("does not allow free text"), "{error}");
    }

    #[test]
    fn blank_free_text_does_not_count_as_an_answer() {
        let questions = vec![question("target", false, true)];
        let error = validate_answers(&questions, &[answer("target", &[], Some("   "))])
            .expect_err("whitespace is not an answer");
        assert!(error.contains("no selection and no free text"), "{error}");
    }

    #[test]
    fn every_asked_question_must_be_answered() {
        let questions = vec![
            question("target", false, true),
            question("scope", false, true),
        ];
        let error = validate_answers(&questions, &[answer("target", &["Staging"], None)])
            .expect_err("a partial answer set");
        assert!(error.contains("was not answered"), "{error}");
    }

    #[test]
    fn answering_one_question_twice_is_refused() {
        let questions = vec![question("target", false, true)];
        let error = validate_answers(
            &questions,
            &[
                answer("target", &["Staging"], None),
                answer("target", &["Production"], None),
            ],
        )
        .expect_err("two answers for one question");
        assert!(error.contains("more than once"), "{error}");
    }

    #[test]
    fn answers_come_back_in_the_order_they_were_asked() {
        let questions = vec![
            question("target", false, true),
            question("scope", false, true),
        ];
        let validated = validate_answers(
            &questions,
            &[
                answer("scope", &["Production"], None),
                answer("target", &["Staging"], None),
            ],
        )
        .expect("valid");
        assert_eq!(validated[0].id, "target");
        assert_eq!(validated[1].id, "scope");
    }

    #[test]
    fn a_secret_answer_carries_only_the_reference_it_was_asked_for() {
        let questions = vec![secret_question("stripe_key", "STRIPE_API_KEY")];
        let validated = validate_answers(
            &questions,
            &[secret_answer("stripe_key", Some("session:STRIPE_API_KEY"))],
        )
        .expect("the ref for the secret that was asked for");
        assert_eq!(
            validated[0].secret_ref.as_deref(),
            Some("session:STRIPE_API_KEY")
        );
        assert!(validated[0].selected.is_empty());
        assert!(validated[0].other_text.is_none());
    }

    /// THREAT[TM-AGENT-016]: there must be no path from a typed credential to a
    /// tool result. A client that put the value in a value-bearing field gets an
    /// error, not a result that silently persisted it.
    #[test]
    fn a_secret_answer_that_carries_a_value_is_refused() {
        let questions = vec![secret_question("stripe_key", "STRIPE_API_KEY")];

        let mut with_free_text = secret_answer("stripe_key", Some("session:STRIPE_API_KEY"));
        with_free_text.other_text = Some("rk_live_verysecret".to_string());
        let error = validate_answers(&questions, &[with_free_text])
            .expect_err("free text on a secret question");
        assert!(error.contains("only secret_ref"), "{error}");

        let mut with_selection = secret_answer("stripe_key", Some("session:STRIPE_API_KEY"));
        with_selection.selected = vec!["rk_live_verysecret".to_string()];
        let error = validate_answers(&questions, &[with_selection])
            .expect_err("a selection on a secret question");
        assert!(error.contains("only secret_ref"), "{error}");
    }

    /// THREAT[TM-TOOL-036]: the caller does not get to say which secret it was
    /// asked for, any more than it gets to say which options were offered.
    #[test]
    fn a_reference_to_another_secret_is_refused() {
        let questions = vec![secret_question("stripe_key", "STRIPE_API_KEY")];
        let error = validate_answers(
            &questions,
            &[secret_answer(
                "stripe_key",
                Some("session:AWS_SECRET_ACCESS_KEY"),
            )],
        )
        .expect_err("a ref to a secret that was never asked for");
        assert!(error.contains("not asked for that secret"), "{error}");

        let error = validate_answers(&questions, &[secret_answer("stripe_key", None)])
            .expect_err("no ref at all");
        assert!(error.contains("no secret_ref"), "{error}");
    }

    #[test]
    fn a_choice_answer_cannot_mint_a_secret_reference() {
        let questions = vec![question("target", false, true)];
        let mut smuggled = answer("target", &["Staging"], None);
        smuggled.secret_ref = Some("session:STRIPE_API_KEY".to_string());
        let error = validate_answers(&questions, &[smuggled])
            .expect_err("a secret ref on a choice question");
        assert!(error.contains("not a secret question"), "{error}");
    }

    /// The whole result is what reaches the event log and model context. Nothing
    /// in it may resemble a credential.
    #[test]
    fn the_serialized_secret_result_contains_no_value_field() {
        let questions = vec![secret_question("stripe_key", "STRIPE_API_KEY")];
        let validated = validate_answers(
            &questions,
            &[secret_answer("stripe_key", Some("session:STRIPE_API_KEY"))],
        )
        .expect("valid");
        let encoded =
            serde_json::to_string(&build_result(AskUserStatus::Answered, validated)).unwrap();
        assert!(encoded.contains("session:STRIPE_API_KEY"), "{encoded}");
        assert!(!encoded.contains("value"), "{encoded}");
    }

    /// Attribution is the server's to decide. A `cancelled` outcome is the
    /// question being superseded, which no person chose.
    #[test]
    fn attribution_follows_the_outcome_not_the_caller() {
        let answers = vec![answer("target", &["Staging"], None)];
        assert_eq!(
            build_result(AskUserStatus::Answered, answers.clone()).answered_by,
            AskUserAnsweredBy::User
        );
        assert_eq!(
            build_result(AskUserStatus::Declined, answers.clone()).answered_by,
            AskUserAnsweredBy::User
        );
        assert_eq!(
            build_result(AskUserStatus::Cancelled, answers.clone()).answered_by,
            AskUserAnsweredBy::Unattended
        );
        assert_eq!(
            build_result(AskUserStatus::TimedOut, answers).answered_by,
            AskUserAnsweredBy::Timeout
        );
    }

    #[test]
    fn only_a_chosen_or_defaulted_outcome_carries_answers() {
        let answers = vec![answer("target", &["Staging"], None)];
        assert_eq!(
            build_result(AskUserStatus::Answered, answers.clone())
                .answers
                .len(),
            1
        );
        // A timeout applies the declared defaults (EVE-1056), so it carries
        // them. `answered_by: timeout` is what tells the model no human spoke.
        let timed_out = build_result(AskUserStatus::TimedOut, answers.clone());
        assert_eq!(timed_out.answers.len(), 1);
        assert_eq!(timed_out.answered_by, AskUserAnsweredBy::Timeout);
        // A decline that shipped the options would read to the model as a choice.
        assert!(
            build_result(AskUserStatus::Declined, answers.clone())
                .answers
                .is_empty()
        );
        assert!(
            build_result(AskUserStatus::Cancelled, answers)
                .answers
                .is_empty()
        );
    }

    /// The sweep reads the deadline off the same parse every answer surface
    /// uses, so a call recorded before the server stamped deadlines has to come
    /// back as `None` rather than as some invented instant.
    #[test]
    fn the_pending_call_carries_its_stamped_deadline() {
        let stamped = pending_from_events(
            &[event_with_ask_user_arguments(serde_json::json!({
                "questions": [{
                    "id": "target",
                    "header": "Target",
                    "question": "Which environment?",
                    "options": [
                        {"label": "Staging", "description": "staging", "default": true},
                        {"label": "Prod", "description": "prod", "default": false}
                    ]
                }],
                "timeout_seconds": 300,
                "expires_at": "2099-01-01T00:00:00Z",
            }))],
            None,
        )
        .expect("the ask_user call is pending");
        assert_eq!(
            stamped.expires_at.map(|at| at.to_rfc3339()),
            Some("2099-01-01T00:00:00+00:00".to_string())
        );

        let unstamped = pending_from_events(
            &[event_with_ask_user_arguments(serde_json::json!({
                "questions": [{
                    "id": "target",
                    "header": "Target",
                    "question": "Which environment?",
                    "options": [
                        {"label": "Staging", "description": "staging", "default": true},
                        {"label": "Prod", "description": "prod", "default": false}
                    ]
                }],
                "timeout_seconds": 300,
            }))],
            None,
        )
        .expect("the ask_user call is pending");
        assert_eq!(unstamped.expires_at, None);
    }
}
