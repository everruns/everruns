//! V3 Forever-Running Loop Example
//!
//! Minimal example of a Temporal workflow that runs forever.
//!
//! Prerequisites:
//!   docker compose -f harness/docker-compose.yml up temporal -d
//!
//! Run worker:
//!   cargo run --example v3_loop -p everruns-worker
//!
//! Start workflow (in another terminal):
//!   temporal workflow start --task-queue v3-loop --type loop_workflow --input '{"iteration":0}'
//!
//! Send signal to speed up:
//!   temporal workflow signal --workflow-id <id> --name wake

use everruns_worker::v3::{run_v3_worker, TASK_QUEUE, WORKFLOW_TYPE};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,everruns_worker=debug")
        .init();

    println!("=== V3 Forever Loop Example ===");
    println!();
    println!("Start a workflow with:");
    println!("  temporal workflow start \\");
    println!("    --task-queue {} \\", TASK_QUEUE);
    println!("    --type {} \\", WORKFLOW_TYPE);
    println!("    --input '{{\"iteration\":0}}'");
    println!();

    let addr = std::env::var("TEMPORAL_ADDRESS").unwrap_or_else(|_| "http://localhost:7233".into());
    run_v3_worker(&addr).await
}
