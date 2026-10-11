//! Ask for a judgment directly — no agent, no session, no history.
//!
//! The counterpart to `direct_llm`: same shape, different contract. A model
//! answers in prose you have to parse; a decision service answers in numbers your code
//! can act on.
//!
//! OpenAI's Decisions API on `gpt-6-luna` (requires OPENAI_API_KEY):
//! ```text
//! cargo run -p everruns --features openai --example direct_decisions
//! ```
//! An optional positional argument replaces the content being classified.

use everruns::{Decisions, OpenAI, Provider};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let content = args.next().unwrap_or_else(|| {
        "I've been on hold for two hours and nobody can tell me why my card was charged twice."
            .into()
    });
    if args.next().is_some() || content.trim().is_empty() {
        return Err("Usage: direct_decisions [CONTENT]".into());
    }

    // An OpenAI provider carries the Decisions API next to chat, so the same
    // credential reaches both. TypeSafe System One plugs in the same way:
    // `Decisions::new("jev-latest", TypeSafeAI::from_env()?)` with `typesafe`.
    println!("OpenAI Decisions API over HTTP.\n");
    let decisions = Decisions::new("gpt-6-luna", Provider::from(OpenAI::from_env()?));

    // One question, one number. This is the whole API for the common case.
    println!("> {content}");
    let urgency = decisions
        .probability("Does this convey urgency?", content.clone())
        .await?;
    println!("urgency: {urgency:.2}\n");

    // Independent questions ride one request and answer in parallel, so asking
    // several is the cheap path. Ids label answers for this code and are never
    // shown to the model, so each question reads on its own.
    let answers = decisions
        .about(content)
        .noul("urgent", "Does this convey urgency?")
        .score(
            "severity",
            "How severe is the problem the writer describes?",
            [
                "A minor annoyance",
                "A real problem with their account",
                "Serious harm requiring immediate action",
            ],
        )
        .choice(
            "queue",
            "Which team should handle this message?",
            ["billing", "technical", "sales"],
        )
        .send()
        .await?;

    println!("urgent:   {:.2}", answers.probability("urgent")?);
    println!(
        "severity: {:.2} of 2  (probability it is at least a real problem: {:.2})",
        answers.score("severity")?,
        // The tail, not the average: something probably fine but possibly
        // awful must not average into fine.
        answers.tail("severity", 1)?
    );
    println!("queue:    {}", answers.selected("queue")?);

    let usage = &answers.outcome().usage;
    println!(
        "\nthree questions, one request ({} input tokens)",
        usage.input_tokens
    );
    Ok(())
}
