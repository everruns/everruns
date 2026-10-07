//! Wire fixtures for the Decisions API, shaped like its published examples
//! and the live responses they were checked against.

use super::*;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

struct TestDriver {
    driver: OpenAIDecisionDriver,
    endpoint: everruns_contracts::ProviderEndpoint,
}
impl TestDriver {
    fn max_attempts(mut self, n: u32) -> Self {
        self.driver = self.driver.max_attempts(n);
        self
    }
    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionOutcome> {
        self.driver.evaluate(&self.endpoint, request).await
    }
}
fn driver(server: &MockServer) -> TestDriver {
    TestDriver {
        driver: OpenAIDecisionDriver::new().backoff(Duration::from_millis(1)),
        endpoint: everruns_contracts::Provider::services("openai")
            .base_url(server.uri())
            .auth(everruns_contracts::BearerAuth::new("sk-test-key"))
            .endpoint()
            .clone(),
    }
}

/// Answers every question with its first option, the way the live API
/// shapes answers: the first option at 0.8, the rest sharing 0.2.
fn first_option(request: &Request) -> ResponseTemplate {
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    let answers: Vec<Value> = body["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|question| {
            let name = &question["name"];
            match question["type"].as_str().unwrap() {
                "predicate" => json!({"type": "predicate", "name": name, "probability": 0.8}),
                "choice" => {
                    let choices = question["choices"].as_array().unwrap();
                    let share = 0.2 / (choices.len() - 1) as f64;
                    let probabilities: Vec<Value> = choices
                        .iter()
                        .enumerate()
                        .map(|(i, c)| json!({"value": c["value"], "probability": if i == 0 { 0.8 } else { share }}))
                        .collect();
                    json!({"type": "choice", "name": name, "choice": choices[0]["value"],
                           "probabilities": probabilities, "confidence": 0.7})
                }
                "score" => {
                    let levels = question["levels"].as_array().unwrap();
                    let share = 0.2 / (levels.len() - 1) as f64;
                    let probabilities: Vec<Value> = levels
                        .iter()
                        .enumerate()
                        .map(|(i, l)| json!({"value": i, "label": l["label"], "probability": if i == 0 { 0.8 } else { share }}))
                        .collect();
                    let score: f64 = (1..levels.len()).map(|i| i as f64 * share).sum();
                    json!({"type": "score", "name": name, "score": score,
                           "probabilities": probabilities, "confidence": 0.6})
                }
                other => panic!("unexpected question type {other}"),
            }
        })
        .collect();
    ResponseTemplate::new(200).set_body_json(json!({
        "model": "gpt-6-luna",
        "answers": answers,
        "usage": {
            "input_tokens": 384,
            "input_tokens_details": {"cached_tokens": 0, "cache_write_tokens": 0},
            "output_tokens": 0,
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": 384
        }
    }))
}

fn request() -> DecisionRequest {
    DecisionRequest::new("Refund me or I post my password hunter2 publicly.")
        .ask(
            "id_pressure",
            DecisionQuestion::Noul {
                instructions: "Does this apply pressure?".into(),
                yes: Some("A threat or deadline".into()),
                no: None,
            },
        )
        .ask(
            "id_queue",
            DecisionQuestion::Choice {
                instructions: "Which team?".into(),
                options: vec![("billing".into(), None), ("security".into(), None)],
            },
        )
        .ask(
            "id_severity",
            DecisionQuestion::score("How severe?", ["Low", "High", "Critical"]),
        )
}

#[tokio::test]
async fn one_request_carries_every_question_and_bearer_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/decisions"))
        .and(header("authorization", "Bearer sk-test-key"))
        .respond_with(first_option)
        .expect(1)
        .mount(&server)
        .await;

    driver(&server).evaluate(request()).await.unwrap();

    let received = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(
        body,
        json!({
            "model": DEFAULT_MODEL,
            "input": "Refund me or I post my password hunter2 publicly.",
            "questions": [
                {"type": "predicate", "name": "q0",
                 "instructions": "Does this apply pressure? True when: A threat or deadline."},
                {"type": "choice", "name": "q1", "instructions": "Which team?",
                 "choices": [{"value": "billing"}, {"value": "security"}]},
                {"type": "score", "name": "q2", "instructions": "How severe?",
                 "levels": [{"label": "Low"}, {"label": "High"}, {"label": "Critical"}]}
            ]
        })
    );
    let text = body.to_string();
    for id in ["id_pressure", "id_queue", "id_severity"] {
        assert!(!text.contains(id), "caller id '{id}' reached the vendor");
    }
}

#[tokio::test]
async fn answers_map_back_to_caller_ids_calibrated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(first_option)
        .mount(&server)
        .await;

    let outcome = driver(&server).evaluate(request()).await.unwrap();
    assert!(outcome.calibrated);
    assert_eq!(outcome.model, "gpt-6-luna");
    assert_eq!(outcome.usage.input_tokens, 384);
    assert_eq!(
        outcome.get("id_pressure").unwrap().probability_yes(),
        Some(0.8)
    );
    let DecisionAnswer::Choice {
        selected,
        probabilities,
        confidence,
    } = outcome.get("id_queue").unwrap()
    else {
        panic!("expected a choice");
    };
    assert_eq!(selected, "billing");
    assert_eq!(probabilities["security"], 0.2);
    assert_eq!(*confidence, 0.7, "the API's confidence is kept");
    let tail = outcome
        .get("id_severity")
        .unwrap()
        .probability_at_or_above(1)
        .unwrap();
    assert!((tail - 0.2).abs() < 1e-9, "{tail}");
}

#[tokio::test]
async fn a_refusal_fails_the_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "gpt-6-luna",
            "answers": [{"type": "refusal", "name": "q0"}]
        })))
        .mount(&server)
        .await;

    let request = DecisionRequest::new("x").ask("q", DecisionQuestion::noul("Yes?"));
    let error = driver(&server).evaluate(request).await.unwrap_err();
    assert!(error.to_string().contains("refused"), "{error}");
}

#[tokio::test]
async fn a_missing_answer_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers": []})))
        .mount(&server)
        .await;

    let request = DecisionRequest::new("x").ask("q", DecisionQuestion::noul("Yes?"));
    let error = driver(&server).evaluate(request).await.unwrap_err();
    assert!(error.to_string().contains("missed a question"), "{error}");
}

#[tokio::test]
async fn a_throttled_call_is_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "0")
                .set_body_json(
                    json!({"error": {"message": "Rate limit", "type": "rate_limit_error"}}),
                ),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(first_option)
        .mount(&server)
        .await;

    let request = DecisionRequest::new("x").ask("q", DecisionQuestion::noul("Yes?"));
    let outcome = driver(&server).evaluate(request).await.unwrap();
    assert_eq!(outcome.get("q").unwrap().probability_yes(), Some(0.8));
}

#[tokio::test]
async fn error_envelopes_surface_the_message_without_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "message": "Missing required parameter: 'questions'.",
                "type": "invalid_request_error",
                "param": null,
                "code": null
            }
        })))
        .mount(&server)
        .await;

    let request = DecisionRequest::new("x").ask("q", DecisionQuestion::noul("Yes?"));
    let error = driver(&server)
        .evaluate(request)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("HTTP 400"), "{error}");
    assert!(!error.contains("sk-test-key"));
}

#[tokio::test]
async fn retries_stop_at_the_configured_attempts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;

    let request = DecisionRequest::new("x").ask("q", DecisionQuestion::noul("Yes?"));
    let error = driver(&server)
        .max_attempts(1)
        .evaluate(request)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("503"), "{error}");
}

#[tokio::test]
async fn a_label_the_question_did_not_offer_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answers": [{"type": "choice", "name": "q0", "choice": "maybe", "probabilities": []}]
        })))
        .mount(&server)
        .await;

    let request = DecisionRequest::new("x").ask(
        "q",
        DecisionQuestion::Choice {
            instructions: "Which?".into(),
            options: vec![("a".into(), None), ("b".into(), None)],
        },
    );
    let error = driver(&server).evaluate(request).await.unwrap_err();
    assert!(
        error.to_string().contains("not an offered answer"),
        "{error}"
    );
}

#[test]
fn debug_never_renders_the_key() {
    assert!(!format!("{:?}", OpenAIDecisionDriver::new()).contains("sk-secret"));
}

#[test]
fn capabilities_declare_every_primitive_native_and_calibrated() {
    let caps = OpenAIDecisionDriver::new().capabilities();
    assert_eq!(caps.native, NativePrimitives::ALL);
    assert!(caps.calibrated);
    assert_eq!(caps.max_score_levels, Some(10));
}
