#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The published integration executes through the ordinary Framework facade.

use everruns::IntoCapability;
use everruns_core::Capability;
use everruns_integrations_typesafe::{Jev, JevCapability, TypeSafeAIClient};
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

const CREDENTIAL: &str = "sentinel-typesafe-credential";

fn joke_call() -> serde_json::Value {
    json!({
        "state": "Why did the chicken cross the road? To get to the other side.",
        "questions": [
            {"id": "is_funny", "type": "noul", "instructions": "Would a general audience laugh?"},
            {"id": "humor", "type": "score", "instructions": "How funny is it?",
             "levels": ["Not funny at all", "Mildly amusing", "Genuinely funny", "Hilarious"]}
        ]
    })
}

#[tokio::test]
async fn an_agent_tool_call_returns_decision_ready_numbers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "jev-1.13.0",
            "answers": {
                "is_funny": {"type": "noul", "noul": 0.43},
                "humor": {
                    "type": "score",
                    "score": 0.38,
                    "legend": {"0": "Not funny at all", "1": "Mildly amusing",
                               "2": "Genuinely funny", "3": "Hilarious"},
                    "probabilities": {"0": 0.63, "1": 0.37, "2": 0.0, "3": 0.0},
                    "confidence": 0.62
                }
            },
            "usage": {"input_tokens": 344, "output_tokens": 36}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let definition = Jev::with_client(
        TypeSafeAIClient::builder(CREDENTIAL)
            .base_url(server.uri())
            .build(),
    )
    .into_capability()
    .into_parts()
    .definition
    .unwrap();

    let output = definition.tools()[0]
        .invoke(
            joke_call(),
            everruns::capability::Context::new("jev_decision", "session", "workspace"),
        )
        .await
        .expect("tool call succeeds");

    assert_eq!(output["answers"]["is_funny"]["probability_yes"], 0.43);
    assert_eq!(output["answers"]["humor"]["level"], 0);
    assert_eq!(output["answers"]["humor"]["label"], "Not funny at all");
    assert_eq!(output["answers"]["humor"]["confidence"], 0.62);
    assert_eq!(output["usage"]["input_tokens"], 344);
}

#[tokio::test]
async fn framework_reports_http_errors_without_upstream_credential_echoes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string(format!("rejected {CREDENTIAL}")))
        .expect(1)
        .mount(&server)
        .await;

    let definition = Jev::with_client(
        TypeSafeAIClient::builder(CREDENTIAL)
            .base_url(server.uri())
            .build(),
    )
    .into_capability()
    .into_parts()
    .definition
    .unwrap();

    let error = definition.tools()[0]
        .invoke(
            joke_call(),
            everruns::capability::Context::new("jev_decision", "session", "workspace"),
        )
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("401"));
    assert!(!format!("{error:?}").contains(CREDENTIAL));
}

#[test]
fn adapters_share_tool_protocol_and_keep_credentials_private() {
    let spec = Jev::new(CREDENTIAL).into_capability();
    assert!(!format!("{spec:?}").contains(CREDENTIAL));
    assert_eq!(spec.capability_ref().id(), "jev");

    let definition = spec.into_parts().definition.unwrap();
    let hosted = JevCapability;
    let tools = hosted.tools();
    let framework = definition.tools()[0].spec();
    assert_eq!(framework.name(), tools[0].name());
    assert_eq!(framework.description(), tools[0].description());
    let mut schema = framework.input_schema().clone();
    schema.as_object_mut().unwrap().remove("$schema");
    schema.as_object_mut().unwrap().remove("title");
    assert_eq!(schema, tools[0].parameters_schema());
}
