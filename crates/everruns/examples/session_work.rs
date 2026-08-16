//! Run with: `cargo run -p everruns --example session_work`

use std::time::Duration;

use everruns::work::{TaskOutcome, TaskRequest, WakePolicy, WakeReason, WorkQueue};
use everruns::{Agent, Engine, Model};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("You summarize completed background work.")
        .model(Model::simulated("Background work handled."))
        .build()?;
    let session = Engine::new().create(agent.clone());

    let queue = WorkQueue::in_memory();
    let session_work = session.work(&queue);
    session_work
        .submit(
            TaskRequest::new("thumbnail", json!({ "image": "cover.png" }))
                .idempotency_key("thumbnail:cover.png")
                .wake_policy(WakePolicy::OnCompletion),
        )
        .await?;

    // The host owns polling and execution. A durable host can replace the
    // backend while application code keeps using the same API.
    for delivery in queue.claim_due(Duration::from_secs(30), 16).await? {
        let output = match delivery.task.kind.as_str() {
            "thumbnail" => json!({ "path": "cover-thumb.png" }),
            kind => return Err(format!("no handler for {kind}").into()),
        };
        queue
            .finish(
                &delivery,
                TaskOutcome::success_with_summary("thumbnail ready", output),
            )
            .await?;
    }

    for delivery in queue.claim_wakes(Duration::from_secs(30), 16).await? {
        if let WakeReason::TaskFinished { outcome, .. } = &delivery.wake.reason {
            session
                .run(format!("Background task completed: {outcome:?}"))
                .await?;
        }
        queue.acknowledge_wake(&delivery).await?;
    }
    Ok(())
}
