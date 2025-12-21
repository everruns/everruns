// Temporal worker implementation
// Decision: Use the temporal-sdk-core's Core trait for polling and completion
//
// This worker:
// 1. Polls for workflow tasks and drives workflow state machines
// 2. Polls for activity tasks and executes activities
// 3. Reports activity heartbeats for long-running operations
// 4. Handles graceful shutdown

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use everruns_storage::repositories::Database;
use temporal_sdk_core::protos::coresdk::{
    activity_result::{self, ActivityResult},
    activity_task::{activity_task, ActivityTask},
    common::Payload,
    workflow_activation::{wf_activation_job, WfActivation},
    workflow_commands::{
        workflow_command, CompleteWorkflowExecution, FailWorkflowExecution, ScheduleActivity,
    },
    workflow_completion::{self, WfActivationCompletion},
    ActivityTaskCompletion,
};
use temporal_sdk_core::{PollActivityError, PollWfError};
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::activities::{
    activity_types, call_model_activity, execute_tool_activity, load_agent_activity,
    CallModelInput, ExecuteToolInput, LoadAgentInput,
};
use crate::client::TemporalWorkerCore;
use crate::runner::RunnerConfig;
use crate::types::*;
use crate::workflow_registry::WorkflowRegistry;
use crate::traits::Workflow;

/// Temporal worker that processes workflow and activity tasks
pub struct TemporalWorker {
    /// Temporal core for polling (wrapped in Arc for sharing)
    core: Arc<TemporalWorkerCore>,
    /// Database connection
    db: Database,
    /// Worker configuration
    #[allow(dead_code)]
    config: RunnerConfig,
    /// Workflow registry for creating workflow instances
    registry: Arc<WorkflowRegistry>,
    /// Shutdown signal sender
    shutdown_tx: watch::Sender<bool>,
    /// Shutdown signal receiver
    shutdown_rx: watch::Receiver<bool>,
}

impl TemporalWorker {
    /// Create a new Temporal worker with default workflow registry
    pub async fn new(config: RunnerConfig, db: Database) -> Result<Self> {
        Self::with_registry(config, db, WorkflowRegistry::with_defaults()).await
    }

    /// Create a new Temporal worker with a custom workflow registry
    pub async fn with_registry(
        config: RunnerConfig,
        db: Database,
        registry: WorkflowRegistry,
    ) -> Result<Self> {
        let core = TemporalWorkerCore::new(config.clone())
            .await
            .context("Failed to create Temporal worker core")?;

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Ok(Self {
            core: Arc::new(core),
            db,
            config,
            registry: Arc::new(registry),
            shutdown_tx,
            shutdown_rx,
        })
    }

    /// Run the worker, processing tasks until shutdown
    pub async fn run(&self) -> Result<()> {
        info!(
            task_queue = %self.config.temporal_task_queue(),
            "Starting Temporal worker"
        );

        // Spawn workflow task poller
        let workflow_handle = spawn_workflow_poller(
            self.core.clone(),
            self.db.clone(),
            self.registry.clone(),
            self.shutdown_rx.clone(),
        );

        // Spawn activity task poller
        let activity_handle =
            spawn_activity_poller(self.core.clone(), self.db.clone(), self.shutdown_rx.clone());

        // Wait for shutdown signal
        let mut shutdown_rx = self.shutdown_rx.clone();
        shutdown_rx.changed().await.ok();

        info!("Shutdown signal received, stopping pollers");

        // Cancel polling tasks
        workflow_handle.abort();
        activity_handle.abort();

        // Shutdown the core
        self.core.shutdown().await;

        info!("Temporal worker stopped");
        Ok(())
    }

    /// Signal the worker to shutdown
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

/// Spawn workflow task polling loop
fn spawn_workflow_poller(
    core: Arc<TemporalWorkerCore>,
    db: Database,
    registry: Arc<WorkflowRegistry>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> JoinHandle<()> {
    // Track active workflow instances (using trait objects for dynamic dispatch)
    let workflows: Arc<Mutex<HashMap<String, Box<dyn Workflow>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    info!("Workflow poller shutting down");
                    break;
                }
                result = poll_and_process_workflow_task(&core, &db, &registry, workflows.clone()) => {
                    if let Err(e) = result {
                        match e.downcast_ref::<PollWfError>() {
                            Some(PollWfError::ShutDown) => {
                                info!("Workflow poller received shutdown");
                                break;
                            }
                            _ => {
                                error!(error = %e, "Workflow task processing error");
                                // Brief pause before retry
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                        }
                    }
                }
            }
        }
    })
}

/// Spawn activity task polling loop
fn spawn_activity_poller(
    core: Arc<TemporalWorkerCore>,
    db: Database,
    mut shutdown_rx: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    info!("Activity poller shutting down");
                    break;
                }
                result = poll_and_process_activity_task(&core, &db) => {
                    if let Err(e) = result {
                        match e.downcast_ref::<PollActivityError>() {
                            Some(PollActivityError::ShutDown) => {
                                info!("Activity poller received shutdown");
                                break;
                            }
                            _ => {
                                error!(error = %e, "Activity task processing error");
                                // Brief pause before retry
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                        }
                    }
                }
            }
        }
    })
}

/// Poll and process a single workflow task
async fn poll_and_process_workflow_task(
    core: &TemporalWorkerCore,
    _db: &Database,
    registry: &WorkflowRegistry,
    workflows: Arc<Mutex<HashMap<String, Box<dyn Workflow>>>>,
) -> Result<()> {
    // Poll for workflow task
    let task = core.core().poll_workflow_task().await?;

    debug!(
        run_id = %task.run_id,
        jobs = task.jobs.len(),
        "Received workflow task"
    );

    // Check if this is only a RemoveFromCache job - if so, just remove and don't complete
    let is_only_eviction = task.jobs.len() == 1
        && task.jobs.first().is_some_and(|j| {
            matches!(
                j.variant,
                Some(wf_activation_job::Variant::RemoveFromCache(_))
            )
        });

    if is_only_eviction {
        debug!(run_id = %task.run_id, "Handling eviction-only task");
        let mut workflows_guard = workflows.lock().await;
        workflows_guard.remove(&task.run_id);
        // Don't send completion for eviction-only tasks
        return Ok(());
    }

    // Process workflow activation
    let commands = process_workflow_activation(&task, registry, workflows).await?;

    // Build completion
    let completion = if commands.is_empty() {
        WfActivationCompletion {
            task_token: task.task_token,
            status: Some(
                workflow_completion::wf_activation_completion::Status::Successful(
                    workflow_completion::Success { commands: vec![] },
                ),
            ),
        }
    } else {
        // Convert commands to variants
        let variants: Vec<workflow_command::Variant> =
            commands.into_iter().filter_map(|cmd| cmd.variant).collect();
        WfActivationCompletion::ok_from_cmds(variants, task.task_token)
    };

    // Complete the workflow task
    core.core().complete_workflow_task(completion).await?;

    Ok(())
}

/// Process workflow activation and return commands
async fn process_workflow_activation(
    task: &WfActivation,
    registry: &WorkflowRegistry,
    workflows: Arc<Mutex<HashMap<String, Box<dyn Workflow>>>>,
) -> Result<Vec<temporal_sdk_core::protos::coresdk::workflow_commands::WorkflowCommand>> {
    let mut workflows_guard = workflows.lock().await;
    let mut commands = vec![];

    debug!(
        run_id = %task.run_id,
        job_count = task.jobs.len(),
        jobs = ?task.jobs.iter().map(|j| match &j.variant {
            Some(wf_activation_job::Variant::StartWorkflow(_)) => "StartWorkflow",
            Some(wf_activation_job::Variant::ResolveActivity(r)) => &r.activity_id,
            Some(wf_activation_job::Variant::RemoveFromCache(_)) => "RemoveFromCache",
            _ => "Other",
        }).collect::<Vec<_>>(),
        "Processing workflow activation"
    );

    for job in &task.jobs {
        match &job.variant {
            Some(wf_activation_job::Variant::StartWorkflow(start)) => {
                info!(
                    workflow_id = %start.workflow_id,
                    workflow_type = %start.workflow_type,
                    "Starting workflow"
                );

                // Parse workflow input as raw JSON
                let input: serde_json::Value = if let Some(args) = start.arguments.first() {
                    serde_json::from_slice(&args.data).context("Failed to parse workflow input")?
                } else {
                    return Err(anyhow::anyhow!("Workflow started without input"));
                };

                // Create workflow instance using registry
                let mut workflow = registry
                    .create(&start.workflow_type, input)
                    .context("Failed to create workflow")?;

                // Start the workflow
                let actions = workflow.on_start();

                // Process actions
                for action in actions {
                    if let Some(cmd) = action_to_command(action) {
                        commands.push(cmd);
                    }
                }

                // Store workflow (now Box<dyn Workflow>)
                workflows_guard.insert(task.run_id.clone(), workflow);
            }

            Some(wf_activation_job::Variant::ResolveActivity(resolve)) => {
                // Log the activity result status
                let result_status = match &resolve.result {
                    Some(ActivityResult {
                        status: Some(activity_result::activity_result::Status::Completed(_)),
                    }) => "Completed",
                    Some(ActivityResult {
                        status: Some(activity_result::activity_result::Status::Failed(f)),
                    }) => {
                        debug!(
                            activity_id = %resolve.activity_id,
                            failure_message = ?f.failure.as_ref().map(|f| &f.message),
                            "Activity failed"
                        );
                        "Failed"
                    }
                    Some(ActivityResult {
                        status: Some(activity_result::activity_result::Status::Canceled(_)),
                    }) => "Canceled",
                    _ => "Unknown",
                };

                debug!(
                    activity_id = %resolve.activity_id,
                    result_status = %result_status,
                    "Activity resolved"
                );

                if let Some(workflow) = workflows_guard.get_mut(&task.run_id) {
                    let actions = match &resolve.result {
                        Some(ActivityResult {
                            status:
                                Some(activity_result::activity_result::Status::Completed(success)),
                        }) => {
                            let result = success
                                .result
                                .as_ref()
                                .map(|p| serde_json::from_slice(&p.data).unwrap_or_default())
                                .unwrap_or_default();
                            debug!(
                                activity_id = %resolve.activity_id,
                                result = %result,
                                "Activity completed with result"
                            );
                            workflow.on_activity_completed(&resolve.activity_id, result)
                        }
                        Some(ActivityResult {
                            status: Some(activity_result::activity_result::Status::Failed(failure)),
                        }) => {
                            let error = failure
                                .failure
                                .as_ref()
                                .map(|f| f.message.clone())
                                .unwrap_or_else(|| "Unknown error".to_string());
                            error!(
                                activity_id = %resolve.activity_id,
                                error = %error,
                                "Activity FAILED - calling on_activity_failed"
                            );
                            workflow.on_activity_failed(&resolve.activity_id, &error)
                        }
                        _ => {
                            warn!(
                                activity_id = %resolve.activity_id,
                                "Unexpected activity result status"
                            );
                            vec![]
                        }
                    };

                    for action in actions {
                        if let Some(cmd) = action_to_command(action) {
                            commands.push(cmd);
                        }
                    }
                } else {
                    warn!(
                        run_id = %task.run_id,
                        activity_id = %resolve.activity_id,
                        "Workflow not found in cache for activity resolution"
                    );
                }
            }

            Some(wf_activation_job::Variant::RemoveFromCache(_)) => {
                // RemoveFromCache is handled at the task level for eviction-only tasks
                // For mixed tasks, we defer removal until after processing
                debug!(run_id = %task.run_id, "RemoveFromCache job received (deferring removal)");
            }

            other => {
                warn!(job = ?other, "Unhandled workflow activation job");
            }
        }
    }

    Ok(commands)
}

/// Convert WorkflowAction to Temporal command
fn action_to_command(
    action: WorkflowAction,
) -> Option<temporal_sdk_core::protos::coresdk::workflow_commands::WorkflowCommand> {
    use temporal_sdk_core::protos::coresdk::workflow_commands::WorkflowCommand;

    match action {
        WorkflowAction::ScheduleActivity {
            activity_id,
            activity_type,
            input,
        } => {
            let input_bytes = serde_json::to_vec(&input).unwrap_or_default();
            Some(WorkflowCommand {
                variant: Some(workflow_command::Variant::ScheduleActivity(
                    ScheduleActivity {
                        activity_id,
                        activity_type,
                        task_queue: TASK_QUEUE.to_string(),
                        arguments: vec![Payload {
                            data: input_bytes,
                            metadata: Default::default(),
                        }],
                        schedule_to_start_timeout: Some(Duration::from_secs(60).into()),
                        start_to_close_timeout: Some(Duration::from_secs(300).into()),
                        heartbeat_timeout: Some(Duration::from_secs(30).into()),
                        ..Default::default()
                    },
                )),
            })
        }
        WorkflowAction::CompleteWorkflow { result } => {
            let result_payload = result.map(|r| Payload {
                data: serde_json::to_vec(&r).unwrap_or_default(),
                metadata: Default::default(),
            });
            Some(WorkflowCommand {
                variant: Some(workflow_command::Variant::CompleteWorkflowExecution(
                    CompleteWorkflowExecution {
                        result: result_payload,
                    },
                )),
            })
        }
        WorkflowAction::FailWorkflow { reason } => Some(WorkflowCommand {
            variant: Some(workflow_command::Variant::FailWorkflowExecution(
                FailWorkflowExecution {
                    failure: Some(
                        temporal_sdk_core::protos::coresdk::common::UserCodeFailure {
                            message: reason,
                            ..Default::default()
                        },
                    ),
                },
            )),
        }),
        WorkflowAction::None => None,
    }
}

/// Poll and process a single activity task
async fn poll_and_process_activity_task(core: &TemporalWorkerCore, db: &Database) -> Result<()> {
    // Poll for activity task
    let task = core.core().poll_activity_task().await?;

    // Check for empty task token (invalid task)
    if task.task_token.is_empty() {
        warn!("Received activity task with empty task token, skipping");
        return Ok(());
    }

    debug!(
        task_token_len = task.task_token.len(),
        variant = ?task.variant.as_ref().map(|v| match v {
            activity_task::Variant::Start(s) => format!("Start({})", s.activity_type),
            activity_task::Variant::Cancel(_) => "Cancel".to_string(),
        }),
        "Received activity task"
    );

    // Process activity
    let result = process_activity(&task, db).await;

    // Complete the activity
    let completion = ActivityTaskCompletion {
        task_token: task.task_token,
        result: Some(result),
    };

    core.core().complete_activity_task(completion).await?;

    Ok(())
}

/// Process an activity task and return the result
async fn process_activity(task: &ActivityTask, db: &Database) -> ActivityResult {
    match &task.variant {
        Some(activity_task::Variant::Start(start)) => {
            // Check for empty activity type - this can happen with synthetic tasks
            if start.activity_type.is_empty() {
                error!(
                    workflow_execution = ?start.workflow_execution,
                    workflow_type = %start.workflow_type,
                    "Received activity task with empty activity_type - this may indicate a Temporal SDK issue or workflow bug"
                );
                return ActivityResult {
                    status: Some(activity_result::activity_result::Status::Failed(
                        activity_result::Failure {
                            failure: Some(
                                temporal_sdk_core::protos::coresdk::common::UserCodeFailure {
                                    message: format!(
                                        "Activity task has empty activity_type (workflow_type: {})",
                                        start.workflow_type
                                    ),
                                    ..Default::default()
                                },
                            ),
                        },
                    )),
                };
            }

            info!(
                activity_type = %start.activity_type,
                workflow_type = %start.workflow_type,
                "Executing activity"
            );

            let input_data = start
                .input
                .first()
                .map(|p| p.data.clone())
                .unwrap_or_default();

            let result = execute_activity(db, &start.activity_type, &input_data).await;

            match result {
                Ok(output) => {
                    let output_bytes = serde_json::to_vec(&output).unwrap_or_default();
                    ActivityResult::ok(Payload {
                        data: output_bytes,
                        metadata: Default::default(),
                    })
                }
                Err(e) => {
                    // Log the full error chain for debugging
                    let error_chain: Vec<String> = e.chain().map(|err| err.to_string()).collect();
                    error!(
                        error = %e,
                        error_chain = ?error_chain,
                        activity_type = %start.activity_type,
                        workflow_type = %start.workflow_type,
                        "Activity failed"
                    );
                    // Include the full error message in the Temporal failure
                    // This will be visible in Temporal UI and propagated to the workflow
                    let full_error = format!("{:#}", e);
                    ActivityResult {
                        status: Some(activity_result::activity_result::Status::Failed(
                            activity_result::Failure {
                                failure: Some(
                                    temporal_sdk_core::protos::coresdk::common::UserCodeFailure {
                                        message: full_error,
                                        ..Default::default()
                                    },
                                ),
                            },
                        )),
                    }
                }
            }
        }
        Some(activity_task::Variant::Cancel(_)) => {
            warn!("Activity cancellation requested");
            ActivityResult {
                status: Some(activity_result::activity_result::Status::Canceled(
                    activity_result::Cancelation { details: None },
                )),
            }
        }
        None => {
            error!("Activity task has no variant");
            ActivityResult {
                status: Some(activity_result::activity_result::Status::Failed(
                    activity_result::Failure {
                        failure: Some(
                            temporal_sdk_core::protos::coresdk::common::UserCodeFailure {
                                message: "Activity task has no variant".to_string(),
                                ..Default::default()
                            },
                        ),
                    },
                )),
            }
        }
    }
}

/// Execute an activity by type
async fn execute_activity(
    db: &Database,
    activity_type: &str,
    input_data: &[u8],
) -> Result<serde_json::Value> {
    match activity_type {
        activity_types::LOAD_AGENT => {
            let input: LoadAgentInput = serde_json::from_slice(input_data)?;
            let output = load_agent_activity(db.clone(), input).await?;
            Ok(serde_json::to_value(output)?)
        }
        activity_types::CALL_MODEL => {
            let input: CallModelInput = serde_json::from_slice(input_data)?;
            let output = call_model_activity(db.clone(), input).await?;
            Ok(serde_json::to_value(output)?)
        }
        activity_types::EXECUTE_TOOL => {
            let input: ExecuteToolInput = serde_json::from_slice(input_data)?;
            let output = execute_tool_activity(db.clone(), input).await?;
            Ok(serde_json::to_value(output)?)
        }
        _ => {
            // Provide a helpful error message with known activity types
            Err(anyhow::anyhow!(
                "Unknown activity type: '{}'. Known activities: load-agent, call-model, execute-tool. \
                This may indicate a workflow bug or version mismatch.",
                activity_type
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_to_command_schedule_activity() {
        let action = WorkflowAction::ScheduleActivity {
            activity_id: "test-1".to_string(),
            activity_type: "test_activity".to_string(),
            input: serde_json::json!({"key": "value"}),
        };

        let cmd = action_to_command(action);
        assert!(cmd.is_some());
    }

    #[test]
    fn test_action_to_command_complete_workflow() {
        let action = WorkflowAction::CompleteWorkflow {
            result: Some(serde_json::json!({"status": "done"})),
        };

        let cmd = action_to_command(action);
        assert!(cmd.is_some());
    }

    #[test]
    fn test_action_to_command_none() {
        let action = WorkflowAction::None;
        let cmd = action_to_command(action);
        assert!(cmd.is_none());
    }
}
