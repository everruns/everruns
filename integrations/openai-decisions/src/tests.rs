//! Wire fixtures for the provisional Decisions API shape.

use super::*;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn driver(server: &MockServer) -> OpenAIDecisions {
    OpenAIDecisions::new("sk-test-key")
        .base_url(server.uri())
        .backoff(Duration::from_millis(1))
}

/// Answers each question from its options: the first label, with a
/// distribution only when `with_probabilities`.
fn first_option(with_probabilities: bool) -> impl Fn(&Request) -> ResponseTemplate {
    move |request: &Request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let labels: Vec<String> = body["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["label"].as_str().unwrap().to_string())
            .collect();
        let mut reply = json!({
            "id": "dec_1",
            "object": "decision",
            "model": "gpt-6-luna-2026-09-29",
            "label": labels[0],
            "confidence": 0.8,
            "usage": {"input_tokens": 40, "output_tokens": 1}
        });
        if with_probabilities {
            let share = 0.2 / (labels.len() - 1) as f64;
            let dist: serde_json::Map<String, Value> = labels
                .iter()
                .enumerate()
                .map(|(i, l)| (l.clone(), json!(if i == 0 { 0.8 } else { share })))
                .collect();
            reply["probabilities"] = Value::Object(dist);
        }
        ResponseTemplate::new(200).set_body_json(reply)
    }
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
async fn requests_carry_the_mapped_question_and_bearer_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/decisions"))
        .and(header("authorization", "Bearer sk-test-key"))
        .respond_with(first_option(false))
        .expect(3)
        .mount(&server)
        .await;

    driver(&server).evaluate(request()).await.unwrap();

    let bodies: Vec<Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    let noul = bodies
        .iter()
        .find(|b| b["instructions"] == "Does this apply pressure?")
        .expect("noul sent");
    assert_eq!(noul["model"], DEFAULT_MODEL);
    assert_eq!(
        noul["options"],
        json!([
            {"label": "yes", "description": "A threat or deadline"},
            {"label": "no"}
        ])
    );
    let score = bodies
        .iter()
        .find(|b| b["options"].as_array().unwrap().len() == 3)
        .expect("score sent");
    assert_eq!(
        score["options"][2],
        json!({"label": "2", "description": "Critical"})
    );
    for body in &bodies {
        let text = body.to_string();
        for id in ["id_pressure", "id_queue", "id_severity"] {
            assert!(!text.contains(id), "caller id '{id}' reached the vendor");
        }
    }
}

#[tokio::test]
async fn labels_without_a_distribution_are_one_hot_and_uncalibrated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(first_option(false))
        .mount(&server)
        .await;

    let outcome = driver(&server).evaluate(request()).await.unwrap();
    assert!(
        !outcome.calibrated,
        "a lone confidence is not a distribution"
    );
    assert_eq!(outcome.model, "gpt-6-luna-2026-09-29");
    assert_eq!(
        outcome.usage.input_tokens, 120,
        "usage sums across questions"
    );
    assert_eq!(
        outcome.get("id_pressure").unwrap().probability_yes(),
        Some(1.0)
    );
    let DecisionAnswer::Choice {
        selected,
        probabilities,
        ..
    } = outcome.get("id_queue").unwrap()
    else {
        panic!("expected a choice");
    };
    assert_eq!(selected, "billing");
    assert_eq!(probabilities["security"], 0.0);
    assert_eq!(
        outcome
            .get("id_severity")
            .unwrap()
            .probability_at_or_above(1),
        Some(0.0)
    );
}

#[tokio::test]
async fn a_full_distribution_is_kept_and_reported_calibrated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(first_option(true))
        .mount(&server)
        .await;

    let outcome = driver(&server).evaluate(request()).await.unwrap();
    assert!(outcome.calibrated);
    assert_eq!(
        outcome.get("id_pressure").unwrap().probability_yes(),
        Some(0.8)
    );
    let tail = outcome
        .get("id_severity")
        .unwrap()
        .probability_at_or_above(1)
        .unwrap();
    assert!((tail - 0.2).abs() < 1e-9, "{tail}");
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
        .respond_with(first_option(false))
        .mount(&server)
        .await;

    let request = DecisionRequest::new("x").ask("q", DecisionQuestion::noul("Yes?"));
    let outcome = driver(&server).evaluate(request).await.unwrap();
    assert_eq!(outcome.get("q").unwrap().probability_yes(), Some(1.0));
}

#[tokio::test]
async fn error_envelopes_surface_the_message_without_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "message": "Decision API is not enabled for this user.",
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
    assert!(
        error.contains("400 invalid_request_error): Decision API is not enabled"),
        "{error}"
    );
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
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"decision": "maybe"})))
        .mount(&server)
        .await;

    let request = DecisionRequest::new("x").ask("q", DecisionQuestion::noul("Yes?"));
    let error = driver(&server).evaluate(request).await.unwrap_err();
    assert!(
        error.to_string().contains("not an offered answer"),
        "{error}"
    );
}

#[test]
fn debug_never_renders_the_key() {
    assert!(!format!("{:?}", OpenAIDecisions::new("sk-secret")).contains("sk-secret"));
}

#[test]
fn wrapped_responses_parse_too() {
    let parsed = parse_response(&json!({
        "model": "gpt-6-luna",
        "output": {"answer": "no", "confidence": 0.9},
        "usage": {"prompt_tokens": 7, "completion_tokens": 1}
    }))
    .unwrap();
    assert_eq!(parsed.label, "no");
    assert_eq!(parsed.model.as_deref(), Some("gpt-6-luna"));
    assert_eq!(parsed.usage.input_tokens, 7);
}
