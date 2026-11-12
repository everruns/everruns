#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Live smoke against Gemini Decision 1 on the Gemini API: one noul, one
//! choice, one score, in one Decisions API call through the Gemini provider.
//!
//! Compiled only with `--features gemini-decisions-live-tests`, and fails
//! closed without `GEMINI_API_KEY`, so a missing secret in CI cannot report
//! green:
//! `doppler run -- cargo test -p everruns-integrations --features gemini-decisions-live-tests --test gemini_decisions_live`.
#![cfg(feature = "gemini-decisions-live-tests")]

use everruns_contracts::runtime::{DecisionAnswer, DecisionQuestion, DecisionRequest};
use everruns_drivers::gemini::provider;

#[tokio::test]
async fn answers_each_primitive_calibrated() {
    let key = std::env::var("GEMINI_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .expect("GEMINI_API_KEY must be set for live API tests (available via Doppler)");
    let outcome = provider("gemini", key)
        .evaluate_decisions(
            DecisionRequest::new("My card was charged twice and I need this fixed today.")
                .model("gemini-decision-1")
                .ask(
                    "is it urgent?",
                    DecisionQuestion::noul("Does this convey urgency?"),
                )
                .ask(
                    "queue",
                    DecisionQuestion::Choice {
                        instructions: "Which team should handle this?".into(),
                        options: vec![
                            (
                                "billing".into(),
                                Some("Payments, invoices, and refunds.".into()),
                            ),
                            ("technical".into(), None),
                            ("sales".into(), None),
                        ],
                    },
                )
                .ask(
                    "severity",
                    DecisionQuestion::score(
                        "How severe is the problem?",
                        ["Minor", "Real", "Serious"],
                    ),
                ),
        )
        .await
        .expect("Gemini Decision 1 answers");
    println!("{outcome:#?}");
    assert_eq!(outcome.model, "gemini-decision-1");
    assert_eq!(outcome.answers.len(), 3);
    assert!(outcome.calibrated);
    assert!(outcome.usage.input_tokens > 0);
    assert!(
        outcome
            .get("is it urgent?")
            .unwrap()
            .probability_yes()
            .unwrap()
            > 0.5
    );
    let DecisionAnswer::Choice { selected, .. } = outcome.get("queue").unwrap() else {
        panic!("expected a choice");
    };
    assert_eq!(selected, "billing");
    assert!(matches!(
        outcome.get("severity").unwrap(),
        DecisionAnswer::Score { .. }
    ));
}
