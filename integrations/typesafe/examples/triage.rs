//! Route a support ticket: one request, three judgments, code decides.
//!
//! ```sh
//! TYPESAFE_API_KEY=... cargo run -p everruns-integrations-typesafe --example triage
//! ```
//!
//! Confidence is the second axis. The answer says *what*; confidence says
//! whether to act on it without a person in the loop.

use everruns_integrations_typesafe::{Evaluation, Question, TypeSafeAIClient};

/// Below this, the distribution is too flat to auto-route.
const AUTO_ROUTE_CONFIDENCE: f64 = 0.7;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = TypeSafeAIClient::from_env()?;

    // State can be structured: give the model the record, not a paraphrase of it.
    let ticket = serde_json::json!({
        "customer": {"plan": "enterprise", "tenure_months": 41},
        "messages": [
            {"from": "customer", "text": "Payouts have failed for three days. No reply yet."},
            {"from": "agent", "text": "Thanks for reaching out, could you share an example id?"},
            {"from": "customer", "text": "I sent three already. This is costing us real money."}
        ]
    });

    let judgment = client
        .evaluate(
            Evaluation::new(ticket)
                .ask(
                    "needs_human",
                    Question::noul(
                        "Does `messages` show the customer asking for something an automated \
                         reply cannot resolve?",
                    ),
                )
                .ask(
                    "team",
                    Question::choice(
                        "Which team should own this ticket next?",
                        [
                            ("billing", "Payments, invoicing, payouts, refunds"),
                            ("technical", "Bugs, outages, API and integration failures"),
                            ("success", "Relationship, escalations, account reviews"),
                        ],
                    ),
                )
                .ask(
                    "frustration",
                    Question::score(
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

    let team = judgment.choice("team")?;
    let frustration = judgment.score("frustration")?;

    println!("needs a human   {:.2}", judgment.noul("needs_human")?);
    println!(
        "team            {} (confidence {:.2}) {:?}",
        team.choice, team.confidence, team.probabilities
    );
    println!(
        "frustration     {:.2} — {}",
        frustration.score,
        frustration.nearest_label().unwrap_or("?")
    );

    // "Any serious signal" is a separate condition, not a weighted average:
    // a bimodal frustration answer must not be averaged into calm.
    let escalate = frustration.probability_at_or_above(2) > 0.4;
    match (team.confidence >= AUTO_ROUTE_CONFIDENCE, escalate) {
        (_, true) => println!("\n-> escalate to success, notify the account owner"),
        (true, false) => println!("\n-> auto-route to {}", team.choice),
        (false, false) => println!("\n-> queue for manual triage (distribution too flat)"),
    }
    Ok(())
}
