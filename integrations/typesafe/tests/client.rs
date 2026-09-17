//! Client behavior against a mock TypeSafe endpoint: wire shape, retries, and
//! the credential-safety contract on error paths.

use std::time::Duration;

use everruns_integrations_typesafe::{
    Error, Evaluation, Question, RetryPolicy, TypeSafeClient, client::DEFAULT_BASE_URL,
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

const CREDENTIAL: &str = "sentinel-typesafe-credential";

fn client(server: &MockServer) -> TypeSafeClient {
    TypeSafeClient::builder(CREDENTIAL)
        .base_url(server.uri())
        .retry(RetryPolicy {
            max_attempts: 3,
            backoff: Duration::from_millis(1),
        })
        .build()
}

fn joke_evaluation() -> Evaluation {
    Evaluation::new("Why did the chicken cross the road? To get to the other side.").ask(
        "is_funny",
        Question::noul("Would a general audience laugh?"),
    )
}

#[tokio::test]
async fn posts_the_documented_wire_shape_and_parses_every_primitive() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("authorization", format!("Bearer {CREDENTIAL}")))
        .and(body_json(json!({
            "state": "a ticket",
            "model": "jev-latest",
            "questions": {
                "funny": {"type": "noul", "instructions": "Is it funny?"},
                "team": {
                    "type": "choice",
                    "instructions": "Who handles this?",
                    "criteria": {"billing": "Money", "technical": null}
                },
                "anger": {
                    "type": "score",
                    "instructions": "How angry?",
                    "criteria": ["Calm", "Annoyed", "Furious"]
                }
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "jev-1.13.0",
            "answers": {
                "funny": {"type": "noul", "noul": 0.43},
                "team": {
                    "type": "choice",
                    "choice": "technical",
                    "probabilities": {"billing": 0.1, "technical": 0.9},
                    "confidence": 0.82
                },
                "anger": {
                    "type": "score",
                    "score": 1.6,
                    "legend": {"0": "Calm", "1": "Annoyed", "2": "Furious"},
                    "probabilities": {"0": 0.05, "1": 0.3, "2": 0.65},
                    "confidence": 0.78
                }
            },
            "usage": {"input_tokens": 312, "output_tokens": 48}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let judgment = client(&server)
        .evaluate(
            Evaluation::new("a ticket")
                .ask("funny", Question::noul("Is it funny?"))
                .ask(
                    "team",
                    Question::choice(
                        "Who handles this?",
                        [("billing", json!("Money")), ("technical", json!(null))],
                    ),
                )
                .ask(
                    "anger",
                    Question::score("How angry?", ["Calm", "Annoyed", "Furious"]),
                ),
        )
        .await
        .expect("evaluation succeeds");

    assert_eq!(judgment.model, "jev-1.13.0");
    assert_eq!(judgment.noul("funny").unwrap(), 0.43);

    let team = judgment.choice("team").unwrap();
    assert_eq!(team.choice, "technical");
    assert_eq!(team.probability_of("billing"), 0.1);
    assert_eq!(team.probability_of("absent"), 0.0);
    assert_eq!(team.confidence, 0.82);

    let anger = judgment.score("anger").unwrap();
    assert_eq!(anger.nearest_level(), 2);
    assert_eq!(anger.nearest_label(), Some("Furious"));
    assert_eq!(anger.normalized(), 0.8);
    assert!((anger.probability_at_or_above(1) - 0.95).abs() < 1e-9);
    assert_eq!(judgment.usage.input_tokens, 312);
}

#[tokio::test]
async fn reading_an_answer_as_the_wrong_primitive_is_an_error_not_a_default() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "jev-1.13.0",
            "answers": {"is_funny": {"type": "noul", "noul": 0.9}},
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })))
        .mount(&server)
        .await;

    let judgment = client(&server)
        .evaluate(joke_evaluation())
        .await
        .expect("evaluation succeeds");

    let error = judgment.score("is_funny").expect_err("wrong primitive");
    assert!(matches!(
        error,
        Error::AnswerType {
            expected: "score",
            actual: "noul",
            ..
        }
    ));
    assert!(matches!(
        judgment.noul("never_asked").expect_err("missing"),
        Error::UnknownAnswer(_)
    ));
}

#[tokio::test]
async fn rate_limits_are_retried_and_then_succeed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "jev-1.13.0",
            "answers": {"is_funny": {"type": "noul", "noul": 0.7}},
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let judgment = client(&server)
        .evaluate(joke_evaluation())
        .await
        .expect("retry succeeds");
    assert_eq!(judgment.noul("is_funny").unwrap(), 0.7);
}

#[tokio::test]
async fn authentication_failures_are_not_retried_and_never_echo_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_string(format!("rejected Authorization: Bearer {CREDENTIAL}")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let error = client(&server)
        .evaluate(joke_evaluation())
        .await
        .expect_err("401 fails");
    assert_eq!(error.status(), Some(401));
    assert!(!error.is_retryable());
    assert!(
        !format!("{error}").contains(CREDENTIAL),
        "error must not echo the credential: {error}"
    );
}

#[tokio::test]
async fn validation_failures_surface_the_field_the_api_named() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "message": "questions.anger.criteria must contain at least 2 items"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let error = client(&server)
        .evaluate(joke_evaluation())
        .await
        .expect_err("422 fails");
    assert!(
        format!("{error}").contains("criteria must contain at least 2 items"),
        "{error}"
    );
}

#[tokio::test]
async fn an_unparseable_body_is_a_decode_error_not_a_silent_default() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>gateway</html>"))
        .mount(&server)
        .await;

    let error = client(&server)
        .evaluate(joke_evaluation())
        .await
        .expect_err("garbage body fails");
    assert!(matches!(error, Error::Decode(_)), "{error}");
}

#[tokio::test]
async fn malformed_requests_are_rejected_before_any_round_trip() {
    let server = MockServer::start().await;
    // No mock is mounted: any outbound request would fail the test.
    let client = client(&server);

    let error = client
        .evaluate(Evaluation::new("state"))
        .await
        .expect_err("no questions");
    assert!(matches!(error, Error::InvalidRequest(_)), "{error}");

    let error = client
        .evaluate(Evaluation::new("state").ask("blank", Question::noul("   ")))
        .await
        .expect_err("blank instructions");
    assert!(format!("{error}").contains("empty instructions"), "{error}");

    let error = client
        .evaluate(Evaluation::new("state").ask("thin", Question::score("How bad?", ["Only one"])))
        .await
        .expect_err("one level");
    assert!(
        format!("{error}").contains("at least two levels"),
        "{error}"
    );

    assert_eq!(server.received_requests().await.unwrap().len(), 0);
}

#[tokio::test]
async fn retries_stop_at_the_configured_attempt_count() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(529))
        .expect(3)
        .mount(&server)
        .await;

    let error = client(&server)
        .evaluate(joke_evaluation())
        .await
        .expect_err("stays overloaded");
    assert_eq!(error.status(), Some(529));
}

#[tokio::test]
async fn retries_can_be_disabled() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(529))
        .expect(1)
        .mount(&server)
        .await;

    let error = TypeSafeClient::builder(CREDENTIAL)
        .base_url(server.uri())
        .retry(RetryPolicy::none())
        .build()
        .evaluate(joke_evaluation())
        .await
        .expect_err("stays overloaded");
    assert_eq!(error.status(), Some(529));
}

#[test]
fn the_default_endpoint_is_the_documented_one() {
    assert_eq!(DEFAULT_BASE_URL, "https://api.typesafe.ai");
}

#[tokio::test]
async fn boolean_and_probability_alias_the_wire_names() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_json(json!({
            "state": "Charged twice. Please refund the duplicate.",
            "model": "jev-latest",
            "questions": {"refund": {"type": "noul", "instructions": "Is a refund requested?"}}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "jev-1.13.0",
            "answers": {"refund": {"type": "noul", "noul": 0.97}},
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let judgment = client(&server)
        .evaluate(
            Evaluation::new("Charged twice. Please refund the duplicate.")
                .ask("refund", Question::boolean("Is a refund requested?")),
        )
        .await
        .expect("evaluation succeeds");

    assert_eq!(judgment.probability("refund").unwrap(), 0.97);
    assert_eq!(
        judgment.probability("refund").unwrap(),
        judgment.noul("refund").unwrap()
    );
}
