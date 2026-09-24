//! Unit tests for [`super`], the `ask_user` capability.
//!
//! Split out of `ask_user.rs` to keep it under the file-size ratchet.

use super::*;
use chrono::Datelike;

fn option(label: &str, is_default: bool) -> Value {
    json!({
        "label": label,
        "description": format!("{label} description"),
        "default": is_default
    })
}

fn question(options: Vec<Value>) -> Value {
    json!({
        "header": "Target",
        "question": "Which environment should I use?",
        "options": options
    })
}

fn request(question: Value) -> Value {
    json!({"questions": [question]})
}

fn parse_stamp(normalized: &Value, field: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(normalized[field].as_str().unwrap())
        .unwrap()
        .with_timezone(&chrono::Utc)
}

/// The shipped card derived the nudge as `expires_at - 60s`, which at the
/// default timeout is the intended four-minute mark. Emitting the deadline
/// server-side must not move it.
#[test]
fn the_default_timeout_keeps_its_four_minute_nudge() {
    let asked_at = chrono::Utc::now();
    let (nudge_at, expires_at) = deadlines_for(asked_at, DEFAULT_ASK_USER_TIMEOUT_SECONDS);

    assert_eq!(expires_at - asked_at, chrono::Duration::seconds(300));
    assert_eq!(expires_at - nudge_at, chrono::Duration::seconds(60));
    assert_eq!(nudge_at - asked_at, chrono::Duration::seconds(240));
}

/// A fixed 60s lead on a short window would put the nudge at or before the
/// moment the question was asked, so the card would open already warning.
#[test]
fn a_short_timeout_scales_the_nudge_instead_of_preceding_the_question() {
    for timeout_seconds in [1, 2, 5, 10, 60, 120, 299] {
        let asked_at = chrono::Utc::now();
        let (nudge_at, expires_at) = deadlines_for(asked_at, timeout_seconds);

        assert!(
            nudge_at >= asked_at,
            "{timeout_seconds}s nudged before the question was asked"
        );
        assert!(
            nudge_at < expires_at,
            "{timeout_seconds}s left no countdown at all"
        );
    }
}

#[test]
fn normalization_stamps_the_deadlines_the_server_will_act_on() {
    let before = chrono::Utc::now();
    let normalized = normalize_ask_user_arguments(&json!({
        "questions": [question(vec![option("Staging", false), option("Prod", false)])],
        "timeout_seconds": 120,
    }))
    .unwrap();
    let after = chrono::Utc::now();

    let asked_at = parse_stamp(&normalized, "asked_at");
    let nudge_at = parse_stamp(&normalized, "nudge_at");
    let expires_at = parse_stamp(&normalized, "expires_at");

    assert!(asked_at >= before && asked_at <= after);
    assert_eq!(expires_at - asked_at, chrono::Duration::seconds(120));
    assert!(nudge_at > asked_at && nudge_at < expires_at);
}

/// The sweep resolves the call at `expires_at`, so a model that sets its
/// own would be choosing when it stops waiting for a human.
#[test]
fn a_model_supplied_deadline_is_overwritten() {
    let normalized = normalize_ask_user_arguments(&json!({
        "questions": [question(vec![option("Staging", false), option("Prod", false)])],
        "timeout_seconds": 60,
        "asked_at": "2000-01-01T00:00:00Z",
        "nudge_at": "2000-01-01T00:00:00Z",
        "expires_at": "2099-01-01T00:00:00Z",
    }))
    .unwrap();

    let asked_at = parse_stamp(&normalized, "asked_at");
    let expires_at = parse_stamp(&normalized, "expires_at");

    assert!(asked_at.year() > 2000, "asked_at kept the model's value");
    assert_eq!(expires_at - asked_at, chrono::Duration::seconds(60));
}

/// A normalized call has to survive a round trip, because that is exactly
/// what every reader of the persisted event does. `deny_unknown_fields`
/// makes this the test that catches a stamped field nobody declared.
#[test]
fn a_normalized_call_deserializes_again() {
    let normalized = normalize_ask_user_arguments(&request(question(vec![
        option("Staging", false),
        option("Prod", false),
    ])))
    .unwrap();

    let parsed: AskUserRequest = serde_json::from_value(normalized).unwrap();
    assert!(parsed.expires_at.is_some());
}

/// The parameters schema is `additionalProperties: false` and validates the
/// *normalized* call, not just what the model wrote. A field stamped by
/// normalization but not declared there gets the whole call rejected before
/// the tool ever runs, which is silent from the model's side.
#[test]
fn every_normalized_field_is_declared_in_the_schema() {
    let normalized = normalize_ask_user_arguments(&request(question(vec![
        option("Staging", false),
        option("Prod", false),
    ])))
    .unwrap();
    let schema = ask_user_parameters_schema();
    let declared = schema["properties"].as_object().unwrap();

    assert_eq!(schema["additionalProperties"], json!(false));
    for field in normalized.as_object().unwrap().keys() {
        assert!(
            declared.contains_key(field),
            "normalization stamps {field:?}, which the schema would reject"
        );
    }
    for field in ["asked_at", "nudge_at", "expires_at"] {
        assert_eq!(
            declared[field]["readOnly"],
            json!(true),
            "{field} is server-owned and must say so"
        );
    }
}

#[test]
fn declared_defaults_prefer_the_declared_option_then_the_first() {
    let normalized = normalize_ask_user_arguments(&json!({
        "questions": [
            question(vec![option("Staging", false), option("Prod", true)]),
            question(vec![option("Alpha", false), option("Beta", false)]),
        ]
    }))
    .unwrap();
    let request: AskUserRequest = serde_json::from_value(normalized).unwrap();

    let answers = declared_defaults(&request.questions);

    assert_eq!(answers.len(), 2);
    assert_eq!(answers[0].selected, vec!["Prod".to_string()]);
    assert_eq!(answers[1].selected, vec!["Alpha".to_string()]);
    // A default answer never carries free text or a credential handle.
    assert!(answers.iter().all(|answer| answer.other_text.is_none()));
    assert!(answers.iter().all(|answer| answer.secret_ref.is_none()));
}

#[test]
fn definition_is_an_undeferrable_read_only_client_tool() {
    let capability = AskUserCapability::client_side();
    assert_eq!(capability.category(), Some("Core"));
    assert!(capability.tools().is_empty());
    let definitions = capability.tool_definitions();
    let [ToolDefinition::ClientSide(definition)] = definitions.as_slice() else {
        panic!("ask_user must be a client-side tool");
    };
    assert_eq!(definition.name, ASK_USER_TOOL_NAME);
    assert_eq!(definition.display_name.as_deref(), Some("Ask User"));
    assert_eq!(definition.category.as_deref(), Some("Core"));
    assert_eq!(definition.deferrable, DeferrablePolicy::Never);
    assert_eq!(definition.hints.readonly, Some(true));
    assert_eq!(definition.hints.destructive, Some(false));
}

#[test]
fn definition_schema_carries_the_contract_limits() {
    let definition = AskUserCapability::client_side()
        .tool_definitions()
        .remove(0);
    let schema = definition.parameters();
    let questions = &schema["properties"]["questions"];
    assert_eq!(questions["minItems"], 1);
    assert_eq!(questions["maxItems"], MAX_ASK_USER_QUESTIONS);
    assert_eq!(
        questions["items"]["properties"]["kind"]["enum"],
        json!(["choice", "text", "secret"])
    );
    assert_eq!(
        questions["items"]["properties"]["header"]["maxLength"],
        MAX_ASK_USER_HEADER_CHARS
    );
    // A `secret` question offers none, so the 2-option floor for a choice
    // is enforced in validation rather than in the shared schema.
    assert_eq!(questions["items"]["properties"]["options"]["minItems"], 0);
    assert_eq!(
        questions["items"]["properties"]["secret_name"]["maxLength"],
        MAX_ASK_USER_SECRET_NAME_CHARS
    );
    assert_eq!(
        questions["items"]["properties"]["options"]["maxItems"],
        MAX_ASK_USER_OPTIONS
    );
    assert_eq!(
        schema["properties"]["timeout_seconds"]["maximum"],
        DEFAULT_ASK_USER_TIMEOUT_SECONDS
    );
}

#[test]
fn validation_accepts_the_confirmed_default_rules() {
    for value in [
        request(question(vec![
            option("Staging", false),
            option("Prod", false),
        ])),
        request(question(vec![
            option("Staging", true),
            option("Prod", false),
        ])),
        request(json!({
            "header": "Targets",
            "question": "Which environments should I use?",
            "multi_select": true,
            "options": [option("Staging", true), option("Prod", true)]
        })),
        request(json!({
            "kind": "text",
            "header": "Branch",
            "question": "What should I call this branch?"
        })),
    ] {
        assert!(normalize_ask_user_arguments(&value).is_ok(), "{value}");
    }
}

#[test]
fn validation_rejects_contract_violations() {
    let valid = question(vec![option("Staging", false), option("Prod", false)]);
    let cases = [
        json!({"questions": []}),
        json!({"questions": [valid.clone(), valid.clone(), valid.clone(), valid.clone(), valid.clone()]}),
        request(
            json!({"header": "12345678901234567", "question": "Too long?", "options": [option("Yes", false), option("No", false)]}),
        ),
        request(question(vec![option("Only", false)])),
        request(question(vec![option("Yes", true), option("No", true)])),
        request(json!({
            "kind": "text",
            "header": "Branch",
            "question": "What should I call this branch?",
            "options": [option("One", false), option("Two", false)]
        })),
        // A secret that offers options, has no name, has no purpose, or
        // shares the call with another question.
        request(
            json!({"kind": "secret", "header": "Token", "question": "What is it?", "secret_name": "TOKEN", "purpose": "Why", "options": [option("One", false), option("Two", false)]}),
        ),
        request(
            json!({"kind": "secret", "header": "Token", "question": "What is it?", "purpose": "Why"}),
        ),
        request(
            json!({"kind": "secret", "header": "Token", "question": "What is it?", "secret_name": "TOKEN"}),
        ),
        request(
            json!({"kind": "secret", "header": "Token", "question": "What is it?", "secret_name": "  ", "purpose": "Why"}),
        ),
        json!({"questions": [
            {"kind": "secret", "header": "Token", "question": "What is it?", "secret_name": "TOKEN", "purpose": "Why"},
            valid.clone()
        ]}),
        // A choice question may not carry the secret fields.
        request(
            json!({"header": "Target", "question": "Which?", "secret_name": "TOKEN", "options": [option("One", false), option("Two", false)]}),
        ),
        json!({"questions": [valid.clone()], "timeout_seconds": 301}),
        json!({"questions": [valid.clone()], "unexpected": true}),
        request(question(vec![option("Same", false), option("Same", false)])),
    ];
    for value in cases {
        assert!(normalize_ask_user_arguments(&value).is_err(), "{value}");
    }
}

#[test]
fn normalization_generates_ids_and_materializes_defaults() {
    let value = json!({
        "human_intent": "Asking where to deploy",
        "questions": [
            {"id": "question_2", "header": "First", "question": "First?", "options": [option("A", false), option("B", false)]},
            {"header": "Second", "question": "Second?", "options": [option("C", false), option("D", false)]}
        ]
    });
    let normalized = normalize_ask_user_arguments(&value).unwrap();
    assert_eq!(normalized["questions"][0]["id"], "question_2");
    assert_eq!(normalized["questions"][1]["id"], "question_2_2");
    assert_eq!(normalized["questions"][1]["kind"], "choice");
    assert_eq!(normalized["questions"][1]["allow_other"], true);
    assert_eq!(
        normalized["timeout_seconds"],
        DEFAULT_ASK_USER_TIMEOUT_SECONDS
    );
    assert_eq!(normalized["human_intent"], "Asking where to deploy");
}

#[test]
fn text_normalization_materializes_its_free_text_shape() {
    let normalized = normalize_ask_user_arguments(&request(json!({
        "kind": "text",
        "header": "Branch",
        "question": "What should I call this branch?",
        "multi_select": true,
        "allow_other": false
    })))
    .unwrap();

    assert_eq!(normalized["questions"][0]["kind"], "text");
    assert_eq!(normalized["questions"][0]["id"], "question_1");
    assert_eq!(normalized["questions"][0]["options"], json!([]));
    assert_eq!(normalized["questions"][0]["multi_select"], false);
    assert_eq!(normalized["questions"][0]["allow_other"], true);
}

#[test]
fn result_contract_round_trips_all_outcomes() {
    for status in [
        AskUserStatus::Answered,
        AskUserStatus::Declined,
        AskUserStatus::Cancelled,
        AskUserStatus::TimedOut,
    ] {
        let result = AskUserResult {
            status,
            answered_by: AskUserAnsweredBy::User,
            answers: vec![AskUserAnswer {
                id: "target".to_string(),
                selected: vec!["Staging".to_string()],
                other_text: None,
                secret_ref: None,
            }],
        };
        let encoded = serde_json::to_value(&result).unwrap();
        assert_eq!(
            serde_json::from_value::<AskUserResult>(encoded).unwrap(),
            result
        );
    }
}

#[test]
fn prompt_and_localization_preserve_the_safety_boundary() {
    let capability = AskUserCapability::client_side();
    let prompt = capability.system_prompt_addition().unwrap();
    assert!(prompt.contains("request_approval"));
    assert!(prompt.contains("Never use `ask_user` as a consent gate"));
    assert!(prompt.contains("Do not re-ask a declined question"));
    assert!(prompt.contains("Ask only when blocked"));
    assert!(prompt.contains("never ask what code or context answers"));
    assert!(prompt.contains("`input_required`"));
    assert!(prompt.contains("`message_task`"));
    assert!(prompt.contains("never answer for them"));
    assert_eq!(
        capability.localized_name(Some("uk-UA")),
        "Запитати користувача"
    );
}

#[tokio::test]
async fn defaults_responder_returns_declared_defaults_without_waiting() {
    let capability = AskUserCapability::default();
    let tools = capability.tools();
    let [tool] = tools.as_slice() else {
        panic!("default ask_user strategy must contribute one tool");
    };

    let ToolExecutionResult::Success(result) = tool
        .execute(json!({
            "questions": [
                {
                    "header": "Target",
                    "question": "Where should I deploy?",
                    "options": [option("Staging", true), option("Production", false)]
                },
                {
                    "header": "Regions",
                    "question": "Which regions?",
                    "multi_select": true,
                    "options": [option("US", true), option("EU", true)]
                },
                {
                    "header": "Format",
                    "question": "Which format?",
                    "options": [option("JSON", false), option("YAML", false)]
                }
            ]
        }))
        .await
    else {
        panic!("default responder must return a successful tool result");
    };
    let outcome: AskUserResult = serde_json::from_value(result).unwrap();

    assert_eq!(outcome.status, AskUserStatus::Answered);
    assert_eq!(outcome.answered_by, AskUserAnsweredBy::Unattended);
    assert_eq!(
        outcome.answers,
        vec![
            AskUserAnswer {
                id: "question_1".to_string(),
                selected: vec!["Staging".to_string()],
                other_text: None,
                secret_ref: None,
            },
            AskUserAnswer {
                id: "question_2".to_string(),
                selected: vec!["US".to_string(), "EU".to_string()],
                other_text: None,
                secret_ref: None,
            },
            AskUserAnswer {
                id: "question_3".to_string(),
                selected: vec!["JSON".to_string()],
                other_text: None,
                secret_ref: None,
            },
        ]
    );
}

/// A responder that overrides `ask_in` learns which session and tool call
/// asked; one that does not keeps its `ask`-only behavior.
#[tokio::test]
async fn execute_with_context_routes_through_ask_in() {
    use std::sync::Mutex;

    #[derive(Default)]
    struct Routing {
        seen: Mutex<Option<AskContext>>,
    }

    #[async_trait]
    impl AskUser for Routing {
        async fn ask(&self, _questions: &[AskUserQuestion]) -> AskUserResult {
            panic!("ask_in must be used when the call has a context");
        }

        async fn ask_in(
            &self,
            context: &AskContext,
            questions: &[AskUserQuestion],
        ) -> AskUserResult {
            *self.seen.lock().unwrap() = Some(context.clone());
            DefaultsResponder.ask(questions).await
        }
    }

    let responder = Arc::new(Routing::default());
    let capability = AskUserCapability::new(responder.clone());
    let tools = capability.tools();
    let [tool] = tools.as_slice() else {
        panic!("in-process ask_user contributes one tool");
    };
    let session_id = SessionId::new();
    let mut context = ToolContext::new(session_id);
    context.tool_call_id = Some("call_ask".to_string());
    let arguments = json!({
        "questions": [{
            "header": "Target",
            "question": "Where should I deploy?",
            "options": [option("Staging", true), option("Production", false)]
        }]
    });

    let ToolExecutionResult::Success(_) = tool.execute_with_context(arguments, &context).await
    else {
        panic!("responder answers");
    };
    let seen = responder.seen.lock().unwrap().clone().expect("ask_in ran");
    assert_eq!(seen.session_id(), session_id);
    assert_eq!(seen.tool_call_id(), "call_ask");
    assert_eq!(seen.turn_id(), None);

    // Default `ask_in` delegates to `ask`, so the unattended responder
    // still answers when the engine supplies a context.
    let defaults = AskUserCapability::default();
    let tools = defaults.tools();
    let [tool] = tools.as_slice() else {
        panic!("default ask_user strategy must contribute one tool");
    };
    let ToolExecutionResult::Success(result) = tool
        .execute_with_context(
            json!({
                "questions": [{
                    "header": "Target",
                    "question": "Where?",
                    "options": [option("Staging", true), option("Production", false)]
                }]
            }),
            &context,
        )
        .await
    else {
        panic!("default responder answers");
    };
    let outcome: AskUserResult = serde_json::from_value(result).unwrap();
    assert_eq!(outcome.answered_by, AskUserAnsweredBy::Unattended);
}

/// EVE-1058: a credential has no default, so nothing answers for the person.
#[tokio::test]
async fn a_secret_question_never_auto_resolves() {
    let capability = AskUserCapability::default();
    let tools = capability.tools();
    let [tool] = tools.as_slice() else {
        panic!("default ask_user strategy must contribute one tool");
    };

    let arguments = json!({
        "questions": [{
            "kind": "secret",
            "header": "Stripe key",
            "question": "Which Stripe restricted key should I use?",
            "secret_name": "STRIPE_API_KEY",
            "purpose": "Read-only charge lookups."
        }]
    });
    let ToolExecutionResult::Success(result) = tool.execute(arguments).await else {
        panic!("the responder must return a successful tool result");
    };
    let outcome: AskUserResult = serde_json::from_value(result).unwrap();

    assert_eq!(outcome.status, AskUserStatus::Declined);
    assert_eq!(outcome.answered_by, AskUserAnsweredBy::Unattended);
    assert!(outcome.answers.is_empty());
}

#[tokio::test]
async fn a_text_question_never_auto_resolves() {
    let capability = AskUserCapability::default();
    let tools = capability.tools();
    let [tool] = tools.as_slice() else {
        panic!("default ask_user strategy must contribute one tool");
    };
    let ToolExecutionResult::Success(result) = tool
        .execute(json!({
            "questions": [{
                "kind": "text",
                "header": "Branch",
                "question": "What should I call this branch?"
            }]
        }))
        .await
    else {
        panic!("the responder must return a successful tool result");
    };
    let outcome: AskUserResult = serde_json::from_value(result).unwrap();

    assert_eq!(outcome.status, AskUserStatus::Declined);
    assert_eq!(outcome.answered_by, AskUserAnsweredBy::Unattended);
    assert!(outcome.answers.is_empty());
}

#[test]
fn a_secret_question_has_no_free_text_or_multi_select_path() {
    let normalized = normalize_ask_user_arguments(&json!({
        "questions": [{
            "kind": "secret",
            "header": "Stripe key",
            "question": "Which key?",
            "multi_select": true,
            "allow_other": true,
            "secret_name": "STRIPE_API_KEY",
            "purpose": "Read-only charge lookups."
        }]
    }))
    .expect("a secret question with the defaults left alone is accepted");

    // Free text is the trap this kind exists to close: it would carry the
    // typed credential into the tool result.
    assert_eq!(normalized["questions"][0]["allow_other"], false);
    assert_eq!(normalized["questions"][0]["multi_select"], false);
    assert_eq!(normalized["questions"][0]["id"], "question_1");
    assert_eq!(normalized["questions"][0]["kind"], "secret");
}

#[test]
fn a_secret_answer_has_no_value_field_to_put_a_credential_in() {
    let encoded = serde_json::to_value(AskUserAnswer {
        id: "stripe_key".to_string(),
        selected: Vec::new(),
        other_text: None,
        secret_ref: Some(session_secret_ref("STRIPE_API_KEY")),
    })
    .unwrap();
    assert_eq!(encoded["secret_ref"], "session:STRIPE_API_KEY");
    assert!(encoded.get("value").is_none());
    // `deny_unknown_fields` means a client cannot add one either.
    assert!(
        serde_json::from_value::<AskUserAnswer>(json!({
            "id": "stripe_key", "value": "rk_live_verysecret"
        }))
        .is_err()
    );
}

#[test]
fn prompt_guidance_points_at_the_secret_kind_instead_of_prose() {
    let capability = AskUserCapability::client_side();
    let prompt = capability.system_prompt_addition().unwrap();
    assert!(prompt.contains(r#"kind: "secret""#), "{prompt}");
    assert!(prompt.contains("never ask for one in prose or an option"));
    assert!(prompt.contains("never the value"));
    assert!(prompt.contains("never auto-resolves"));
}

/// EVE-1057: `DefaultsResponder` and `unattended_ask_user_result` answer an
/// unanswerable question set the same way.
///
/// Two implementations exist because the engine must be able to resolve one
/// without depending on this crate, and the contract types live here. This
/// is what stops them drifting: a model that marked an option `default`
/// would otherwise get that option from one path and the first option from
/// the other, depending only on which surface could not be asked.
#[tokio::test]
async fn defaults_responder_matches_unattended_result() {
    let cases = vec![
        // A declared default, not first in the list.
        serde_json::json!({"questions": [{
            "kind": "choice", "id": "target", "header": "Target",
            "question": "Where?", "multi_select": false, "allow_other": true,
            "options": [
                {"label": "Staging", "description": "Safe."},
                {"label": "Production", "description": "Live.", "default": true}
            ]
        }]}),
        // No default at all: the first option wins.
        serde_json::json!({"questions": [{
            "kind": "choice", "id": "target", "header": "Target",
            "question": "Where?", "multi_select": false, "allow_other": true,
            "options": [
                {"label": "Staging", "description": "Safe."},
                {"label": "Production", "description": "Live."}
            ]
        }]}),
        // Multi-select with several defaults: all of them are taken.
        serde_json::json!({"questions": [{
            "kind": "choice", "id": "checks", "header": "Checks",
            "question": "Which?", "multi_select": true, "allow_other": false,
            "options": [
                {"label": "Lint", "description": "Fast.", "default": true},
                {"label": "Tests", "description": "Slow.", "default": true},
                {"label": "Docs", "description": "Rare."}
            ]
        }]}),
        // A secret question: neither path may invent a credential.
        serde_json::json!({"questions": [{
            "kind": "secret", "id": "stripe_key", "header": "Stripe key",
            "question": "Which Stripe restricted key should I use?",
            "multi_select": false, "allow_other": false, "options": [],
            "secret_name": "STRIPE_API_KEY",
            "purpose": "Read-only charge lookups."
        }]}),
        // Free-form text likewise has no unattended value.
        serde_json::json!({"questions": [{
            "kind": "text", "id": "branch_name", "header": "Branch",
            "question": "What should I call this branch?",
            "multi_select": false, "allow_other": true, "options": []
        }]}),
        // Two questions at once.
        serde_json::json!({"questions": [
            {"kind": "choice", "id": "a", "header": "A", "question": "A?",
             "multi_select": false, "allow_other": true,
             "options": [{"label": "A1", "description": "x", "default": true},
                         {"label": "A2", "description": "y"}]},
            {"kind": "choice", "id": "b", "header": "B", "question": "B?",
             "multi_select": false, "allow_other": true,
             "options": [{"label": "B1", "description": "x"},
                         {"label": "B2", "description": "y"}]}
        ]}),
    ];

    for arguments in cases {
        let request: AskUserRequest =
            serde_json::from_value(arguments.clone()).expect("fixture parses");
        let typed = DefaultsResponder.ask(&request.questions).await;
        let typed_json = serde_json::to_value(&typed).expect("serialises");
        let engine_json = crate::tool_types::unattended_ask_user_result(&arguments);
        assert_eq!(engine_json, typed_json, "diverged on {arguments}");
    }
}
