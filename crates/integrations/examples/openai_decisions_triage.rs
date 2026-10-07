//! Route a support ticket with OpenAI's Decisions API: one request, three
//! judgments, code decides.
//!
//! ```sh
//! OPENAI_API_KEY=... cargo run -p everruns-integrations --features openai-decisions --example openai_decisions_triage
//! ```
//!
//! The same questions as `typesafe_triage`, asked through the vendor-neutral
//! `DecisionsService`, so swapping the vendor changes one line.

use everruns_contracts::runtime::{
    DecisionAnswer, DecisionQuestion, DecisionRequest, DecisionsService,
};
use everruns_integrations::openai_decisions::OpenAIDecisions;

/// Below this, the distribution is too flat to auto-route.
const AUTO_ROUTE_CONFIDENCE: f64 = 0.7;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = OpenAIDecisions::new(std::env::var("OPENAI_API_KEY")?);

    // State can be structured: give the model the record, not a paraphrase of it.
    let ticket = serde_json::json!({
        "customer": {"plan": "enterprise", "tenure_months": 41},
        "messages": [
            {"from": "customer", "text": "Payouts have failed for three days. No reply yet."},
            {"from": "agent", "text": "Thanks for reaching out, could you share an example id?"},
            {"from": "customer", "text": "I sent three already. This is costing us real money."}
        ]
    });

    let outcome = service
        .evaluate(
            DecisionRequest::new(ticket)
                .ask(
                    "needs_human",
                    DecisionQuestion::noul(
                        "Does `messages` show the customer asking for something an automated \
                         reply cannot resolve?",
                    ),
                )
                .ask(
                    "team",
                    DecisionQuestion::Choice {
                        instructions: "Which team should own this ticket next?".into(),
                        options: vec![
                            (
                                "billing".into(),
                                Some("Payments, invoicing, payouts, refunds".into()),
                            ),
                            (
                                "technical".into(),
                                Some("Bugs, outages, API and integration failures".into()),
                            ),
                            (
                                "success".into(),
                                Some("Relationship, escalations, account reviews".into()),
                            ),
                        ],
                    },
                )
                .ask(
                    "frustration",
                    DecisionQuestion::score(
                        "How frustrated is the customer in their most recent message?",
                        [
                            "Calm; neutral or friendly",
                            "Frustrated; clearly unhappy but still cooperative",
                            "Very angry; threatening escalation or churn",
                        ],
                    ),
                ),
        )
        .await?;

    let needs_human = outcome
        .get("needs_human")
        .and_then(DecisionAnswer::probability_yes)
        .ok_or("no needs_human answer")?;
    let Some(DecisionAnswer::Choice {
        selected: team,
        confidence,
        probabilities,
    }) = outcome.get("team")
    else {
        return Err("no team answer".into());
    };
    let frustration = outcome.get("frustration").ok_or("no frustration answer")?;
    let DecisionAnswer::Score { score, .. } = frustration else {
        return Err("frustration is not a score".into());
    };

    println!(
        "model           {} (calibrated: {})",
        outcome.model, outcome.calibrated
    );
    println!("needs a human   {needs_human:.2}");
    println!("team            {team} (confidence {confidence:.2}) {probabilities:?}");
    println!("frustration     {score:.2} of 2");
    println!("input tokens    {}", outcome.usage.input_tokens);

    // "Any serious signal" is a separate condition, not a weighted average:
    // a bimodal frustration answer must not be averaged into calm.
    let escalate = frustration.probability_at_or_above(2).unwrap_or(0.0) > 0.4;
    match (*confidence >= AUTO_ROUTE_CONFIDENCE, escalate) {
        (_, true) => println!("\n-> escalate to success, notify the account owner"),
        (true, false) => println!("\n-> auto-route to {team}"),
        (false, false) => println!("\n-> queue for manual triage (distribution too flat)"),
    }
    Ok(())
}
