use super::*;
use everruns_contracts::{Provider, ProviderAuth, ProviderAuthRequest};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
fn request() -> DecisionRequest {
    DecisionRequest::new("state")
        .model("jev-1.13.0")
        .ask("yes", DecisionQuestion::noul("Is it true?"))
        .ask(
            "kind",
            DecisionQuestion::Choice {
                instructions: "Pick".into(),
                options: vec![("a".into(), None), ("b".into(), None)],
            },
        )
        .ask("level", DecisionQuestion::score("Rate", ["low", "high"]))
}
fn response() -> serde_json::Value {
    json!({"id":"request-1","model":"typesafe/jev-1.13-20260917","provider":"TypeSafe","answers":{
    "yes":{"type":"noul","noul":0.8},
    "kind":{"type":"choice","choice":"a","confidence":0.9,"probabilities":{"a":0.8,"b":0.2}},
    "level":{"type":"score","score":0.8,"confidence":0.9,"legend":{"0":"low","1":"high"},"probabilities":{"0":0.2,"1":0.8}}
},"usage":{"input_tokens":20,"output_tokens":2,"cost":0.001}})
}
#[test]
fn complete_distribution_and_attribution_are_preserved() {
    let outcome = decode(&request(), response()).unwrap();
    assert_eq!(outcome.answers.len(), 3);
    assert_eq!(outcome.attribution.unwrap().cost_usd, Some(0.001));
    assert_eq!(outcome.model, "typesafe/jev-1.13-20260917");
}
#[test]
fn malformed_and_incomplete_answers_fail_closed() {
    for mutated in ["missing", "mass", "range", "unknown", "confidence", "cost"] {
        let mut value = response();
        match mutated {
            "missing" => {
                value["answers"].as_object_mut().unwrap().remove("yes");
            }
            "mass" => value["answers"]["kind"]["probabilities"]["a"] = json!(0.1),
            "range" => value["answers"]["yes"]["noul"] = json!(2),
            "unknown" => value["answers"]["kind"]["choice"] = json!("unknown"),
            "confidence" => {
                value["answers"]["level"]
                    .as_object_mut()
                    .unwrap()
                    .remove("confidence");
            }
            _ => value["usage"]["cost"] = json!(-1),
        }
        assert!(decode(&request(), value).is_err(), "{mutated}");
    }
}
#[tokio::test]
async fn same_account_resolves_refreshable_auth_for_every_service_call() {
    struct Refresh(AtomicUsize);
    #[async_trait]
    impl ProviderAuth for Refresh {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        async fn headers(&self, _: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
            Ok(vec![(
                "authorization".into(),
                format!("Bearer key-{}", self.0.fetch_add(1, Ordering::SeqCst)),
            )])
        }
    }
    let server = MockServer::start().await;
    for i in 0..2 {
        Mock::given(method("POST"))
            .and(path("/systemone"))
            .and(header("authorization", format!("Bearer key-{i}")))
            .and(body_partial_json(json!({"model":"typesafe/jev-1.13"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(response()))
            .expect(1)
            .mount(&server)
            .await;
    }
    let auth = Arc::new(Refresh(AtomicUsize::new(0)));
    let provider = Provider::services("account-1")
        .with_decisions(OpenRouterDecisionDriver::default())
        .base_url(server.uri())
        .auth_arc(auth.clone());
    provider.evaluate_decisions(request()).await.unwrap();
    provider.evaluate_decisions(request()).await.unwrap();
    assert_eq!(auth.0.load(Ordering::SeqCst), 2);
    assert_eq!(openrouter_model("other/jev-1.13.0"), "other/jev-1.13.0");
}

/// OpenRouter's System One rejects a Noul `criteria` object unless it carries
/// both `true` and `false` (probed 2026-10-12); TypeSafe accepts either. A
/// bare question therefore sends no criteria, and a half-described one is
/// completed so the same request works on both endpoints.
#[test]
fn noul_criteria_are_omitted_or_complete_on_the_wire() {
    let bare = encode(
        &DecisionRequest::new("s")
            .model("m")
            .ask("q", DecisionQuestion::noul("True?")),
    )
    .unwrap();
    assert!(bare["questions"]["q"].get("criteria").is_none(), "{bare}");

    let half = DecisionRequest::new("s").model("m").ask(
        "q",
        DecisionQuestion::Noul {
            instructions: "Fails WCAG 1.3.1?".into(),
            yes: Some("It fails 1.3.1".into()),
            no: None,
        },
    );
    let half = encode(&half).unwrap();
    assert_eq!(half["questions"]["q"]["criteria"]["true"], "It fails 1.3.1");
    assert!(
        half["questions"]["q"]["criteria"]["false"].is_string(),
        "{half}"
    );
}

#[tokio::test]
async fn jev_1_14_and_latest_use_exact_account_wire_ids() {
    for (model, wire) in [
        ("jev-1.14.0", "typesafe/jev-1.14"),
        ("typesafe/jev-1.14", "typesafe/jev-1.14"),
        ("jev-latest", "~typesafe/jev-latest"),
        ("typesafe/jev-latest", "~typesafe/jev-latest"),
        ("~typesafe/jev-latest", "~typesafe/jev-latest"),
        ("other/jev-1.14.0", "other/jev-1.14.0"),
        ("typesafe/jev-1.14-20261014", "typesafe/jev-1.14-20261014"),
    ] {
        let server = MockServer::start().await;
        let mut reply = response();
        reply["model"] = json!("typesafe/jev-1.14-20261014");
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(json!({"model": wire})))
            .respond_with(ResponseTemplate::new(200).set_body_json(reply))
            .expect(1)
            .mount(&server)
            .await;
        let provider = Provider::services("router")
            .with_decisions(OpenRouterDecisionDriver::default())
            .base_url(format!("{}/v1", server.uri()))
            .auth(everruns_contracts::BearerAuth::new("test-key"));
        let outcome = provider
            .evaluate_decisions(request().model(model))
            .await
            .unwrap();
        assert_eq!(outcome.model, "typesafe/jev-1.14-20261014");
    }

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(body_partial_json(json!({"model": "jev-1.14.0"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(response()))
        .expect(1)
        .mount(&server)
        .await;
    let provider = Provider::services("direct")
        .with_decisions(SystemOneDecisionDriver::default())
        .base_url(format!("{}/v1", server.uri()))
        .auth(everruns_contracts::BearerAuth::new("test-key"));
    provider
        .evaluate_decisions(request().model("jev-1.14.0"))
        .await
        .unwrap();
    for received in server.received_requests().await.unwrap() {
        assert!(
            received
                .headers
                .keys()
                .all(|name| !name.as_str().starts_with("typesafe-danger-zone"))
        );
    }
}
