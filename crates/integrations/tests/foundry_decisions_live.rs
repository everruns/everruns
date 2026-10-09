#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Live smoke against Microsoft-Decision-1 on Microsoft Foundry: one noul, one
//! choice, one score, in one System One call through the Foundry provider.
//!
//! Compiled only with `--features foundry-decisions-live-tests`, and fails
//! closed without `AZURE_FOUNDRY_ENDPOINT` and `AZURE_FOUNDRY_API_KEY`, so a
//! missing secret in CI cannot report green. `FOUNDRY_DECISION_DEPLOYMENT`
//! names the deployment (default `Decision-1`, the dev project's):
//! `doppler run -- cargo test -p everruns-integrations --features foundry-decisions-live-tests --test foundry_decisions_live`.
#![cfg(feature = "foundry-decisions-live-tests")]

use everruns_contracts::runtime::{DecisionAnswer, DecisionQuestion, DecisionRequest};
use everruns_drivers::mai::{MaiAuth, provider};

fn env(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| panic!("{name} must be set for live API tests (available via Doppler)"))
}

#[tokio::test]
async fn answers_each_primitive_calibrated() {
    let deployment =
        std::env::var("FOUNDRY_DECISION_DEPLOYMENT").unwrap_or_else(|_| "Decision-1".into());
    let foundry = provider(
        "foundry",
        env("AZURE_FOUNDRY_ENDPOINT"),
        MaiAuth::ApiKey(env("AZURE_FOUNDRY_API_KEY")),
    );
    let outcome = foundry
        .evaluate_decisions(
            DecisionRequest::new("My card was charged twice and I need this fixed today.")
                .model(deployment)
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
        .expect("Microsoft-Decision-1 answers");
    println!("{outcome:#?}");
    assert_eq!(outcome.model, "microsoft-decision-1");
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
