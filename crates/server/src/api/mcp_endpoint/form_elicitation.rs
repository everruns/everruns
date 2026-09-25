// Form mode elicitation for Everruns' own MCP server endpoint (EVE-1060).
//
// `ask_user` is the question Everruns has and MCP has the form for. The
// protocol's form mode carries a `requestedSchema` the client renders itself,
// so a question set reaches an MCP client as a form rather than as prose the
// model has to parse back.
//
// Three decisions shape the mapping:
//
// 1. **Flat, primitive, enum.** `requestedSchema` is a restricted profile: one
//    level of properties, each a primitive or an enum of them. Nesting and
//    arrays are outside it, so the whole question set flattens into one object
//    and every question becomes one property (or, below, several).
//
// 2. **Multi-select is one boolean per option.** There is no array in the
//    profile. A comma-joined string would have to be re-split against labels
//    that may contain commas, and refusing form mode for multi-select would
//    leave a client that *can* be asked staring at a parked turn. Booleans stay
//    inside the profile and round-trip losslessly, at the cost of a wide form.
//    The key is `{question_id}__{option_index}`, by index rather than by label
//    so the round trip does not depend on a model-authored label being a usable
//    property name.
//
// 3. **A secret question is never a form field.** TM-AGENT-016: an `ask_user`
//    answer is a tool result, so a credential typed into a form property would
//    be plaintext in the event log and in model context forever. The elicitation
//    path screens a secret question set out before it builds anything, and
//    `requested_schema` refuses one again, so the boundary holds even if a
//    future caller forgets the first check. The one thing this module must
//    never grow is a property that could carry a value.

use everruns_builtins::ask_user::{AskUserAnswer, AskUserQuestion, AskUserQuestionKind};
use serde_json::{Map, Value, json};

/// Key form-mode elicitations are delivered under in `inputRequests`. Stable,
/// for the same reason URL mode's keys are: a client retrying sees a coherent
/// key across rounds.
pub(super) const ASK_USER_REQUEST_KEY: &str = "ask_user";

/// Whether a session this call starts may park on an `ask_user` question set:
/// only a client that can be elicited, over a protocol that can carry the
/// elicitation, has anyone to answer it (EVE-1057, EVE-1060).
pub(super) fn client_can_answer_questions(params: &Value, protocol_version: &str) -> bool {
    super::tool_registry::supports_mrtr(protocol_version)
        && super::elicitation::client_supports_elicitation(params)
}

/// The session-level pause hint a capable client's session is created with.
///
/// A claim about *this* client: set only when the caller said it can be
/// elicited, so an unelicitable caller's turn resolves the question with the
/// model's declared defaults instead of parking forever (EVE-1057). The hint is
/// session-level, so it is decided when the session is created.
pub(super) fn ask_user_pause_hint() -> Value {
    json!({ "ask_user": true })
}

/// What a client did with the form it was handed.
///
/// MCP's accept/decline/cancel trichotomy maps onto the `ask_user` contract's
/// outcomes without translation: an accepted form is an answer, a declined one
/// is a finished decision the model must not re-ask, and a dismissed one is a
/// cancellation nobody spoke for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FormOutcome {
    Answered(Vec<AskUserAnswer>),
    Declined,
    Cancelled,
}

/// Property key carrying one option of a multi-select question.
fn option_key(question_id: &str, index: usize) -> String {
    format!("{question_id}__{index}")
}

/// The options, rendered as the trade-off lines a client shows under the
/// question. Option descriptions are the whole reason the model wrote them, so
/// they ride the `description` rather than being dropped for bare labels.
fn option_lines(question: &AskUserQuestion) -> String {
    question
        .options
        .iter()
        .map(|option| format!("{} — {}", option.label, option.description))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `requestedSchema` for a question set, or `None` when the set cannot be
/// projected (a question with no id could not be correlated back to an answer).
pub(super) fn requested_schema(questions: &[AskUserQuestion]) -> Option<Value> {
    let mut properties = Map::new();
    let mut required: Vec<Value> = Vec::new();

    for question in questions {
        // THREAT[TM-AGENT-016]: a credential is never a form property. Callers
        // screen secret questions out before they get here; this refuses one
        // anyway, so the boundary lives in the code that builds the schema and
        // cannot be lost by a caller that forgets.
        if question.kind == AskUserQuestionKind::Secret {
            return None;
        }
        let id = question.id.as_deref()?;
        if question.kind == AskUserQuestionKind::Text {
            properties.insert(
                id.to_string(),
                json!({
                    "type": "string",
                    "title": question.header,
                    "description": question.question,
                }),
            );
            required.push(json!(id));
            continue;
        }
        if question.options.is_empty() {
            return None;
        }
        let lines = option_lines(question);

        if question.multi_select {
            for (index, option) in question.options.iter().enumerate() {
                let key = option_key(id, index);
                let mut property = json!({
                    "type": "boolean",
                    "title": format!("{}: {}", question.header, option.label),
                    "description": format!("{}\n\n{} — {}", question.question, option.label, option.description),
                });
                if option.is_default {
                    property["default"] = json!(true);
                }
                required.push(json!(key));
                properties.insert(key, property);
            }
            continue;
        }

        let labels: Vec<Value> = question
            .options
            .iter()
            .map(|option| json!(option.label))
            .collect();
        let mut property = json!({
            "type": "string",
            "title": question.header,
            "description": format!("{}\n\n{lines}", question.question),
            // `enumNames` is the display half of the pair; the labels are what
            // the person chose between, so they are both the value and the name.
            "enum": labels,
            "enumNames": labels,
        });
        if let Some(default) = question.options.iter().find(|option| option.is_default) {
            property["default"] = json!(default.label);
        }
        required.push(json!(id));
        properties.insert(id.to_string(), property);
    }

    if properties.is_empty() {
        return None;
    }
    Some(json!({
        "type": "object",
        "properties": properties,
        "required": required,
    }))
}

/// Message the client shows above the form.
pub(super) fn form_message(questions: &[AskUserQuestion]) -> String {
    match questions {
        [only] => only.question.clone(),
        many => format!(
            "The agent needs {} decisions before it can continue.",
            many.len()
        ),
    }
}

/// Build the `InputRequiredResult` carrying one form mode elicitation, or
/// `None` when the question set cannot be projected into the profile.
pub(super) fn form_elicitation_result(
    questions: &[AskUserQuestion],
    signed: &str,
) -> Option<Value> {
    let request = form_input_request(questions)?;
    Some(json!({
        "resultType": "input_required",
        "inputRequests": {
            ASK_USER_REQUEST_KEY: request
        },
        "requestState": signed,
    }))
}

pub(super) fn form_input_request(questions: &[AskUserQuestion]) -> Option<Value> {
    let schema = requested_schema(questions)?;
    Some(json!({
        "method": "elicitation/create",
        "params": {
            "mode": "form",
            "message": form_message(questions),
            "requestedSchema": schema,
        }
    }))
}

/// Read what the client did with the form.
///
/// The selections here are *claims*, not conclusions: every label is checked
/// against the emitted question set by the shared resolution operation
/// (THREAT[TM-AGENT-015]), which is also what rejects a missing or malformed
/// answer. This only maps the wire shape onto the contract's.
pub(super) fn outcome_from_response(
    questions: &[AskUserQuestion],
    response: &Value,
) -> Result<FormOutcome, String> {
    match response.get("action").and_then(Value::as_str) {
        Some("accept") => {}
        Some("decline") => return Ok(FormOutcome::Declined),
        Some("cancel") => return Ok(FormOutcome::Cancelled),
        Some(other) => return Err(format!("Unknown elicitation action {other:?}")),
        None => return Err("Elicitation response carries no 'action'".to_string()),
    }
    let content = response
        .get("content")
        .ok_or("An accepted elicitation carries 'content'")?;
    Ok(FormOutcome::Answered(answers_from_content(
        questions, content,
    )))
}

/// Map submitted form content onto one answer per question.
///
/// A question the content says nothing about is left out rather than answered
/// empty, so validation reports the question nobody answered instead of a
/// selection nobody made.
fn answers_from_content(questions: &[AskUserQuestion], content: &Value) -> Vec<AskUserAnswer> {
    let mut answers = Vec::new();
    for question in questions {
        let Some(id) = question.id.as_deref() else {
            continue;
        };
        let (selected, other_text) = if question.kind == AskUserQuestionKind::Text {
            match content.get(id).and_then(Value::as_str) {
                Some(text) => (Vec::new(), Some(text.to_string())),
                None => continue,
            }
        } else if question.multi_select {
            let keys: Vec<String> = (0..question.options.len())
                .map(|index| option_key(id, index))
                .collect();
            if keys.iter().all(|key| content.get(key).is_none()) {
                continue;
            }
            (
                question
                    .options
                    .iter()
                    .zip(&keys)
                    .filter(|(_, key)| content.get(key).and_then(Value::as_bool) == Some(true))
                    .map(|(option, _)| option.label.clone())
                    .collect(),
                None,
            )
        } else {
            match content.get(id).and_then(Value::as_str) {
                Some(label) => (vec![label.to_string()], None),
                None => continue,
            }
        };
        answers.push(AskUserAnswer {
            id: id.to_string(),
            selected,
            other_text,
            secret_ref: None,
        });
    }
    answers
}

// ---------------------------------------------------------------------------
// Serving a form mode elicitation
// ---------------------------------------------------------------------------

use super::{
    AppState, AuthUser, JsonRpcResponse, ResolvedOrg, classify_mcp_execute_error, elicitation,
    error_result_payload, resolve_org_override, tool_registry,
};
use everruns_core::Caller;
use everruns_provider::typed_id::SessionId;

pub(super) async fn pending_questions_for_session(
    caller: &Caller,
    session_id: SessionId,
    state: &AppState,
) -> Result<Option<crate::api::question_answers::PendingQuestions>, String> {
    let session = state
        .session_service
        .get(caller, session_id.uuid(), None)
        .await
        .map_err(|error| error.to_string())?;
    let Some(session) = session else {
        return Ok(None);
    };
    if session.status != everruns_platform::SessionStatus::WaitingForToolResults {
        return Ok(None);
    }

    let events = state
        .db
        .list_events(
            session_id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(crate::api::question_answers::QUESTION_LOOKBACK_EVENTS),
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(crate::api::question_answers::pending_from_events(
        &events, None,
    ))
}

/// Serve an `ask_user` question set a session is parked on as a form mode
/// elicitation (EVE-1060), and apply the answer that comes back.
///
/// Returns `None` whenever the poll should be answered as an ordinary
/// `session_get_status` result: nothing is parked, the question set is not
/// projectable, the client cannot be elicited, or the client's answer just
/// resumed the turn and the status it is about to read is the news.
///
/// Deliberately unlike `handle_elicited_tool`: a client that never declared
/// `elicitation` gets no `MissingRequiredClientCapability` here. A credential
/// genuinely cannot be obtained any other way, but an unanswerable question
/// can — EVE-1057 already answered it with the model's declared defaults, so
/// failing the poll would turn a working headless run into an error.
#[allow(clippy::too_many_arguments)]
pub(super) async fn ask_user_form_elicitation(
    id: &Option<Value>,
    params: &Value,
    arguments: &Value,
    auth_user: &AuthUser,
    org: &ResolvedOrg,
    state: &AppState,
    protocol_version: &str,
) -> Option<JsonRpcResponse> {
    if !tool_registry::supports_mrtr(protocol_version) {
        return None;
    }
    let session_id: SessionId = arguments
        .get("session_id")
        .and_then(Value::as_str)?
        .parse()
        .ok()?;
    let org = resolve_org_override(arguments, auth_user, org, state)
        .await
        .ok()?;
    let caller = Caller::from(&org);
    let pending = pending_questions_for_session(&caller, session_id, state)
        .await
        .ok()??;
    // THREAT[TM-AGENT-016]: a credential is never a form property. An
    // `ask_user` answer is a tool result, so a secret typed into one would be
    // plaintext in the event log and in model context for the rest of the
    // session. A secret question set is left to the URL mode path
    // (`ElicitationIntent::SessionSecret`) and the browser card.
    if pending
        .questions
        .iter()
        .any(|question| question.kind == AskUserQuestionKind::Secret)
    {
        return None;
    }

    let intent = elicitation::QuestionSetIntent {
        session_id: session_id.to_string(),
        tool_call_id: pending.tool_call_id.clone(),
    };
    let signing_secret = state.auth.config.jwt.secret.clone();
    let now = chrono::Utc::now().timestamp();

    if let Some(request_state) = elicitation::request_state(params) {
        match elicitation::verify_token::<elicitation::QuestionSetIntent>(
            request_state,
            &signing_secret,
            auth_user.id,
            now,
        ) {
            // Only an answer to *this* question set counts. State minted for
            // another one is stale, not fatal: fall through and elicit afresh.
            Ok(token) if token.intent == intent => {
                if let Some(response) = elicitation::input_response(params, ASK_USER_REQUEST_KEY) {
                    return apply_form_answers(id, response, &caller, session_id, &pending, state)
                        .await;
                }
            }
            Ok(_) => {}
            Err(error) => {
                return Some(JsonRpcResponse::invalid_params(id.clone(), error.message()));
            }
        }
    }

    if !elicitation::client_supports_elicitation(params) {
        return None;
    }
    let signed = elicitation::sign_token(
        &elicitation::ElicitationToken::new(auth_user.id, org.public_id.clone(), intent, now),
        &signing_secret,
    );
    let result = form_elicitation_result(&pending.questions, &signed)?;
    tracing::info!(
        org.id = %org.public_id,
        questions = pending.questions.len(),
        "MCP form elicitation issued"
    );
    Some(JsonRpcResponse::success(id.clone(), result))
}

/// Apply a client's form response to the parked question set.
///
/// Routed through the one shared resolution operation (EVE-1054), which owns
/// validating every selection against what was actually asked, attribution, and
/// the single-claim resume — `/mcp` gets no second implementation of any of
/// them. A successful resolve returns `None` so the same `session_get_status`
/// call goes on to report the resumed session.
async fn apply_form_answers(
    id: &Option<Value>,
    response: &Value,
    caller: &Caller,
    session_id: SessionId,
    pending: &crate::api::question_answers::PendingQuestions,
    state: &AppState,
) -> Option<JsonRpcResponse> {
    use crate::api::question_answers::{QuestionResolver, ResolveError, resolve_question_answers};
    use everruns_builtins::ask_user::AskUserStatus;

    let outcome = match outcome_from_response(&pending.questions, response) {
        Ok(outcome) => outcome,
        Err(message) => return Some(JsonRpcResponse::invalid_params(id.clone(), message)),
    };
    // The trichotomy maps onto the contract's outcomes without translation. A
    // cancel is the one outcome the typed answer endpoint refuses to take from
    // a caller, and it is safe here because it carries no answers: dismissing
    // a form claims nothing on the person's behalf.
    let (status, answers) = match outcome {
        FormOutcome::Answered(answers) => (AskUserStatus::Answered, answers),
        FormOutcome::Declined => (AskUserStatus::Declined, Vec::new()),
        FormOutcome::Cancelled => (AskUserStatus::Cancelled, Vec::new()),
    };

    let resolver = QuestionResolver {
        db: &state.db,
        session_service: &state.session_service,
        event_service: &state.event_service,
        runner: state.runner.clone(),
    };
    match resolve_question_answers(
        &resolver,
        caller,
        session_id,
        Some(&pending.tool_call_id),
        status,
        &answers,
    )
    .await
    {
        Ok(_) => None,
        // Someone answered first, or the turn already moved on. The status
        // result this falls through to is a truer answer than an error.
        Err(
            ResolveError::AlreadyResolved
            | ResolveError::NotWaiting(_)
            | ResolveError::NoPendingQuestions
            | ResolveError::WrongPendingCall,
        ) => None,
        Err(ResolveError::Invalid(detail)) => {
            // The turn stays parked, so the client can correct the form and
            // answer again rather than losing the question.
            let envelope = classify_mcp_execute_error(&detail);
            Some(JsonRpcResponse::success(
                id.clone(),
                error_result_payload(&detail, Some(&envelope)),
            ))
        }
        Err(ResolveError::Internal(detail)) => {
            tracing::error!(error = %detail, "Failed to resolve an MCP form elicitation");
            let message = "Failed to record the answer".to_string();
            let envelope = classify_mcp_execute_error(&message);
            Some(JsonRpcResponse::success(
                id.clone(),
                error_result_payload(&message, Some(&envelope)),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_builtins::ask_user::AskUserOption;

    fn option(label: &str, is_default: bool) -> AskUserOption {
        AskUserOption {
            label: label.to_string(),
            description: format!("{label} trade-off"),
            is_default,
        }
    }

    fn question(id: &str, multi_select: bool, options: Vec<AskUserOption>) -> AskUserQuestion {
        AskUserQuestion {
            kind: AskUserQuestionKind::Choice,
            id: Some(id.to_string()),
            header: "Target".to_string(),
            question: "Which environment?".to_string(),
            multi_select,
            allow_other: true,
            options,
            secret_name: None,
            purpose: None,
        }
    }

    fn text_question(id: &str) -> AskUserQuestion {
        AskUserQuestion {
            kind: AskUserQuestionKind::Text,
            id: Some(id.to_string()),
            header: "Branch".to_string(),
            question: "What should I call this branch?".to_string(),
            multi_select: false,
            allow_other: true,
            options: Vec::new(),
            secret_name: None,
            purpose: None,
        }
    }

    #[test]
    fn a_single_select_question_becomes_one_enum_property() {
        let questions = vec![question(
            "target",
            false,
            vec![option("Staging", true), option("Production", false)],
        )];
        let schema = requested_schema(&questions).expect("projectable");
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!(["target"]));
        let property = &schema["properties"]["target"];
        assert_eq!(property["type"], "string");
        assert_eq!(property["enum"], json!(["Staging", "Production"]));
        assert_eq!(property["enumNames"], json!(["Staging", "Production"]));
        assert_eq!(property["default"], "Staging");
        // The trade-offs the model wrote survive into what the client renders.
        let description = property["description"].as_str().unwrap();
        assert!(description.contains("Which environment?"));
        assert!(description.contains("Staging — Staging trade-off"));
        assert!(description.contains("Production — Production trade-off"));
    }

    /// The decision in the module header: booleans, not an array or a joined
    /// string, because the profile has no array.
    #[test]
    fn a_multi_select_question_becomes_one_boolean_per_option() {
        let questions = vec![question(
            "areas",
            true,
            vec![option("API", false), option("UI", true)],
        )];
        let schema = requested_schema(&questions).expect("projectable");
        assert_eq!(schema["required"], json!(["areas__0", "areas__1"]));
        assert_eq!(schema["properties"]["areas__0"]["type"], "boolean");
        assert_eq!(schema["properties"]["areas__0"]["title"], "Target: API");
        assert!(schema["properties"]["areas__0"].get("default").is_none());
        assert_eq!(schema["properties"]["areas__1"]["default"], json!(true));
        assert_eq!(schema["properties"]["areas__1"]["title"], "Target: UI");
    }

    #[test]
    fn a_text_question_becomes_a_plain_string_property() {
        let questions = vec![text_question("branch_name")];
        let schema = requested_schema(&questions).expect("projectable");
        assert_eq!(schema["required"], json!(["branch_name"]));
        let property = &schema["properties"]["branch_name"];
        assert_eq!(property["type"], "string");
        assert_eq!(property["title"], "Branch");
        assert_eq!(property["description"], "What should I call this branch?");
        assert!(property.get("enum").is_none());
        assert!(property.get("default").is_none());
    }

    /// THREAT[TM-AGENT-016]: the schema builder itself refuses a credential
    /// question, so the boundary does not depend on every caller remembering.
    #[test]
    fn a_secret_question_is_never_projectable() {
        let secret = AskUserQuestion {
            kind: AskUserQuestionKind::Secret,
            id: Some("stripe".to_string()),
            header: "Stripe key".to_string(),
            question: "Which Stripe restricted key should I use?".to_string(),
            multi_select: false,
            allow_other: false,
            options: Vec::new(),
            secret_name: Some("STRIPE_API_KEY".to_string()),
            purpose: Some("Read-only charge lookups.".to_string()),
        };
        assert!(requested_schema(std::slice::from_ref(&secret)).is_none());
        assert!(form_elicitation_result(std::slice::from_ref(&secret), "state").is_none());
        // Nor alongside a choice question the model could otherwise be asked.
        let mixed = vec![
            question("target", false, vec![option("Staging", false)]),
            secret,
        ];
        assert!(requested_schema(&mixed).is_none());
    }

    #[test]
    fn a_question_with_no_id_is_not_projectable() {
        let mut questions = vec![question("target", false, vec![option("Staging", false)])];
        questions[0].id = None;
        assert!(requested_schema(&questions).is_none());
        assert!(form_elicitation_result(&questions, "state").is_none());
    }

    #[test]
    fn the_result_carries_a_form_mode_elicitation() {
        let questions = vec![question("target", false, vec![option("Staging", false)])];
        let result = form_elicitation_result(&questions, "state").expect("projectable");
        assert_eq!(result["resultType"], "input_required");
        assert_eq!(result["requestState"], "state");
        let request = &result["inputRequests"]["ask_user"];
        assert_eq!(request["method"], "elicitation/create");
        assert_eq!(request["params"]["mode"], "form");
        assert_eq!(request["params"]["message"], "Which environment?");
        assert_eq!(request["params"]["requestedSchema"]["type"], "object");
    }

    #[test]
    fn an_accepted_form_round_trips_both_question_shapes() {
        let questions = vec![
            question(
                "target",
                false,
                vec![option("Staging", false), option("Production", false)],
            ),
            question(
                "areas",
                true,
                vec![option("API", false), option("UI", false)],
            ),
        ];
        let response = json!({
            "action": "accept",
            "content": {
                "target": "Production",
                "areas__0": true,
                "areas__1": false,
            }
        });
        let FormOutcome::Answered(answers) = outcome_from_response(&questions, &response).unwrap()
        else {
            panic!("expected answers");
        };
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].selected, vec!["Production".to_string()]);
        assert_eq!(answers[1].selected, vec!["API".to_string()]);
        assert!(answers.iter().all(|answer| answer.secret_ref.is_none()));
    }

    #[test]
    fn an_accepted_form_round_trips_a_text_answer() {
        let questions = vec![text_question("branch_name")];
        let response = json!({
            "action": "accept",
            "content": { "branch_name": "feature/open-question" }
        });
        let FormOutcome::Answered(answers) = outcome_from_response(&questions, &response).unwrap()
        else {
            panic!("expected answers");
        };

        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].id, "branch_name");
        assert!(answers[0].selected.is_empty());
        assert_eq!(
            answers[0].other_text.as_deref(),
            Some("feature/open-question")
        );
    }

    #[test]
    fn a_question_the_content_skips_is_left_unanswered() {
        let questions = vec![
            question("target", false, vec![option("Staging", false)]),
            question("areas", true, vec![option("API", false)]),
        ];
        let response = json!({ "action": "accept", "content": { "target": "Staging" } });
        let FormOutcome::Answered(answers) = outcome_from_response(&questions, &response).unwrap()
        else {
            panic!("expected answers");
        };
        // Only `target`. `areas` reaches validation as unanswered, which names
        // the question rather than inventing an empty selection for it.
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].id, "target");
    }

    #[test]
    fn decline_and_cancel_stay_distinct() {
        let questions = vec![question("target", false, vec![option("Staging", false)])];
        assert_eq!(
            outcome_from_response(&questions, &json!({ "action": "decline" })).unwrap(),
            FormOutcome::Declined
        );
        assert_eq!(
            outcome_from_response(&questions, &json!({ "action": "cancel" })).unwrap(),
            FormOutcome::Cancelled
        );
        assert!(outcome_from_response(&questions, &json!({ "action": "accept" })).is_err());
        assert!(outcome_from_response(&questions, &json!({})).is_err());
    }
}
