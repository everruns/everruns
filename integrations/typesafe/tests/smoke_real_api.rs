//! Real API smoke tests for TypeSafe.
//!
//! Gated behind the `integration` feature — only compiled when run with:
//!   cargo test -p everruns-integrations-typesafe --features integration
//!
//! The key comes from Doppler as TYPESAFE_API_KEY. Tests panic when it is
//! missing, so a dropped credential never looks like a pass.

#![cfg(feature = "integration")]

use everruns_integrations_typesafe::{Evaluation, Question, TypeSafeClient};

macro_rules! require_api_key {
    () => {
        match std::env::var("TYPESAFE_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => panic!("TYPESAFE_API_KEY not set — cannot run integration tests"),
        }
    };
}

fn humor_questions(evaluation: Evaluation) -> Evaluation {
    evaluation
        .ask(
            "is_joke",
            Question::noul("Is this text told as a joke, rather than a plain statement?"),
        )
        .ask(
            "humor",
            Question::score(
                "How funny would a general adult audience find this?",
                [
                    "Not funny at all; no attempt at humor, or the attempt fails completely",
                    "Mildly amusing; raises a smile but not a laugh",
                    "Genuinely funny; most people would laugh",
                    "Hilarious; consistently lands with almost everyone",
                ],
            ),
        )
}

/// The generic verification case: the agent asks how funny something is and
/// gets a calibrated answer back, not a second opinion in prose.
#[tokio::test]
async fn rates_a_joke_and_separates_it_from_a_flat_statement() {
    let client = TypeSafeClient::new(require_api_key!());

    let joke = client
        .evaluate(humor_questions(Evaluation::new(
            "I told my wife she was drawing her eyebrows too high. She looked surprised.",
        )))
        .await
        .expect("joke evaluation succeeds");
    let statement = client
        .evaluate(humor_questions(Evaluation::new(
            "The quarterly maintenance window begins at 02:00 UTC on Saturday.",
        )))
        .await
        .expect("statement evaluation succeeds");

    let joke_humor = joke.score("humor").expect("score answer");
    let statement_humor = statement.score("humor").expect("score answer");

    assert!(
        joke.noul("is_joke").unwrap() > statement.noul("is_joke").unwrap(),
        "a joke must read as more joke-like than a maintenance notice"
    );
    assert!(
        joke_humor.score > statement_humor.score,
        "a working joke must outscore a maintenance notice: {} vs {}",
        joke_humor.score,
        statement_humor.score
    );
    assert!(statement_humor.nearest_level() == 0);
    assert!((0.0..=1.0).contains(&joke_humor.normalized()));
    assert!(joke.usage.input_tokens > 0);
}

/// Every primitive comes back in the documented shape from one request.
#[tokio::test]
async fn answers_all_three_primitives_in_a_single_request() {
    let client = TypeSafeClient::new(require_api_key!());

    let judgment = client
        .evaluate(
            Evaluation::new("My payouts have been failing for three days and nobody has replied.")
                .ask("is_urgent", Question::noul("Does this convey urgency?"))
                .ask(
                    "team",
                    Question::choice(
                        "Which team should handle this?",
                        [
                            ("billing", "Payments, invoicing, payouts, refunds"),
                            ("technical", "Bugs, outages, integrations"),
                            ("sales", "Pricing, upgrades, new accounts"),
                        ],
                    ),
                )
                .ask(
                    "frustration",
                    Question::score(
                        "How frustrated is the customer?",
                        ["Calm", "Frustrated", "Very angry"],
                    ),
                ),
        )
        .await
        .expect("evaluation succeeds");

    assert!(judgment.noul("is_urgent").unwrap() > 0.5);
    let team = judgment.choice("team").expect("choice answer");
    assert_eq!(team.choice, "billing");
    assert!(team.probabilities.len() == 3);
    assert!((team.probabilities.values().sum::<f64>() - 1.0).abs() < 0.05);
    let frustration = judgment.score("frustration").expect("score answer");
    assert!(frustration.score > 0.5, "{}", frustration.score);
}

/// A bad key fails loudly, is not retried, and never echoes the credential.
#[tokio::test]
async fn an_invalid_key_is_rejected_without_echoing_it() {
    let client = TypeSafeClient::new("ts-not-a-real-key-sentinel");
    let error = client
        .evaluate(Evaluation::new("x").ask("q", Question::noul("Is this English?")))
        .await
        .expect_err("an invalid key must fail");
    assert!(!format!("{error}").contains("sentinel"), "{error}");
}
