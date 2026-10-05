//! Live smoke against the real Decisions API: one choice, one noul, one score.
//!
//! Compiled only with `--features live-tests`, and returns early without
//! `OPENAI_API_KEY`. Needs an account with Decisions API access:
//! `doppler run -- cargo test -p everruns-integrations-openai-decisions --features live-tests`.
#![cfg(feature = "live-tests")]
#![allow(clippy::expect_used)]

use everruns_contracts::runtime::{DecisionQuestion, DecisionRequest, DecisionsService};
use everruns_integrations_openai_decisions::OpenAIDecisions;

#[tokio::test]
async fn answers_each_primitive() {
    let Ok(key) = std::env::var("OPENAI_API_KEY") else {
        eprintln!("OPENAI_API_KEY unset; skipping");
        return;
    };
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
                            ("billing".into(), None),
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
}
