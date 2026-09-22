#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The deployment decisions over a mock endpoint.
//!
//! `classifier_live.rs` proves the judgments are calibrated; this proves the
//! seam around them — which model a request reaches, how a vendor answer
//! becomes a `DecisionOutcome`, and what an upstream failure says. Those
//! are the parts a live test cannot pin down without spending a real request
//! per case, and the parts that break when either side of the mapping moves.

use everruns_core::{DecisionAnswer, DecisionQuestion, DecisionRequest, DecisionsService};
use everruns_integrations_typesafe::{TypeSafeAI, TypeSafeAIClient};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const CREDENTIAL: &str = "sentinel-decisions-credential";

fn service(server: &MockServer) -> TypeSafeAI {
    TypeSafeAI::with_client(
        TypeSafeAIClient::builder(CREDENTIAL)
            .base_url(server.uri())
            .build(),
    )
}

async fn answering(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn model_of(request: &Request) -> String {
    request
        .body_json::<serde_json::Value>()
        .expect("a JSON body")["model"]
        .as_str()
        .expect("a model")
        .to_string()
}

const ONE_NOUL: fn() -> serde_json::Value = || {
    json!({
        "model": "jev-1.13.0",
        "answers": {"q": {"type": "noul", "noul": 0.5}},
        "usage": {"input_tokens": 1, "output_tokens": 1}
    })
};

fn ask_one() -> DecisionRequest {
    DecisionRequest::new("some content").ask(
        "q",
        DecisionQuestion::Noul {
            instructions: "Is it urgent?".to_string(),
            yes: None,
            no: None,
        },
    )
}

/// Every primitive survives the trip through core's neutral types. A dropped
/// distribution or confidence would leave a guardrail thresholding on a mean.
#[tokio::test]
async fn an_outcome_carries_the_model_usage_and_every_answer_shape() {
    let server = MockServer::start().await;
    answering(
        &server,
        json!({
            "model": "jev-1.13.0",
            "answers": {
                "urgent": {"type": "noul", "noul": 0.81},
                "team": {
                    "type": "choice",
                    "choice": "billing",
                    "probabilities": {"billing": 0.7, "technical": 0.3},
                    "confidence": 0.66
                },
                "anger": {
                    "type": "score",
                    "score": 1.4,
                    "legend": {"0": "Calm", "1": "Annoyed", "2": "Furious"},
                    "probabilities": {"0": 0.1, "1": 0.4, "2": 0.5},
                    "confidence": 0.72
                }
            },
            "usage": {"input_tokens": 311, "output_tokens": 27}
        }),
    )
    .await;

    let outcome = service(&server)
        .evaluate(
            DecisionRequest::new("a ticket")
                .ask(
                    "urgent",
                    DecisionQuestion::Noul {
                        instructions: "Is it urgent?".to_string(),
                        yes: None,
                        no: None,
                    },
                )
                .ask(
                    "team",
                    DecisionQuestion::Choice {
                        instructions: "Who handles this?".to_string(),
                        options: vec![
                            ("billing".to_string(), None),
                            ("technical".to_string(), None),
                        ],
                    },
                )
                .ask(
                    "anger",
                    DecisionQuestion::score("How angry?", ["Calm", "Annoyed", "Furious"]),
                ),
        )
        .await
        .expect("judgment succeeds");

    // The model that answered, not the one that was asked for.
    assert_eq!(outcome.model, "jev-1.13.0");
    assert_eq!(outcome.usage.input_tokens, 311);
    assert_eq!(outcome.usage.output_tokens, 27);

    assert_eq!(outcome.get("urgent").unwrap().probability_yes(), Some(0.81));

    let DecisionAnswer::Choice {
        selected,
        probabilities,
        confidence,
    } = outcome.get("team").expect("a choice answer")
    else {
        panic!("expected a choice answer");
    };
    assert_eq!(selected, "billing");
    assert_eq!(probabilities["technical"], 0.3);
    assert_eq!(*confidence, 0.66);

    let anger = outcome.get("anger").expect("a score answer");
    let tail = anger.probability_at_or_above(2).expect("a score answer");
    assert!((tail - 0.5).abs() < 1e-9, "{tail}");
    assert_eq!(anger.confidence(), Some(0.72));
}

/// Three ways a model can be chosen, in the order that matters. The platform
/// pins its decisions by naming none, so the default must hold; an embedder
/// naming one on the service must get it; and a request naming one must win
/// over both, because that is the only knob a caller has.
#[tokio::test]
async fn the_request_model_outranks_the_service_model_which_outranks_the_default() {
    let server = MockServer::start().await;
    answering(&server, ONE_NOUL()).await;

    service(&server).evaluate(ask_one()).await.expect("default");
    service(&server)
        .model("jev-pinned")
        .evaluate(ask_one())
        .await
        .expect("service model");
    service(&server)
        .model("jev-pinned")
        .evaluate(ask_one().model("jev-per-request"))
        .await
        .expect("request model");

    let asked: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(model_of)
        .collect();
    assert_eq!(asked, ["jev-latest", "jev-pinned", "jev-per-request"]);
}

/// Guardrails fail open on an error, so the message is what an operator has to
/// debug from — and it must not put the deployment key in their logs.
#[tokio::test]
async fn an_upstream_failure_is_reported_without_echoing_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string(format!("rejected {CREDENTIAL}")))
        .mount(&server)
        .await;

    let error = service(&server).evaluate(ask_one()).await.unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("decision request failed"), "{rendered}");
    assert!(rendered.contains("401"), "{rendered}");
    assert!(!rendered.contains(CREDENTIAL), "{rendered}");
}

/// The deployment client does not retry: guardrails sit in front of tool calls,
/// where a second round trip costs the user more than a fail-open costs policy.
#[tokio::test]
async fn the_deployment_service_spends_exactly_one_request_on_a_rate_limit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1)
        .mount(&server)
        .await;

    let service = TypeSafeAI::with_client(
        TypeSafeAIClient::builder(CREDENTIAL)
            .base_url(server.uri())
            .retry(everruns_integrations_typesafe::RetryPolicy::none())
            .build(),
    );
    service.evaluate(ask_one()).await.unwrap_err();
}
