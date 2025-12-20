// Minimal worker that runs the forever-loop workflow
//
// Usage: cargo run --example v3_loop

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use temporal_sdk_core::protos::coresdk::common::Payload;
use temporal_sdk_core::protos::coresdk::workflow_activation::wf_activation_job;
use temporal_sdk_core::protos::coresdk::workflow_commands::{
    workflow_command, CompleteWorkflowExecution, ScheduleActivity, StartTimer,
};
use temporal_sdk_core::protos::coresdk::workflow_completion::WfActivationCompletion;
use temporal_sdk_core::protos::coresdk::{activity_result::ActivityResult, ActivityTaskCompletion};
use temporal_sdk_core::{Core, CoreInitOptions, ServerGatewayOptions, Url};
use tokio::sync::Mutex;
use tracing::{debug, error, info};

use super::{AgentSessionWorkflow, LoopCommand, LoopInput};

pub const TASK_QUEUE: &str = "v3-loop";
pub const WORKFLOW_TYPE: &str = "loop_workflow";
pub const ACTIVITY_TYPE: &str = "do_work";

/// Run the v3 worker
pub async fn run_v3_worker(temporal_addr: &str) -> Result<()> {
    let url = Url::parse(temporal_addr).context("Invalid temporal address")?;

    let opts = ServerGatewayOptions {
        target_url: url,
        namespace: "default".to_string(),
        task_queue: TASK_QUEUE.to_string(),
        identity: format!("v3-worker-{}", std::process::id()),
        worker_binary_id: "v3-loop".to_string(),
        long_poll_timeout: Duration::from_secs(60),
    };

    let init = CoreInitOptions {
        gateway_opts: opts,
        // Keep workflows in cache to avoid replay complexity in this minimal example
        evict_after_pending_cleared: false,
        max_outstanding_workflow_tasks: 100,
        max_outstanding_activities: 100,
    };

    info!("Connecting to Temporal at {}", temporal_addr);
    let core = temporal_sdk_core::init(init).await?;
    let core: Arc<dyn Core> = Arc::new(core);
    info!("Connected. Polling task queue: {}", TASK_QUEUE);

    let workflows: Arc<Mutex<HashMap<String, AgentSessionWorkflow>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let wf_core = core.clone();
    let act_core = core.clone();
    let wf_workflows = workflows.clone();

    let wf_handle = tokio::spawn(async move {
        loop {
            if let Err(e) = poll_workflow(&wf_core, &wf_workflows).await {
                error!("Workflow poll error: {}", e);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    });

    let act_handle = tokio::spawn(async move {
        loop {
            if let Err(e) = poll_activity(&act_core).await {
                error!("Activity poll error: {}", e);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    });

    tokio::select! {
        _ = wf_handle => {}
        _ = act_handle => {}
    }

    Ok(())
}

async fn poll_workflow(
    core: &Arc<dyn Core>,
    workflows: &Arc<Mutex<HashMap<String, AgentSessionWorkflow>>>,
) -> Result<()> {
    let task = core.poll_workflow_task().await?;
    let mut wfs = workflows.lock().await;

    let mut commands = vec![];

    for job in &task.jobs {
        match &job.variant {
            Some(wf_activation_job::Variant::StartWorkflow(start)) => {
                info!("Starting workflow: {}", start.workflow_id);

                let input: LoopInput = start
                    .arguments
                    .first()
                    .and_then(|p| serde_json::from_slice(&p.data).ok())
                    .unwrap_or_default();

                let mut wf = AgentSessionWorkflow::new(input);
                let cmd = wf.on_start();
                commands.push(command_to_proto(cmd));
                wfs.insert(task.run_id.clone(), wf);
            }

            Some(wf_activation_job::Variant::FireTimer(timer)) => {
                debug!("Timer fired: {}", timer.timer_id);
                if let Some(wf) = wfs.get_mut(&task.run_id) {
                    let seq: u32 = timer.timer_id.parse().unwrap_or(0);
                    let cmd = wf.on_timer_fired(seq);
                    commands.push(command_to_proto(cmd));
                }
            }

            Some(wf_activation_job::Variant::ResolveActivity(resolve)) => {
                debug!("Activity resolved: {}", resolve.activity_id);
                if let Some(wf) = wfs.get_mut(&task.run_id) {
                    let seq: u32 = resolve.activity_id.parse().unwrap_or(0);
                    let cmd = wf.on_activity_completed(seq);
                    info!("Iteration: {}", wf.iteration());
                    commands.push(command_to_proto(cmd));
                }
            }

            Some(wf_activation_job::Variant::SignalWorkflow(_)) => {
                debug!("Signal received");
                if let Some(wf) = wfs.get_mut(&task.run_id) {
                    let cmd = wf.on_signal();
                    commands.push(command_to_proto(cmd));
                }
            }

            Some(wf_activation_job::Variant::RemoveFromCache(_)) => {
                wfs.remove(&task.run_id);
            }

            _ => {}
        }
    }

    let completion = WfActivationCompletion::ok_from_cmds(
        commands.into_iter().flatten().collect(),
        task.task_token,
    );
    core.complete_workflow_task(completion).await?;

    Ok(())
}

async fn poll_activity(core: &Arc<dyn Core>) -> Result<()> {
    use temporal_sdk_core::protos::coresdk::activity_task::activity_task;

    let task = core.poll_activity_task().await?;

    if let Some(activity_task::Variant::Start(start)) = &task.variant {
        let iteration: u32 = start
            .input
            .first()
            .and_then(|p| serde_json::from_slice(&p.data).ok())
            .unwrap_or(0);

        info!("Activity executing: iteration={}", iteration);

        // Simulate work
        tokio::time::sleep(Duration::from_millis(100)).await;

        let result = ActivityResult::ok(Payload {
            data: serde_json::to_vec(&iteration).unwrap_or_default(),
            metadata: Default::default(),
        });

        let completion = ActivityTaskCompletion {
            task_token: task.task_token,
            result: Some(result),
        };
        core.complete_activity_task(completion).await?;
    }

    Ok(())
}

fn command_to_proto(cmd: LoopCommand) -> Option<workflow_command::Variant> {
    match cmd {
        LoopCommand::StartTimer { seq, seconds } => {
            Some(workflow_command::Variant::StartTimer(StartTimer {
                timer_id: seq.to_string(),
                start_to_fire_timeout: Some(Duration::from_secs(seconds).into()),
            }))
        }
        LoopCommand::ScheduleActivity { seq, iteration } => {
            Some(workflow_command::Variant::ScheduleActivity(
                ScheduleActivity {
                    activity_id: seq.to_string(),
                    activity_type: ACTIVITY_TYPE.to_string(),
                    task_queue: TASK_QUEUE.to_string(),
                    arguments: vec![Payload {
                        data: serde_json::to_vec(&iteration).unwrap_or_default(),
                        metadata: Default::default(),
                    }],
                    schedule_to_start_timeout: Some(Duration::from_secs(60).into()),
                    start_to_close_timeout: Some(Duration::from_secs(60).into()),
                    ..Default::default()
                },
            ))
        }
        LoopCommand::Complete { iteration } => {
            info!("Workflow completing after {} iterations", iteration);
            Some(workflow_command::Variant::CompleteWorkflowExecution(
                CompleteWorkflowExecution {
                    result: Some(Payload {
                        data: serde_json::to_vec(&iteration).unwrap_or_default(),
                        metadata: Default::default(),
                    }),
                },
            ))
        }
        LoopCommand::None => None,
    }
}
