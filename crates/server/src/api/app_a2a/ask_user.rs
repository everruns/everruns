// `ask_user` over the inbound A2A channel (EVE-1062).
//
// A2A 0.3 has no elicitation or form primitive, so a parked `ask_user` call is
// projected onto the extension point the protocol does have: a namespaced,
// schema-declared `DataPart` on `TaskStatus.message`. The same message always
// carries a **text part** rendering the identical question in prose — every
// A2A consumer that shipped before this projection reads text and nothing
// else, so the data part adds information and never replaces it.
//
// The answer travels back as a `DataPart` on a `message/send` naming the same
// task, and lands on the shared question-resolution operation (EVE-1054) that
// the browser card and `/mcp` already use. A text-only reply keeps its current
// meaning: the message supersedes the question, which `MessageService::create`
// resolves as `cancelled`.

use std::sync::Arc;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    AppA2aState, AuthorizedA2a, build_task_json, derive_task_state_from_events, internal_error,
    legacy_task_json, rpc_error, rpc_success, session_belongs_to_a2a_channel,
};
use crate::storage::StorageBackend;

/// Find the `ask_user` answer a caller put on the message, if any.
///
/// The payload under [`ASK_USER_ANSWER_KEY`] is the
/// `/v1/sessions/{id}/question-answers` request body verbatim. Reusing that
/// type rather than minting an A2A-shaped twin is what keeps every surface
/// answering `ask_user` in one vocabulary; the schema advertised on the way
/// out describes exactly this shape.
pub(super) fn parse_ask_user_answer_part(
    parts: &[Value],
) -> Result<Option<crate::api::question_answers::QuestionAnswersRequest>, serde_json::Error> {
    for part in parts {
        // The same kind/type/duck-typing tolerance `parse_message_params`
        // applies to text parts.
        let kind = part.get("kind").or_else(|| part.get("type"));
        let is_data = match kind {
            Some(kind) => kind.as_str() == Some("data"),
            None => part.get("data").is_some(),
        };
        if !is_data {
            continue;
        }
        let Some(payload) = part
            .get("data")
            .and_then(|data| data.get(ASK_USER_ANSWER_KEY))
        else {
            continue;
        };
        return serde_json::from_value(payload.clone()).map(Some);
    }
    Ok(None)
}

/// Envelope key carrying a question set out to an A2A caller.
const ASK_USER_QUESTION_KEY: &str = "everruns/ask_user";
/// Envelope key an A2A caller answers with, on the same task id.
const ASK_USER_ANSWER_KEY: &str = "everruns/ask_user_answer";
/// Envelope key naming where a human completes a `secret` question.
const ASK_USER_AUTH_KEY: &str = "everruns/auth_required";
/// Version of the data-part contract, so a consumer can refuse a shape it does
/// not know rather than guess.
const ASK_USER_DATA_VERSION: u32 = 1;

/// A parked `ask_user` call, projected onto the A2A task.
pub(super) struct AskUserProjection {
    /// `input_required`, or `auth_required` when a credential is wanted.
    pub(super) state: &'static str,
    /// `TaskStatus.message`: prose first, typed data second.
    pub(super) message: Value,
}

/// The `ask_user` question set a session is parked on, if any.
///
/// Turn lifecycle events cannot see this — the turn that asked is still open,
/// so `derive_task_state_from_events` reports `working` for a session that is
/// in fact waiting on a person. The session status is what says it parked.
pub(super) async fn pending_ask_user(
    db: &Arc<StorageBackend>,
    session: &crate::storage::SessionRow,
) -> anyhow::Result<Option<crate::api::question_answers::PendingQuestions>> {
    if everruns_platform::SessionStatus::from(session.status.as_str())
        != everruns_platform::SessionStatus::WaitingForToolResults
    {
        return Ok(None);
    }
    let events = db
        .list_events(
            session.id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(crate::api::question_answers::QUESTION_LOOKBACK_EVENTS),
        )
        .await?;
    Ok(crate::api::question_answers::pending_from_events(
        &events, None,
    ))
}

/// The `ask_user` call inside a streamed `tool.call_requested`, if there is one.
///
/// The typed twin of `pending_from_events`, which reads the same payload back
/// out of a stored row.
pub(super) fn pending_ask_user_from_request(
    requested: &everruns_core::events::ToolCallRequestedData,
) -> Option<crate::api::question_answers::PendingQuestions> {
    let call = requested
        .tool_calls
        .iter()
        .find(|call| call.name == everruns_builtins::ask_user::ASK_USER_TOOL_NAME)?;
    let questions = serde_json::from_value(call.arguments.get("questions")?.clone()).ok()?;
    let expires_at = call
        .arguments
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc));
    Some(crate::api::question_answers::PendingQuestions {
        tool_call_id: call.id.clone(),
        questions,
        expires_at,
    })
}

/// Where a person completes a `secret` question, when a UI origin is configured.
fn session_human_url(frontend_url: &str, task_id: &str) -> Option<String> {
    let base = frontend_url.trim_end_matches('/');
    (!base.is_empty()).then(|| format!("{base}/sessions/{task_id}/chat"))
}

/// Project a parked question set onto an A2A task status.
pub(super) fn project_ask_user(
    pending: &crate::api::question_answers::PendingQuestions,
    task_id: &str,
    frontend_url: &str,
) -> AskUserProjection {
    use everruns_builtins::ask_user::AskUserQuestionKind;

    // THREAT[TM-AGENT-016]: a remote agent is never prompted for a human's
    // credential. A `secret` question projects as `auth_required` carrying a
    // URL a person opens — never as a data part with a field to fill in, and
    // never with anywhere for a value to travel back through.
    if pending
        .questions
        .iter()
        .any(|question| question.kind == AskUserQuestionKind::Secret)
    {
        let url = session_human_url(frontend_url, task_id);
        let wanted: Vec<Value> = pending
            .questions
            .iter()
            .filter(|question| question.kind == AskUserQuestionKind::Secret)
            .map(|question| {
                json!({
                    "header": question.header,
                    "question": question.question,
                    "secret_name": question.secret_name,
                    "purpose": question.purpose,
                })
            })
            .collect();
        let data = json!({
            ASK_USER_AUTH_KEY: {
                "version": ASK_USER_DATA_VERSION,
                "task_id": task_id,
                "reason": "secret_question",
                "url": url.clone(),
                "credentials": wanted,
            }
        });
        return AskUserProjection {
            state: "auth_required",
            message: a2a_status_message(
                task_id,
                vec![
                    json!({ "kind": "text", "text": render_secret_prose(&pending.questions, url.as_deref()) }),
                    json!({ "kind": "data", "data": data }),
                ],
            ),
        };
    }

    let data = json!({
        ASK_USER_QUESTION_KEY: {
            "version": ASK_USER_DATA_VERSION,
            "task_id": task_id,
            "tool_call_id": pending.tool_call_id,
            "expires_at": pending.expires_at.map(|at| at.to_rfc3339()),
            "questions": pending.questions,
            "answer_schema": ask_user_answer_schema(pending),
        }
    });
    AskUserProjection {
        state: "input_required",
        message: a2a_status_message(
            task_id,
            vec![
                json!({ "kind": "text", "text": render_questions_prose(pending, task_id) }),
                json!({ "kind": "data", "data": data }),
            ],
        ),
    }
}

/// An A2A `Message` carrying the projection, for `TaskStatus.message`.
fn a2a_status_message(task_id: &str, parts: Vec<Value>) -> Value {
    json!({
        "kind": "message",
        "role": "agent",
        "messageId": Uuid::now_v7().to_string(),
        "taskId": task_id,
        "contextId": task_id,
        "parts": parts,
    })
}

/// JSON Schema for the answer data part, declared alongside the question.
///
/// This is what makes a bare `DataPart` usable in place of the elicitation
/// primitive A2A does not have: the caller is told the exact object to send
/// back, down to the option labels it is allowed to select.
fn ask_user_answer_schema(pending: &crate::api::question_answers::PendingQuestions) -> Value {
    let answers: Vec<Value> = pending
        .questions
        .iter()
        .map(|question| {
            use everruns_builtins::ask_user::AskUserQuestionKind;
            let labels: Vec<&str> = question
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect();
            let mut properties = serde_json::Map::new();
            properties.insert(
                "id".to_string(),
                json!({ "const": question.id.clone().unwrap_or_default() }),
            );
            if question.kind == AskUserQuestionKind::Text {
                properties.insert(
                    "other_text".to_string(),
                    json!({
                        "type": "string",
                        "description": "The free-form answer.",
                    }),
                );
                return json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "other_text"],
                    "properties": properties,
                    "description": question.question,
                });
            }
            let mut selected = json!({
                "type": "array",
                "items": { "enum": labels },
            });
            if !question.multi_select
                && let Some(object) = selected.as_object_mut()
            {
                object.insert("maxItems".to_string(), json!(1));
            }
            properties.insert("selected".to_string(), selected);
            if question.allow_other {
                properties.insert(
                    "other_text".to_string(),
                    json!({
                        "type": ["string", "null"],
                        "description": "Free text, when none of the options fit.",
                    }),
                );
            }
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["id"],
                "properties": properties,
                "description": question.question,
            })
        })
        .collect();

    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": ASK_USER_ANSWER_KEY,
        "type": "object",
        "additionalProperties": false,
        "required": ["answers"],
        "properties": {
            "tool_call_id": { "const": pending.tool_call_id },
            "status": {
                "enum": ["answered", "declined"],
                "default": "answered",
                "description": "`declined` is a finished decision the agent must not re-ask.",
            },
            "answers": {
                "type": "array",
                "minItems": answers.len(),
                "maxItems": answers.len(),
                "description": "One answer per asked question, in any order.",
                "items": { "oneOf": answers },
            },
        },
    })
}

/// The question set in prose, for the caller that reads only text.
fn render_questions_prose(
    pending: &crate::api::question_answers::PendingQuestions,
    task_id: &str,
) -> String {
    let mut out =
        String::from("This task is waiting on an answer before the agent can continue.\n");
    if let Some(expires_at) = pending.expires_at {
        let resolution =
            if everruns_builtins::ask_user::questions_have_no_default_answer(&pending.questions) {
                "it skips the unanswered question set"
            } else {
                "it continues with the defaults marked below"
            };
        out.push_str(&format!(
            "Unanswered by {}, {resolution}.\n",
            expires_at.to_rfc3339(),
        ));
    }
    for (index, question) in pending.questions.iter().enumerate() {
        out.push_str(&format!(
            "\n{}. {} — {}\n",
            index + 1,
            question.header,
            question.question
        ));
        if !question.options.is_empty() {
            out.push_str(if question.multi_select {
                "   Choose any that apply:\n"
            } else {
                "   Choose one:\n"
            });
            for option in &question.options {
                out.push_str(&format!(
                    "   - {}: {}{}\n",
                    option.label,
                    option.description,
                    if option.is_default { " (default)" } else { "" }
                ));
            }
        }
        if question.kind == everruns_builtins::ask_user::AskUserQuestionKind::Text {
            out.push_str("   Answer in your own words with `other_text`.\n");
        } else if question.allow_other {
            out.push_str("   Or answer in your own words with `other_text`.\n");
        }
    }
    out.push_str(&format!(
        "\nAnswer with message/send on taskId {task_id}, carrying a data part whose `data` holds \
         `{ASK_USER_ANSWER_KEY}` — `answer_schema` in the data part of this message gives the \
         exact shape. Replying with text only cancels the question and reaches the agent as an \
         ordinary message instead.\n"
    ));
    out
}

/// A credential request in prose. It says what is wanted and who provides it,
/// and never invites the reader to supply the value.
fn render_secret_prose(
    questions: &[everruns_builtins::ask_user::AskUserQuestion],
    url: Option<&str>,
) -> String {
    use everruns_builtins::ask_user::AskUserQuestionKind;

    let mut out = String::from(
        "This task needs a credential, which is never requested from a calling agent and never \
         travels over A2A. A person with access to the session provides it directly to Everruns; \
         the task stays in auth_required until they do.\n",
    );
    for question in questions
        .iter()
        .filter(|question| question.kind == AskUserQuestionKind::Secret)
    {
        out.push_str(&format!("\n{} — {}\n", question.header, question.question));
        if let Some(name) = &question.secret_name {
            out.push_str(&format!("   Stored as: {name}\n"));
        }
        if let Some(purpose) = &question.purpose {
            out.push_str(&format!("   Used for: {purpose}\n"));
        }
    }
    match url {
        Some(url) => out.push_str(&format!("\nOpen {url} to provide it.\n")),
        None => out.push_str(
            "\nNo public UI origin is configured on this deployment, so there is no link to \
             follow; reach the session owner another way.\n",
        ),
    }
    out
}

/// Attach a projected `TaskStatus.message` to a task object.
pub(super) fn with_status_message(mut task: Value, message: Option<Value>) -> Value {
    if let Some(message) = message
        && let Some(status) = task.get_mut("status").and_then(Value::as_object_mut)
    {
        status.insert("message".to_string(), message);
    }
    task
}

/// Complete the question set this task is parked on, and resume the turn.
///
/// Validation, attribution and idempotency all belong to the shared resolution
/// operation (EVE-1054) — this function only establishes that the caller is
/// allowed to answer *this* task, and that what it is answering is not a
/// credential.
pub(super) async fn handle_ask_user_answer(
    state: &AppA2aState,
    auth: &AuthorizedA2a,
    task_id: Option<&str>,
    submission: crate::api::question_answers::QuestionAnswersRequest,
    rpc_id: Value,
    wrap_legacy_send_response: bool,
) -> Response {
    use crate::api::question_answers::{QuestionResolver, ResolveError, SubmittedStatus};
    use everruns_builtins::ask_user::{AskUserAnswer, AskUserQuestionKind, AskUserStatus};

    let invalid = |rpc_id: Value, detail: &str| -> Response {
        (
            StatusCode::OK,
            rpc_error(rpc_id, -32602, format!("Invalid params: {detail}")),
        )
            .into_response()
    };

    let Some(task_id) = task_id else {
        return invalid(
            rpc_id,
            "an ask_user answer must carry message.taskId naming the task that asked",
        );
    };
    let Ok(session_id) = task_id.parse::<everruns_provider::typed_id::SessionId>() else {
        return invalid(rpc_id, "message.taskId is not a known task id");
    };

    let session = match state.db.get_session(auth.org_id, session_id).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            return (StatusCode::OK, rpc_error(rpc_id, -32001, "Task not found")).into_response();
        }
        Err(err) => return internal_error(err).into_response(),
    };
    // THREAT[TM-A2A-012]: the same channel binding `tasks/get` enforces. An API
    // key for one channel must not be able to answer — and so steer — a turn
    // running behind another channel in the same org.
    if !session_belongs_to_a2a_channel(&session, auth) {
        return (StatusCode::OK, rpc_error(rpc_id, -32001, "Task not found")).into_response();
    }

    // THREAT[TM-AGENT-016]: refuse a credential answer at the channel boundary,
    // whatever the payload claims, so "a remote agent is never asked for a
    // human's credential" is a property of A2A rather than of validation
    // downstream.
    match pending_ask_user(&state.db, &session).await {
        Ok(Some(pending))
            if pending
                .questions
                .iter()
                .any(|question| question.kind == AskUserQuestionKind::Secret) =>
        {
            return invalid(
                rpc_id,
                "this task is waiting on a credential, which A2A never carries. A person \
                 completes it from the URL in the task status message.",
            );
        }
        Ok(_) => {}
        Err(err) => return internal_error(err).into_response(),
    }
    if submission
        .answers
        .iter()
        .any(|answer| answer.secret_ref.is_some())
    {
        return invalid(rpc_id, "secret_ref is not accepted over A2A");
    }

    let status = match submission.status {
        SubmittedStatus::Answered => AskUserStatus::Answered,
        SubmittedStatus::Declined => AskUserStatus::Declined,
    };
    let answers: Vec<AskUserAnswer> = submission
        .answers
        .into_iter()
        .map(AskUserAnswer::from)
        .collect();

    let resolver = QuestionResolver {
        db: &state.db,
        session_service: state.session_service.as_ref(),
        event_service: state.message_service.event_service(),
        runner: state.message_service.runner().clone(),
    };
    // Attribution is the channel's, never the payload's: the answer came from
    // an API-key holder, which is exactly what `Caller::internal` records.
    let caller = everruns_core::Caller::internal(auth.org_id);
    if let Err(error) = crate::api::question_answers::resolve_question_answers(
        &resolver,
        &caller,
        session_id,
        submission.tool_call_id.as_deref(),
        status,
        &answers,
    )
    .await
    {
        let detail = match error {
            ResolveError::NoPendingQuestions => {
                "this task is not waiting on a question set".to_string()
            }
            ResolveError::WrongPendingCall => {
                "this task is waiting on a different call".to_string()
            }
            ResolveError::AlreadyResolved => {
                "this question set has already been answered".to_string()
            }
            ResolveError::NotWaiting(current) => {
                format!("this task is not waiting for an answer (status: {current})")
            }
            ResolveError::Invalid(detail) => detail,
            ResolveError::Internal(detail) => {
                tracing::error!(error = %detail, "A2A ask_user answer failed to resolve");
                return internal_error(anyhow::anyhow!("failed to resolve the ask_user answer"))
                    .into_response();
            }
        };
        return invalid(rpc_id, &detail);
    }

    let state_label = match derive_task_state_from_events(&state.db, session_id).await {
        Ok(label) => label,
        Err(err) => return internal_error(err).into_response(),
    };
    let task = build_task_json(session_id, state_label, None);
    let result = if wrap_legacy_send_response {
        json!({ "task": legacy_task_json(task) })
    } else {
        task
    };
    (StatusCode::OK, rpc_success(rpc_id, result)).into_response()
}

#[cfg(test)]
mod tests {
    use super::super::{parse_message_params, translate_session_event};
    use super::*;
    use everruns_core::events::EventData;

    fn choice_question() -> everruns_builtins::ask_user::AskUserQuestion {
        everruns_builtins::ask_user::AskUserQuestion {
            kind: everruns_builtins::ask_user::AskUserQuestionKind::Choice,
            id: Some("target".to_string()),
            header: "Target".to_string(),
            question: "Which environment should I deploy to?".to_string(),
            multi_select: false,
            allow_other: true,
            options: vec![
                everruns_builtins::ask_user::AskUserOption {
                    label: "Staging".to_string(),
                    description: "Safe, reversible.".to_string(),
                    is_default: true,
                },
                everruns_builtins::ask_user::AskUserOption {
                    label: "Production".to_string(),
                    description: "Live traffic.".to_string(),
                    is_default: false,
                },
            ],
            secret_name: None,
            purpose: None,
        }
    }

    fn secret_question() -> everruns_builtins::ask_user::AskUserQuestion {
        everruns_builtins::ask_user::AskUserQuestion {
            kind: everruns_builtins::ask_user::AskUserQuestionKind::Secret,
            id: Some("stripe_key".to_string()),
            header: "Stripe key".to_string(),
            question: "Which Stripe restricted key should I use?".to_string(),
            multi_select: false,
            allow_other: false,
            options: Vec::new(),
            secret_name: Some("STRIPE_API_KEY".to_string()),
            purpose: Some("Read-only charge lookups.".to_string()),
        }
    }

    fn text_question() -> everruns_builtins::ask_user::AskUserQuestion {
        everruns_builtins::ask_user::AskUserQuestion {
            kind: everruns_builtins::ask_user::AskUserQuestionKind::Text,
            id: Some("branch_name".to_string()),
            header: "Branch".to_string(),
            question: "What should I call this branch?".to_string(),
            multi_select: false,
            allow_other: true,
            options: Vec::new(),
            secret_name: None,
            purpose: None,
        }
    }

    fn pending(
        questions: Vec<everruns_builtins::ask_user::AskUserQuestion>,
    ) -> crate::api::question_answers::PendingQuestions {
        crate::api::question_answers::PendingQuestions {
            tool_call_id: "call_1".to_string(),
            questions,
            expires_at: None,
        }
    }

    /// The text part is the whole compatibility story: a consumer that only
    /// reads text must still learn what was asked.
    #[test]
    fn ask_user_projection_carries_prose_and_typed_data() {
        let projection = project_ask_user(&pending(vec![choice_question()]), "task-1", "");
        assert_eq!(projection.state, "input_required");
        let parts = projection.message["parts"].as_array().unwrap();

        let text = parts
            .iter()
            .find(|part| part["kind"] == "text")
            .and_then(|part| part["text"].as_str())
            .expect("a text part");
        assert!(text.contains("Which environment should I deploy to?"));
        assert!(text.contains("Staging: Safe, reversible. (default)"));
        assert!(text.contains(ASK_USER_ANSWER_KEY));

        let data = parts
            .iter()
            .find(|part| part["kind"] == "data")
            .map(|part| &part["data"][ASK_USER_QUESTION_KEY])
            .expect("a data part");
        assert_eq!(data["tool_call_id"], "call_1");
        assert_eq!(data["questions"][0]["id"], "target");
        assert_eq!(
            data["answer_schema"]["properties"]["answers"]["items"]["oneOf"][0]["properties"]["selected"]
                ["items"]["enum"],
            json!(["Staging", "Production"])
        );
        // Single-select is expressed in the schema, not only in the prose.
        assert_eq!(
            data["answer_schema"]["properties"]["answers"]["items"]["oneOf"][0]["properties"]["selected"]
                ["maxItems"],
            json!(1)
        );
    }

    #[test]
    fn text_question_projects_as_a_free_text_field_without_options() {
        let projection = project_ask_user(&pending(vec![text_question()]), "task-1", "");
        assert_eq!(projection.state, "input_required");
        let data = projection.message["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|part| part["kind"] == "data")
            .map(|part| &part["data"][ASK_USER_QUESTION_KEY])
            .expect("a data part");
        let answer = &data["answer_schema"]["properties"]["answers"]["items"]["oneOf"][0];
        assert_eq!(answer["required"], json!(["id", "other_text"]));
        assert_eq!(answer["properties"]["other_text"]["type"], "string");
        assert!(answer["properties"].get("selected").is_none());
        assert!(
            data["questions"][0]["options"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let text = projection.message["parts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|part| part["kind"] == "text")
            .and_then(|part| part["text"].as_str())
            .expect("a text part");
        assert!(text.contains("Answer in your own words with `other_text`."));
    }

    /// THREAT[TM-AGENT-016]: the credential question is never projected as
    /// something the calling agent could answer.
    #[test]
    fn secret_question_projects_as_auth_required_with_a_url() {
        let projection = project_ask_user(
            &pending(vec![secret_question()]),
            "session_1",
            "https://app.example.test/",
        );
        assert_eq!(projection.state, "auth_required");
        let rendered = projection.message.to_string();
        assert!(
            !rendered.contains(ASK_USER_QUESTION_KEY),
            "no answerable question set: {rendered}"
        );
        assert!(!rendered.contains("answer_schema"), "{rendered}");
        let parts = projection.message["parts"].as_array().unwrap();
        let auth = parts
            .iter()
            .find(|part| part["kind"] == "data")
            .map(|part| &part["data"][ASK_USER_AUTH_KEY])
            .expect("a data part");
        assert_eq!(auth["reason"], "secret_question");
        assert_eq!(
            auth["url"],
            "https://app.example.test/sessions/session_1/chat"
        );
    }

    /// A deployment with no configured UI origin still parks correctly; it just
    /// has no link to hand over.
    #[test]
    fn secret_question_without_a_frontend_url_still_projects_auth_required() {
        let projection = project_ask_user(&pending(vec![secret_question()]), "session_1", "");
        assert_eq!(projection.state, "auth_required");
        let parts = projection.message["parts"].as_array().unwrap();
        let auth = parts
            .iter()
            .find(|part| part["kind"] == "data")
            .map(|part| &part["data"][ASK_USER_AUTH_KEY])
            .expect("a data part");
        assert!(auth["url"].is_null());
    }

    /// A typed answer is the one message shape that legitimately has no text.
    #[test]
    fn parse_message_params_accepts_a_data_part_answer_without_text() {
        let params = json!({
            "message": {
                "role": "user",
                "taskId": "session_1",
                "parts": [{
                    "kind": "data",
                    "data": {
                        ASK_USER_ANSWER_KEY: {
                            "tool_call_id": "call_1",
                            "answers": [{ "id": "target", "selected": ["Staging"] }]
                        }
                    }
                }],
            }
        });
        let parsed = parse_message_params(&params).unwrap();
        assert_eq!(parsed.task_id.as_deref(), Some("session_1"));
        let answer = parsed.answer.expect("the answer was recognised");
        assert_eq!(answer.tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(answer.answers[0].selected, vec!["Staging".to_string()]);
    }

    /// A data part that is not an answer leaves the text rule alone.
    #[test]
    fn parse_message_params_ignores_unrelated_data_parts() {
        let params = json!({
            "message": {
                "role": "user",
                "parts": [{ "kind": "data", "data": { "something/else": { "a": 1 } } }],
            }
        });
        assert!(parse_message_params(&params).is_err());
    }

    #[test]
    fn parse_message_params_rejects_a_malformed_answer_part() {
        let params = json!({
            "message": {
                "role": "user",
                "parts": [{
                    "kind": "data",
                    "data": { ASK_USER_ANSWER_KEY: { "answers": "not an array" } }
                }],
            }
        });
        assert!(parse_message_params(&params).is_err());
    }

    /// A parked question is the one non-terminal stop the stream has; without
    /// this frame the caller waits on a question it cannot see.
    #[test]
    fn translate_parked_ask_user_emits_a_final_input_required_frame() {
        use everruns_core::events::ToolCallRequestedData;
        let data = EventData::ToolCallRequested(ToolCallRequestedData {
            tool_calls: vec![everruns_provider::tool_types::ToolCall {
                id: "call_1".to_string(),
                name: everruns_builtins::ask_user::ASK_USER_TOOL_NAME.to_string(),
                arguments: json!({ "questions": [choice_question()] }),
            }],
            tool_summaries: Vec::new(),
            headline: None,
            completed_headline: None,
        });
        let frame = translate_session_event(&data, "task-1", "ctx-1", "").unwrap();
        assert_eq!(frame["status"]["state"], "input_required");
        assert_eq!(frame["final"], true);
        assert!(
            frame["status"]["message"]["parts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|part| part["kind"] == "text")
        );
    }

    /// Any other client-side tool keeps its current meaning: no A2A frame.
    #[test]
    fn translate_non_ask_user_tool_call_is_filtered_out() {
        use everruns_core::events::ToolCallRequestedData;
        let data = EventData::ToolCallRequested(ToolCallRequestedData {
            tool_calls: vec![everruns_provider::tool_types::ToolCall {
                id: "call_1".to_string(),
                name: "setup_connection".to_string(),
                arguments: json!({}),
            }],
            tool_summaries: Vec::new(),
            headline: None,
            completed_headline: None,
        });
        assert!(translate_session_event(&data, "task-1", "ctx-1", "").is_none());
    }
}
