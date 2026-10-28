//! Offline evals with one provider batch and application-owned polling.
//!
//! ANTHROPIC_API_KEY=... cargo run -p everruns --features anthropic --example batch_eval
//! This calls a real provider, incurs charges and can take up to 24 hours.
//! Unlike batch_classification, this short example keeps its ticket in memory;
//! restarting it submits another job. Use the persisted example for recovery.

mod batch_support;

use everruns::{
    LlmCallConfig,
    batch::{Batch, BatchRequest, BatchStatus},
    llm::{Message, MessageRole},
};
use std::{error::Error, time::Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let (provider, provider_name, default_model) = batch_support::provider()?;
    let scope = std::env::var("BATCH_SCOPE")
        .unwrap_or_else(|_| format!("capital-eval/{provider_name}-default"));
    let client = Batch::new(provider, scope);
    let mut config =
        LlmCallConfig::new(std::env::var("BATCH_MODEL").unwrap_or_else(|_| default_model.into()));
    config.max_tokens = Some(64);
    let cases = [
        ("france", "France", "Paris"),
        ("japan", "Japan", "Tokyo"),
        ("italy", "Italy", "Rome"),
    ];
    let requests = cases
        .iter()
        .map(|(id, country, _)| BatchRequest {
            id: (*id).into(),
            messages: vec![
                Message::text(
                    MessageRole::System,
                    "Answer with only the capital city name, no punctuation.",
                ),
                Message::text(
                    MessageRole::User,
                    format!("What is the capital of {country}?"),
                ),
            ],
            config: config.clone(),
        })
        .collect();
    let ticket = client.submit_requests(requests).await?;
    println!(
        "Submitted {}. Waiting for offline processing...",
        ticket.job_id()
    );
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
