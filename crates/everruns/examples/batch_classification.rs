//! Submit offline classification, then resume in another process.
//!
//! ANTHROPIC_API_KEY=... cargo run -p everruns --features anthropic \
//!   --example batch_classification -- submit /tmp/classification-001
//! Repeat with `poll`, `collect` or `cancel` and the same directory.
//! Set BATCH_SCOPE to the same trusted tenant/account on every invocation.
//!
//! BATCH_PROVIDER=anthropic|openai|gemini selects the enabled provider feature.
//! This calls the real provider batch API and can incur charges.
//! A new directory is required for each submission. An intent without a ticket
//! means creation may have succeeded: reconcile in the provider console before retrying.

mod batch_support;

use std::{error::Error, fs::OpenOptions, io::Write, path::Path};

use everruns::{
    Model,
    batch::{Batch, BatchTicket},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Sentiment {
    Positive,
    Negative,
    Neutral,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Classification {
    sentiment: Sentiment,
}

#[derive(Serialize, Deserialize)]
struct Intent {
    scope: String,
    model: String,
    items: Vec<(String, String)>,
}

fn save(path: &Path, value: &impl Serialize) -> Result<(), Box<dyn Error>> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let valid = match args.first().map(String::as_str) {
        Some("recover") => args.len() == 3,
        Some("submit" | "poll" | "collect" | "cancel") => args.len() == 2,
        _ => false,
    };
    if !valid {
        return Err("usage: batch_classification <submit|poll|collect|cancel> STATE_DIRECTORY; or recover STATE_DIRECTORY PROVIDER_JOB_ID".into());
    }
    let (provider, provider_name, default_model) = batch_support::provider()?;
    let scope = std::env::var("BATCH_SCOPE")
        .unwrap_or_else(|_| format!("classification-example/{provider_name}-default"));
    let batch = Batch::new(provider, scope.clone());
    let directory = Path::new(&args[1]);
    if args[0] == "submit" {
        let model_id = std::env::var("BATCH_MODEL").unwrap_or_else(|_| default_model.into());
        let model = Model::from(model_id.as_str());
        let items = [
            (
                "review_001",
                "The delivery arrived early and everything works.",
            ),
            ("review_002", "The replacement still does not turn on."),
            ("review_003", "The package contains two adapters."),
        ];
        // Reserve a durable intent before the non-idempotent provider call.
        std::fs::create_dir(directory)?;
        save(
            &directory.join("intent.json"),
            &Intent {
                scope: scope.clone(),
                model: model_id,
                items: items
                    .iter()
                    .map(|(id, text)| (id.to_string(), text.to_string()))
                    .collect(),
            },
        )?;
        let mut submission = batch;
        for (id, text) in items {
            submission=submission.request(id,model.completion()
                .system("Classify the review. Reply with only JSON: {\"sentiment\":\"positive\"}, {\"sentiment\":\"negative\"}, or {\"sentiment\":\"neutral\"}.")
                .user(text).max_tokens(128))?;
        }
        let ticket = submission.submit().await?;
        save(&directory.join("ticket.json"), &ticket)?;
        println!(
            "Submitted {} requests as {}. Run poll with the same directory later.",
            ticket.request_ids().len(),
            ticket.job_id()
        );
        return Ok(());
    }
    if args[0] == "recover" {
        let intent: Intent =
            serde_json::from_slice(&std::fs::read(directory.join("intent.json"))?)?;
        if intent.scope != scope {
            return Err("intent belongs to a different tenant/account scope".into());
        }
        let ticket = batch
            .recover(args[2].clone(), intent.items.into_iter().map(|(id, _)| id))
            .await?;
        save(&directory.join("ticket.json"), &ticket)?;
        println!("Recovered {} without submitting new work.", ticket.job_id());
        return Ok(());
    }
    let ticket: BatchTicket =
        serde_json::from_slice(&std::fs::read(directory.join("ticket.json"))?)?;
    match args[0].as_str() {
        "poll" => println!("{:?}", batch.poll(&ticket).await?),
        "cancel" => println!(
            "{:?}; keep polling, then collect partial results",
            batch.cancel(&ticket).await?
        ),
        "collect" => {
            let mut output = Vec::<Value>::new();
            for item in batch.collect(&ticket).await? {
                output.push(match item.result {
                    Ok(response)=>match serde_json::from_str::<Classification>(&response.text) {
                        Ok(classification)=>json!({"id":item.id,"classification":classification,"tokens":response.metadata.total_tokens}),
                        Err(_)=>json!({"id":item.id,"error":"invalid classification JSON","raw":response.text}),
                    },
                    Err(error)=>json!({"id":item.id,"error":format!("{error:?}")}),
                });
            }
            save(&directory.join("results.json"), &output)?;
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => unreachable!(),
    }
    Ok(())
}
