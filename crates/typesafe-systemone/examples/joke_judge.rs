//! Rate jokes with calibrated numbers instead of an opinion in prose.
//!
//! ```sh
//! TYPESAFE_API_KEY=... cargo run -p typesafe-systemone --example joke_judge
//! ```
//!
//! The shape to notice: every judgment about one joke goes in a single request,
//! and the code — not the model — decides what the numbers mean.

use typesafe_systemone::{Evaluation, Question, TypeSafeClient};

const JOKES: &[&str] = &[
    "I told my wife she was drawing her eyebrows too high. She looked surprised.",
    "Why did the chicken cross the road? To get to the other side.",
    "The quarterly maintenance window begins at 02:00 UTC on Saturday.",
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = TypeSafeClient::from_env()?;

    for joke in JOKES {
        let judgment = client
            .evaluate(
                Evaluation::new(*joke)
                    .ask(
                        "is_joke",
                        Question::noul("Is this told as a joke, rather than a plain statement?")
                            .criteria("Told as a joke", "A plain statement or notice"),
                    )
                    .ask(
                        "humor",
                        Question::score(
                            "How funny would a general adult audience find this?",
                            [
                                "Not funny at all; no attempt at humor, or it fails completely",
                                "Mildly amusing; raises a smile but not a laugh",
                                "Genuinely funny; most people would laugh",
                                "Hilarious; lands with almost everyone",
                            ],
                        ),
                    )
                    // Speculative: only read when the joke actually lands.
                    .ask(
                        "is_risky",
                        Question::noul("Would this joke offend a general workplace audience?"),
                    ),
            )
            .await?;

        let is_joke = judgment.noul("is_joke")?;
        let humor = judgment.score("humor")?;

        println!("\n{joke}");
        println!("  joke?      {:.2}", is_joke);
        println!(
            "  humor      {:.2} of {} — {} (confidence {:.2})",
            humor.score,
            humor.level_count() - 1,
            humor.nearest_label().unwrap_or("?"),
            humor.confidence,
        );

        // Policy lives here, in code, and can change without re-running inference.
        let verdict = if is_joke < 0.5 {
            "not a joke; skipped"
        } else if judgment.noul("is_risky")? > 0.5 {
            "funny enough, but not for a work channel"
        } else if humor.probability_at_or_above(2) > 0.5 {
            "post it"
        } else {
            "keep workshopping"
        };
        println!("  verdict    {verdict}");
    }
    Ok(())
}
