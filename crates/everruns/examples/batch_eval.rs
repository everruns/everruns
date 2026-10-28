//! Offline evals with one provider batch and application-owned polling.
//!
//! ANTHROPIC_API_KEY=... cargo run -p everruns --features anthropic --example batch_eval
//! This calls a real provider, incurs charges and can take up to 24 hours.
//! Unlike batch_classification, this short example keeps its ticket in memory;
//! restarting it submits another job. Use the persisted example for recovery.

use everruns::{
    Model, Provider,
    batch::{Batch, BatchStatus},
    providers::anthropic::Anthropic,
};
use std::{error::Error, time::Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let provider: Provider = Anthropic::from_env()?.into();
    let model = Model::from("claude-sonnet-5");
    let cases = [
        ("france", "France", "Paris"),
        ("japan", "Japan", "Tokyo"),
        ("italy", "Italy", "Rome"),
    ];
    let mut submission = Batch::new(provider.clone(), "capital-eval/anthropic-default");
    for (id, country, _) in cases {
        submission = submission.request(
            id,
            model
                .completion()
                .system("Answer with only the capital city name, no punctuation.")
                .user(format!("What is the capital of {country}?"))
                .max_tokens(64),
        )?;
    }
    let ticket = submission.submit().await?;
    println!(
        "Submitted {}. Waiting for offline processing...",
        ticket.job_id()
    );
    let client = Batch::new(provider, "capital-eval/anthropic-default");
    while client.poll(&ticket).await? != BatchStatus::Ended {
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
    let mut correct = 0;
    for result in client.collect(&ticket).await? {
        let expected = cases
            .iter()
            .find(|(id, _, _)| *id == result.id)
            .ok_or("unexpected case ID")?
            .2;
        match result.result {
            Ok(response) => {
                let passed = response.text.trim() == expected;
                correct += usize::from(passed);
                println!(
                    "{}: {} (expected {expected:?}, got {:?})",
                    result.id,
                    if passed { "PASS" } else { "FAIL" },
                    response.text.trim()
                );
            }
            Err(error) => println!("{}: ERROR {error:?}", result.id),
        }
    }
    println!("Accuracy: {correct}/{}", cases.len());
    Ok(())
}
