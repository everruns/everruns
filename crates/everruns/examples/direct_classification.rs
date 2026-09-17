//! Ask for a judgment directly — no agent, no session, no history.
//!
//! The counterpart to `direct_llm`: same shape, different contract. A model
//! answers in prose you have to parse; a judge answers in numbers your code
//! can act on.
//!
//! Offline (no API key):
//! ```text
//! cargo run -p everruns --example direct_classification
//! ```
//! TypeSafe System One (requires TYPESAFE_API_KEY):
//! ```text
//! cargo run -p everruns --features jev --example direct_classification -- --live
//! ```
//! An optional positional argument replaces the content being judged.

use everruns::Classifier;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1).peekable();
    let live = args.peek().is_some_and(|arg| arg == "--live");
    if live {
        args.next();
    }
    let content = args.next().unwrap_or_else(|| {
        "I've been on hold for two hours and nobody can tell me why my card was charged twice."
            .into()
    });
    if args.next().is_some() || content.trim().is_empty() {
        return Err("Usage: direct_classification [--live] [CONTENT]".into());
    }

    let judge = if live {
        #[cfg(feature = "jev")]
        {
            println!("TypeSafe System One over HTTP.\n");
            Classifier::new(everruns::TypeSafeClassifier::from_env()?)
        }
        #[cfg(not(feature = "jev"))]
        {
            return Err("Live mode requires: cargo run -p everruns --features jev --example direct_classification -- --live".into());
        }
    } else {
        println!("Offline simulator: fixed answers; no model inference.\n");
        Classifier::simulated(0.87)
    };

    // One question, one number. This is the whole API for the common case.
    println!("> {content}");
    let urgency = judge
        .probability("Does this convey urgency?", content.clone())
        .await?;
    println!("urgency: {urgency:.2}\n");

    // Independent questions ride one request and answer in parallel, so asking
    // several is the cheap path. Ids label answers for this code and are never
    // shown to the model, so each question reads on its own.
    let answers = judge
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
