#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Live smoke against the real Decisions API: one predicate, one choice, one
//! score, in one call.
//!
//! Compiled only with `--features openai-decisions-live-tests`, and fails
//! closed without `OPENAI_API_KEY`, so a missing secret in CI cannot report
//! green: `doppler run -- cargo test -p everruns-integrations --features openai-decisions-live-tests`.
#![cfg(feature = "openai-decisions-live-tests")]

use everruns_contracts::runtime::{
    DecisionAnswer, DecisionQuestion, DecisionRequest, DecisionsService,
};
use everruns_integrations::openai_decisions::OpenAIDecisions;

#[tokio::test]
async fn answers_each_primitive_calibrated() {
    let key = std::env::var("OPENAI_API_KEY")
        .expect("OPENAI_API_KEY must be set for live API tests (available via Doppler)");
    let outcome = OpenAIDecisions::new(key)
        .evaluate(
            DecisionRequest::new("My card was charged twice and I need this fixed today.")
                .ask(
                    "urgent",
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
        .expect("the Decisions API answers");
    println!("{outcome:#?}");
    assert_eq!(outcome.answers.len(), 3);
    assert!(outcome.calibrated);
    assert!(outcome.usage.input_tokens > 0);
    assert!(outcome.get("urgent").unwrap().probability_yes().unwrap() > 0.5);
    let DecisionAnswer::Choice { selected, .. } = outcome.get("queue").unwrap() else {
        panic!("expected a choice");
    };
    assert_eq!(selected, "billing");
    assert!(matches!(
        outcome.get("severity").unwrap(),
        DecisionAnswer::Score { .. }
    ));
}
