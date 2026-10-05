//! An order pipeline on the generic workflow engine, with no other Everruns
//! crate and no database: `reserve_stock`, then `charge_card`, then `ship`.
//!
//! The card processor fails its first attempt, so the run also shows the task
//! queue retrying an activity under its `RetryPolicy` while the workflow waits.
//!
//! ```sh
//! cargo run -p everruns-durable --example order_pipeline
//! ```
//!
//! CI runs it with `cargo test --example order_pipeline`; it asserts its own
//! outcome and exits non-zero on any failure.

use std::time::Duration;

use everruns_durable::TaskFailureOutcome;
use everruns_durable::prelude::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

#[derive(Clone, Serialize, Deserialize)]
struct Order {
    id: String,
    sku: String,
    cents: u64,
}

/// The workflow: a deterministic state machine over activity results.
struct OrderPipeline {
    order: Order,
    tracking: Option<String>,
}

impl OrderPipeline {
    fn step(&self, activity: &str) -> WorkflowAction {
        let options =
            ActivityOptions::default().with_retry(RetryPolicy::fixed(Duration::from_millis(10), 3));
        WorkflowAction::ScheduleActivity {
            activity_id: activity.to_string(),
            activity_type: activity.to_string(),
            input: json!(self.order),
            options,
        }
    }
}

impl Workflow for OrderPipeline {
    const TYPE: &'static str = "order_pipeline";
    type Input = Order;
    type Output = String;

    fn new(order: Order) -> Self {
        Self {
            order,
            tracking: None,
        }
    }

    fn on_start(&mut self) -> Vec<WorkflowAction> {
        vec![self.step(ReserveStock::TYPE)]
    }

    fn on_activity_completed(&mut self, activity_id: &str, result: Value) -> Vec<WorkflowAction> {
        match activity_id {
            ReserveStock::TYPE => vec![self.step(ChargeCard::TYPE)],
            ChargeCard::TYPE => vec![self.step(Ship::TYPE)],
            _ => {
                let tracking = result.as_str().unwrap_or_default().to_string();
                self.tracking = Some(tracking.clone());
                vec![WorkflowAction::complete(json!(tracking))]
            }
        }
    }

    fn on_activity_failed(&mut self, step: &str, error: &ActivityError) -> Vec<WorkflowAction> {
        vec![WorkflowAction::fail(WorkflowError::new(format!(
            "{step}: {error}"
        )))]
    }

    fn is_completed(&self) -> bool {
        self.tracking.is_some()
    }

    fn result(&self) -> Option<String> {
        self.tracking.clone()
    }
}

/// Activities: the side effects, run by a worker outside the workflow.
struct ReserveStock;
struct ChargeCard;
struct Ship;

#[async_trait::async_trait]
impl Activity for ReserveStock {
    const TYPE: &'static str = "reserve_stock";
    type Input = Order;
    type Output = String;

    async fn execute(&self, _: &ActivityContext, order: Order) -> Result<String, ActivityError> {
        Ok(format!("hold-{}", order.sku))
    }
}

#[async_trait::async_trait]
impl Activity for ChargeCard {
    const TYPE: &'static str = "charge_card";
    type Input = Order;
    type Output = u64;

    async fn execute(&self, ctx: &ActivityContext, order: Order) -> Result<u64, ActivityError> {
        if ctx.attempt == 1 {
            return Err(ActivityError::retryable("card processor timed out"));
        }
        Ok(order.cents)
    }
}

#[async_trait::async_trait]
impl Activity for Ship {
    const TYPE: &'static str = "ship";
    type Input = Order;
    type Output = String;

    async fn execute(&self, _: &ActivityContext, order: Order) -> Result<String, ActivityError> {
        Ok(format!("track-{}", order.id))
    }
}

/// Decodes the task input, runs the activity and encodes its output.
async fn run<A>(activity: A, task: &ClaimedTask) -> Result<Value, ActivityError>
where
    A: Activity,
    A::Input: DeserializeOwned,
{
    let workflow_id = task.workflow_id.expect("workflow activity");
    let ctx = ActivityContext::new(
        workflow_id,
        task.activity_id.clone(),
        task.attempt,
        task.max_attempts,
    );
    let input = serde_json::from_value(task.input.clone())
        .map_err(|e| ActivityError::non_retryable(e.to_string()))?;
    let output = activity.execute(&ctx, input).await?;
    serde_json::to_value(output).map_err(|e| ActivityError::non_retryable(e.to_string()))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut executor = WorkflowExecutor::new(InMemoryWorkflowEventStore::new());
    executor.register::<OrderPipeline>();
    let store = executor.store();

    let order = Order {
        id: "A-1001".into(),
        sku: "kettle".into(),
        cents: 4_900,
    };
    let id = executor
        .start_workflow::<OrderPipeline>(order, None)
        .await?;

    // A minimal worker: claim, execute, report the outcome to the store and
    // the executor, until the workflow leaves the running states.
    let types = [ReserveStock::TYPE, ChargeCard::TYPE, Ship::TYPE].map(String::from);
    store
        .register_worker(WorkerInfo::new("worker-1", types.clone()))
        .await?;
    let mut retries = 0;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !store.get_workflow_info(id).await?.status.is_terminal() {
            let tasks = store.claim_task("worker-1", &types, 10).await?;
            if tasks.is_empty() {
                // A failed attempt is released after its retry delay.
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            for task in tasks {
                let outcome = match task.activity_type.as_str() {
                    ReserveStock::TYPE => run(ReserveStock, &task).await,
                    ChargeCard::TYPE => run(ChargeCard, &task).await,
                    Ship::TYPE => run(Ship, &task).await,
                    other => unreachable!("no handler for {other}"),
                };
                match outcome {
                    Ok(output) => {
                        store
                            .complete_task(task.id, "worker-1", output.clone())
                            .await?;
                        executor
                            .on_activity_completed(id, &task.activity_id, output)
                            .await?;
                    }
                    Err(error) => {
                        let failure = store
                            .fail_task_with_retry(task.id, &error.message, error.retryable)
                            .await?;
                        let will_retry = matches!(failure, TaskFailureOutcome::WillRetry { .. });
                        retries += usize::from(will_retry);
                        executor
                            .on_activity_failed(id, &task.activity_id, error, will_retry)
                            .await?;
                    }
                }
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await??;

    let info = store.get_workflow_info(id).await?;
    println!(
        "order {id}: {:?}, result {:?}, retries {retries}",
        info.status, info.result
    );
    assert_eq!(info.status, WorkflowStatus::Completed);
    assert_eq!(info.result, Some(json!("track-A-1001")));
    assert_eq!(retries, 1, "the first card charge should be retried once");
    Ok(())
}
