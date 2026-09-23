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
//    be plaintext in the event log and in model context forever. Secret
//    questions are excluded before we get here (`mod.rs`), and the one thing
//    this module must never grow is a property that could carry a value.

use everruns_builtins::ask_user::{AskUserAnswer, AskUserQuestion};
use serde_json::{Map, Value, json};

/// Key form-mode elicitations are delivered under in `inputRequests`. Stable,
/// for the same reason URL mode's keys are: a client retrying sees a coherent
/// key across rounds.
pub(super) const ASK_USER_REQUEST_KEY: &str = "ask_user";

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
        let id = question.id.as_deref()?;
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
    let schema = requested_schema(questions)?;
    Some(json!({
        "resultType": "input_required",
        "inputRequests": {
            ASK_USER_REQUEST_KEY: {
                "method": "elicitation/create",
                "params": {
                    "mode": "form",
                    "message": form_message(questions),
                    "requestedSchema": schema,
                }
            }
        },
        "requestState": signed,
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
        let selected = if question.multi_select {
            let keys: Vec<String> = (0..question.options.len())
                .map(|index| option_key(id, index))
                .collect();
            if keys.iter().all(|key| content.get(key).is_none()) {
                continue;
            }
            question
                .options
                .iter()
                .zip(&keys)
                .filter(|(_, key)| content.get(key).and_then(Value::as_bool) == Some(true))
                .map(|(option, _)| option.label.clone())
                .collect()
        } else {
            match content.get(id).and_then(Value::as_str) {
                Some(label) => vec![label.to_string()],
                None => continue,
            }
        };
        answers.push(AskUserAnswer {
            id: id.to_string(),
            selected,
            other_text: None,
            secret_ref: None,
        });
    }
    answers
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_builtins::ask_user::{AskUserOption, AskUserQuestionKind};

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
