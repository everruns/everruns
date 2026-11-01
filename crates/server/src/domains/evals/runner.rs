// Eval run execution: one durable task per case result.
//
// Decision: each (case, trial) result is its own durable task on the server's
// eval pool (`EVAL_CASE_ACTIVITY`), claimed by one replica. A deploy or crash
// leaves a stale claim that the server's reaper hands to another replica, and
// the task resumes the same session instead of starting over: it counts the
// user messages already in the session and the turns that already settled,
// then carries on. This replaced a fire-and-forget task in the API process
// that left runs stuck at "running" after any restart.
//
// Decision: the run is finished by whichever case task settles last. The
// summary is rebuilt from the stored rows (summary.rs) and written with a
// conditional update, so concurrent finishers agree and a cancelled run stays
// cancelled.
//
// Decision: without a durable store (in-memory dev mode) the same case
// function runs on local tasks with bounded concurrency. Same behavior,
// weaker delivery: a restart loses in-flight cases.
//
// Decision: waiting polls the session's settled-turn count. Turn events are
// persisted by whichever replica the worker reports to, so an in-process
// listener cannot see them all; a 1 s poll on an indexed count is cheap.

use crate::domains::evals::graders::{GradeInput, GraderUsage, Graders};
use crate::domains::evals::record::*;
use crate::domains::messages::types::{CreateMessageRequest, InputMessage};
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::domains::sessions::types::CreateSessionRequest;
use crate::storage::StorageBackend;
use crate::storage::UpdateEvalCaseResultRow;
use anyhow::Result;
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::{INPUT_MESSAGE, TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED};
use everruns_core::message::TextAnnotation;
use everruns_durable::{ActivityOptions, TaskDefinition, WorkflowEventStore};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Durable activity type of one eval case result.
pub const EVAL_CASE_ACTIVITY: &str = "eval_case";
/// Cases one replica runs at once by default (`EVAL_MAX_CONCURRENT_CASES`).
const DEFAULT_CONCURRENCY: usize = 8;
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Upper bound on one case's wall time, whatever the case asks for.
const MAX_CASE_DURATION: Duration = Duration::from_secs(600);
const DEFAULT_CASE_TIMEOUT_SECS: u64 = 120;
/// A queued case may wait this long for a free slot before the queue gives up.
const SCHEDULE_TO_START: Duration = Duration::from_secs(24 * 3600);

/// Context needed to execute an eval run.
pub struct EvalRunContext {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    /// Scoring services shared with observers. Model-backed scorers report
    /// themselves unavailable when the deployment has no model stack.
    pub graders: Graders,
    /// Durable queue for case tasks, set once the server's store exists.
    /// Unset means local execution (dev mode, tests).
    pub queue: OnceLock<Arc<dyn WorkflowEventStore + Send + Sync>>,
}

/// One case result to execute: the payload of an `eval_case` task.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvalCaseTask {
    pub org_id: i64,
    pub run_id: Uuid,
    pub result_id: Uuid,
}

/// Start a run once the transaction that created it commits.
pub fn dispatch_eval_run(ctx: Arc<EvalRunContext>, org_id: i64, run_id: Uuid) {
    crate::storage::transaction::spawn_after_commit(async move {
        if let Err(e) = start_run(ctx.clone(), org_id, run_id).await {
            tracing::error!(run_id = %run_id, error = %e, "Eval run failed to start");
            if let Err(update_err) = ctx.db.update_eval_run_status(run_id, "failed", None).await {
                tracing::error!(run_id = %run_id, error = %update_err, "Failed to mark eval run as failed");
            }
        }
    });
}

async fn start_run(ctx: Arc<EvalRunContext>, org_id: i64, run_id: Uuid) -> Result<()> {
    ctx.db
        .update_eval_run_status(run_id, "running", None)
        .await?;
    let results = ctx.db.list_eval_case_results(run_id).await?;
    if results.is_empty() {
        finalize_run_if_complete(&ctx, run_id).await?;
        return Ok(());
    }
    let tasks: Vec<EvalCaseTask> = results
        .iter()
        .map(|r| EvalCaseTask {
            org_id,
            run_id,
            result_id: r.id,
        })
        .collect();

    if let Some(queue) = ctx.queue.get() {
        for task in &tasks {
            enqueue_case(queue.as_ref(), &ctx, task).await?;
        }
        return Ok(());
    }

    // Local fallback: bounded concurrency on this process.
    let semaphore = Arc::new(Semaphore::new(concurrency_from_env()));
    for task in tasks {
        let (ctx, sem) = (ctx.clone(), semaphore.clone());
        tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            run_case_task(&ctx, &task).await;
        });
    }
    Ok(())
}

async fn enqueue_case(
    queue: &(dyn WorkflowEventStore + Send + Sync),
    ctx: &EvalRunContext,
    task: &EvalCaseTask,
) -> Result<()> {
    // The task may outlive the case's own timeout by the time it takes to
    // create the session and score, never by more than this margin.
    let timeout = case_timeout(ctx, task).await + Duration::from_secs(300);
    queue
        .enqueue_task(TaskDefinition {
            workflow_id: None,
            activity_id: format!("{EVAL_CASE_ACTIVITY}:{}", task.result_id),
            activity_type: EVAL_CASE_ACTIVITY.to_string(),
            input: serde_json::to_value(task)?,
            options: ActivityOptions::default()
                .with_schedule_to_start_timeout(SCHEDULE_TO_START)
                .with_start_to_close_timeout(timeout),
        })
        .await?;
    Ok(())
}

async fn case_timeout(ctx: &EvalRunContext, task: &EvalCaseTask) -> Duration {
    let secs = match ctx.db.get_eval_case_result(task.result_id).await {
        Ok(Some(result)) => match ctx.db.get_eval_case(result.eval_case_id).await {
            Ok(Some(case)) => case.timeout_seconds.map(|v| v.max(1) as u64),
            _ => None,
        },
        _ => None,
    };
    Duration::from_secs(secs.unwrap_or(DEFAULT_CASE_TIMEOUT_SECS)).min(MAX_CASE_DURATION)
}

fn concurrency_from_env() -> usize {
    std::env::var("EVAL_MAX_CONCURRENT_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(DEFAULT_CONCURRENCY)
}

/// Start this replica's eval case pool over the durable store and route new
/// runs to it. Also re-queues pending cases of runs that were left running,
/// e.g. by a version without durable case tasks; a duplicate task for a case
/// another replica already holds finds it claimed and does nothing.
pub async fn start_case_pool(
    store: Arc<dyn WorkflowEventStore + Send + Sync>,
    ctx: Arc<EvalRunContext>,
) -> Option<JoinHandle<()>> {
    use everruns_durable::{WorkerPool, WorkerPoolConfig};

    if ctx.queue.set(store.clone()).is_err() {
        tracing::warn!("Eval case pool started twice; keeping the first");
        return None;
    }
    let config = WorkerPoolConfig::new(vec![EVAL_CASE_ACTIVITY.to_string()])
        .with_worker_id(format!("server-evals-{}", Uuid::now_v7()))
        .with_worker_group("server-evals")
        .with_max_concurrency(concurrency_from_env())
        // The server's stale-task reaper (durable_reaper.rs) owns reclaim.
        .without_stale_reclaim();
    let pool = WorkerPool::new(store.clone(), config);
    let handler_ctx = ctx.clone();
    pool.register_handler(EVAL_CASE_ACTIVITY, move |task| {
        let ctx = handler_ctx.clone();
        async move {
            match serde_json::from_value::<EvalCaseTask>(task.input) {
                Ok(task) => run_case_task(&ctx, &task).await,
                Err(error) => tracing::warn!(%error, "Malformed eval case task"),
            }
            Ok(serde_json::Value::Null)
        }
    });
    if let Err(error) = pool.start().await {
        tracing::error!(%error, "Failed to start eval case pool");
        return None;
    }

    match ctx.db.list_pending_eval_case_tasks().await {
        Ok(pending) => {
            for (org_id, run_id, result_id) in pending {
                let task = EvalCaseTask {
                    org_id,
                    run_id,
                    result_id,
                };
                if let Err(error) = enqueue_case(store.as_ref(), &ctx, &task).await {
                    tracing::warn!(%result_id, %error, "Failed to re-queue eval case");
                }
            }
        }
        Err(error) => tracing::warn!(%error, "Failed to list pending eval cases"),
    }

    tracing::info!("Eval case pool started");
    Some(tokio::spawn(async move {
        let _pool = pool;
        std::future::pending::<()>().await
    }))
}

/// Execute one case result, then finish the run if it was the last. Never
/// returns an error: failures are recorded on the result.
pub async fn run_case_task(ctx: &EvalRunContext, task: &EvalCaseTask) {
    if let Err(error) = run_case_task_inner(ctx, task).await {
        tracing::error!(result_id = %task.result_id, %error, "Eval case task failed");
    }
    if let Err(error) = finalize_run_if_complete(ctx, task.run_id).await {
        tracing::error!(run_id = %task.run_id, %error, "Failed to finish eval run");
    }
}

async fn run_case_task_inner(ctx: &EvalRunContext, task: &EvalCaseTask) -> Result<()> {
    let Some(run) = ctx.db.get_eval_run_by_id(task.run_id).await? else {
        return Ok(());
    };
    if run.org_id != task.org_id || !matches!(run.status.as_str(), "pending" | "running") {
        return Ok(());
    }
    let Some(result) = ctx.db.get_eval_case_result(task.result_id).await? else {
        return Ok(());
    };
    if result.eval_run_id != run.id {
        return Ok(());
    }
    match result.status.as_str() {
        // Fresh: take it. Losing the race means another task has it.
        "pending" => {
            if !ctx.db.claim_eval_case_result(result.id).await? {
                return Ok(());
            }
        }
        // Reclaimed after a crash or deploy: resume below.
        "running" => {}
        _ => return Ok(()),
    }

    let start = Instant::now();
    let outcome = match run_caller(ctx, &run).await {
        Ok(caller) => {
            execute_case_inner(
                ctx,
                &caller,
                result.id,
                result.eval_case_id,
                &result.target,
                result.session_id,
                run.model_override.as_deref(),
            )
            .await
        }
        Err(error) => Err(CaseError::Failed(error)),
    };
    let latency_ms = start.elapsed().as_millis() as i64;

    match outcome {
        Ok(_) => {
            ctx.db
                .update_eval_case_result(
                    result.id,
                    UpdateEvalCaseResultRow {
                        latency_ms: Some(latency_ms),
                        ..Default::default()
                    },
                )
                .await?;
        }
        Err(CaseError::Cancelled) => {}
        Err(CaseError::Timeout) => {
            record_case_error(
                ctx,
                result.id,
                "timeout",
                "Case timeout exceeded",
                latency_ms,
            )
            .await?;
        }
        Err(CaseError::Failed(e)) => {
            record_case_error(ctx, result.id, "errored", &e.to_string(), latency_ms).await?;
        }
    }
    Ok(())
}

/// The role recorded for a run started by a credential without a person;
/// `None` when a person started it (their live membership is used instead).
pub(crate) fn started_by_role(caller: &everruns_core::Caller) -> Option<String> {
    caller.user_id.is_none().then(|| caller.role.to_string())
}

/// Whose authority a run's cases use.
///
/// A run a person started runs as that person, resolved again for every case
/// so a member removed mid-run stops getting sessions. A run started by a
/// credential without a person runs with the role it had, never as the
/// internal caller. Only runs created before this was recorded fall back to
/// the internal caller.
async fn run_caller(
    ctx: &EvalRunContext,
    run: &crate::storage::EvalRunRow,
) -> Result<everruns_core::Caller> {
    if let Some(user_id) = run.started_by_user_id {
        return crate::auth::caller_resolution::caller_for_user(&ctx.db, run.org_id, user_id)
            .await
            .map_err(|error| {
                anyhow::anyhow!("the person who started this run can no longer run it: {error}")
            });
    }
    let Some(role) = run.started_by_role.as_deref() else {
        return Ok(everruns_core::Caller::internal(run.org_id));
    };
    Ok(everruns_core::Caller {
        org_id: run.org_id,
        org_public_id: everruns_core::org_public_id_from_internal(run.org_id),
        user_id: None,
        role: role.parse().unwrap_or(everruns_core::OrgRole::Member),
        is_platform_user: false,
        is_internal: false,
        read_only: false,
    })
}

async fn record_case_error(
    ctx: &EvalRunContext,
    result_id: Uuid,
    status: &str,
    message: &str,
    latency_ms: i64,
) -> Result<()> {
    ctx.db
        .update_eval_case_result(
            result_id,
            UpdateEvalCaseResultRow {
                status: Some(status.to_string()),
                error_message: Some(message.to_string()),
                latency_ms: Some(latency_ms),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}

/// Finish a running run when none of its results is still pending or running.
pub async fn finalize_run_if_complete(ctx: &EvalRunContext, run_id: Uuid) -> Result<bool> {
    let Some(run) = ctx.db.get_eval_run_by_id(run_id).await? else {
        return Ok(false);
    };
    if run.status != "running" {
        return Ok(false);
    }
    let results = ctx.db.list_eval_case_results(run_id).await?;
    if results
        .iter()
        .any(|r| matches!(r.status.as_str(), "pending" | "running"))
    {
        return Ok(false);
    }
    let cases = ctx.db.list_eval_cases(run.eval_id).await?;
    let summary = crate::domains::evals::summary::build_run_summary(
        &cases,
        &results,
        run.trials.max(1) as u32,
    );
    let completed = ctx
        .db
        .complete_eval_run_once(run_id, serde_json::to_value(&summary)?)
        .await?;
    if completed {
        tracing::info!(
            run_id = %run_id,
            total = summary.total,
            passed = summary.passed,
            pass_rate = summary.pass_rate,
            "Eval run completed"
        );
    }
    Ok(completed)
}

/// Why a case stopped without a score.
enum CaseError {
    /// The run was cancelled; the result is already marked skipped.
    Cancelled,
    /// The case ran past its timeout.
    Timeout,
    Failed(anyhow::Error),
}

impl<E: Into<anyhow::Error>> From<E> for CaseError {
    fn from(e: E) -> Self {
        CaseError::Failed(e.into())
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_case_inner(
    ctx: &EvalRunContext,
    caller: &everruns_core::Caller,
    result_id: Uuid,
    case_id: Uuid,
    target_json: &Option<serde_json::Value>,
    existing_session: Option<Uuid>,
    run_model_override: Option<&str>,
) -> std::result::Result<(), CaseError> {
    let org_id = caller.org_id;
    let case_row = ctx
        .db
        .get_eval_case(case_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("eval case {case_id} not found"))?;

    let conversation: Vec<EvalInputMessage> = serde_json::from_value(case_row.conversation)?;
    let post: Option<Vec<EvalInputMessage>> =
        case_row.post.map(serde_json::from_value).transpose()?;
    let artifact_specs: Option<Vec<ArtifactSpec>> =
        case_row.artifacts.map(serde_json::from_value).transpose()?;
    let scorers: Vec<Scorer> = serde_json::from_value(case_row.scorers)?;
    let timeout_secs = case_row
        .timeout_seconds
        .map(|v| v.max(1) as u64)
        .unwrap_or(DEFAULT_CASE_TIMEOUT_SECS);
    let case_deadline = Instant::now() + Duration::from_secs(timeout_secs).min(MAX_CASE_DURATION);

    let target: EvalTarget = target_json
        .as_ref()
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("no target for case result {result_id}"))?;

    let (harness_id, agent_id, model_id, system_prompt, max_iterations) = match &target {
        EvalTarget::Session {
            harness_id,
            harness_name,
            agent_id,
            model_id,
            system_prompt,
            max_iterations,
        } => {
            let hid = if let Some(hid) = harness_id {
                hid.uuid()
            } else if let Some(name) = harness_name {
                let h = ctx
                    .db
                    .get_harness_by_name(org_id, name)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("harness '{name}' not found"))?;
                h.id.uuid()
            } else {
                return Err(
                    anyhow::anyhow!("eval target must specify harness_id or harness_name").into(),
                );
            };
            (
                hid,
                agent_id.as_ref().map(|a| a.uuid()),
                // The run's model override beats the target's model.
                run_model_override
                    .map(str::to_string)
                    .or_else(|| model_id.clone()),
                system_prompt.clone(),
                *max_iterations,
            )
        }
        EvalTarget::App { .. } => {
            return Err(anyhow::anyhow!("App targets are not supported in eval execution").into());
        }
        EvalTarget::External { .. } => {
            return Err(anyhow::anyhow!(
                "External targets are imported results and are not executable"
            )
            .into());
        }
    };

    // A reclaimed task resumes its session; a fresh one creates it.
    let session_uuid = match existing_session {
        Some(id) => id,
        None => {
            let session = ctx
                .session_service
                .create(
                    caller,
                    harness_id,
                    agent_id,
                    agent_id.map(everruns_contracts::typed_id::AgentId::from_uuid),
                    crate::domains::sessions::record::SessionSource::Eval,
                    CreateSessionRequest {
                        playground_user_id: None,
                        source: None,
                        workspace_id: None,
                        harness_id: None, // Already resolved
                        harness_name: None,
                        agent_id: None,
                        agent_name: None,
                        virtual_user_id: None,
                        title: Some(format!("Eval: {}", case_row.name)),
                        goal: None,
                        locale: None,
                        tags: vec!["eval".to_string()],
                        model_id: model_id.and_then(|m| m.parse().ok()),
                        capabilities: vec![],
                        sandbox: None,
                        tools: vec![],
                        mcp_servers: Default::default(),
                        system_prompt,
                        initial_files: vec![],
                        hints: None,
                        network_access: None,
                        max_iterations,
                        parallel_tool_calls: None,
                        parent_session_id: None,
                        forked_from_session_id: None,
                        budget_root_session_id: None,
                        seed: everruns_core::SessionSeedMode::Fresh,
                        fork_up_to_sequence: None,
                    },
                )
                .await?;
            let session_uuid = session.id.uuid();
            ctx.db
                .update_eval_case_result(
                    result_id,
                    UpdateEvalCaseResultRow {
                        session_id: Some(session_uuid),
                        ..Default::default()
                    },
                )
                .await?;
            session_uuid
        }
    };
    let session_id = SessionId::from_uuid(session_uuid);

    let sctx = SessionCtx {
        org_id,
        user_id: caller.user_id,
        result_id,
        session_uuid,
        session_id,
        harness_id,
        agent_id,
        case_deadline,
    };

    let messages: Vec<&EvalInputMessage> =
        conversation.iter().chain(post.iter().flatten()).collect();
    let turn_count = messages.len() as u32;

    // Resume point: messages already delivered, and wait out the turn of the
    // last one if it had not settled when the previous owner stopped.
    let sent = count_events_of(ctx, session_id, &[INPUT_MESSAGE]).await?;
    if sent > 0 {
        wait_for_settled_turns(ctx, &sctx, sent).await?;
    }
    for (index, msg) in messages.iter().enumerate().skip(sent) {
        ensure_run_active(ctx, result_id).await?;
        send_message(ctx, &sctx, &msg.content).await?;
        wait_for_settled_turns(ctx, &sctx, index + 1).await?;
    }
    ensure_run_active(ctx, result_id).await?;

    // Collect events for scoring using filtered queries to avoid 10k row cap
    // What the agent said: its text, or the messages it sent in explicit
    // communication.
    let msg_filter = vec![
        "output.message.completed".to_string(),
        "conversation.message".to_string(),
    ];
    let msg_events = ctx
        .db
        .list_events(session_id, None, None, &msg_filter, &[], None, None)
        .await?;

    let tool_filter = vec!["tool.completed".to_string()];
    let tool_events = ctx
        .db
        .list_events(session_id, None, None, &tool_filter, &[], None, None)
        .await?;

    let turn_filter = vec![TURN_COMPLETED.to_string()];
    let turn_events = ctx
        .db
        .list_events(session_id, None, None, &turn_filter, &[], None, None)
        .await?;

    // Extract final assistant message content
    let final_assistant_content = extract_final_assistant_content(&msg_events);

    // Extract the final message's citation annotations (for citation scorers)
    let final_annotations = extract_final_assistant_annotations(&msg_events);

    // Extract tool calls
    let tool_calls = extract_tool_calls(&tool_events);

    // Extract token usage
    let (input_tokens, output_tokens) = extract_token_usage(&turn_events);

    // Run scorers
    let input_message = messages.last().map(|m| m.content.as_str()).unwrap_or("");
    let grade_input = GradeInput {
        input_message,
        final_answer: &final_assistant_content,
        annotations: &final_annotations,
        tool_calls: &tool_calls,
        turns: turn_count,
        tokens: input_tokens + output_tokens,
        session_id: Some(session_uuid),
    };
    let (scores, grader_usage) = run_scorers(&ctx.graders, org_id, &scorers, &grade_input).await;
    let all_passed = scores.iter().all(|s| s.pass);

    // Calculate weighted average score
    let artifacts = collect_case_artifacts(&ctx.db, session_uuid, artifact_specs.as_deref()).await;

    let status = if all_passed { "passed" } else { "failed" };

    ctx.db
        .update_eval_case_result(
            result_id,
            UpdateEvalCaseResultRow {
                status: Some(status.to_string()),
                scores: Some(serde_json::to_value(&scores)?),
                // What grading cost, so judge and decision spend is visible
                // per case.
                metadata: grader_usage.map(|usage| serde_json::json!({ "grader_usage": usage })),
                turns: Some(turn_count as i32),
                latency_ms: None, // Set by caller
                input_tokens: Some(input_tokens as i64),
                output_tokens: Some(output_tokens as i64),
                artifacts: artifacts.map(serde_json::to_value).transpose()?,
                ..Default::default()
            },
        )
        .await?;

    Ok(())
}

struct SessionCtx {
    org_id: i64,
    user_id: Option<Uuid>,
    result_id: Uuid,
    session_uuid: Uuid,
    session_id: SessionId,
    harness_id: Uuid,
    agent_id: Option<Uuid>,
    case_deadline: Instant,
}

async fn count_events_of(
    ctx: &EvalRunContext,
    session_id: SessionId,
    types: &[&str],
) -> Result<usize> {
    let filter: Vec<String> = types.iter().map(|t| t.to_string()).collect();
    Ok(ctx
        .db
        .list_events(session_id, None, None, &filter, &[], None, None)
        .await?
        .len())
}

/// Stop when the run was cancelled under this case (its result is skipped).
async fn ensure_run_active(
    ctx: &EvalRunContext,
    result_id: Uuid,
) -> std::result::Result<(), CaseError> {
    match ctx.db.get_eval_case_result(result_id).await? {
        Some(result) if result.status == "running" => Ok(()),
        _ => Err(CaseError::Cancelled),
    }
}

async fn send_message(ctx: &EvalRunContext, sctx: &SessionCtx, content: &str) -> Result<()> {
    let msg_ctx = CreateMessageContext {
        runtime_subject_principal_id: None,
        org_id: sctx.org_id,
        user_id: sctx.user_id,
        harness_id: sctx.harness_id,
        agent_id: sctx.agent_id,
        session_id: sctx.session_uuid,
        event_metadata: None,
        request_id: None,
    };
    let msg_req = CreateMessageRequest {
        message: InputMessage {
            role: crate::domains::messages::types::MessageRole::User,
            content: vec![everruns_core::InputContentPart::text(content)],
        },
        addressed_participant_id: None,
        controls: Some(everruns_core::Controls {
            hints: Some(std::collections::HashMap::from([(
                "output_streaming".to_string(),
                serde_json::json!(false),
            )])),
            ..Default::default()
        }),
        metadata: None,
        tags: None,
        external_actor: None,
        client_message_id: None,
    };
    ctx.message_service.create(msg_ctx, msg_req).await?;
    Ok(())
}

/// Wait until the session has `expected` settled turns (completed, failed or
/// cancelled), one per user message. The `expected`-th turn decides: failed
/// or cancelled stops the case.
async fn wait_for_settled_turns(
    ctx: &EvalRunContext,
    sctx: &SessionCtx,
    expected: usize,
) -> std::result::Result<(), CaseError> {
    let filter = vec![
        TURN_COMPLETED.to_string(),
        TURN_FAILED.to_string(),
        TURN_CANCELLED.to_string(),
    ];
    let mut polls = 0u32;
    loop {
        // A cancelled run stops its cases within a few seconds.
        if polls % 5 == 4 {
            ensure_run_active(ctx, sctx.result_id).await?;
        }
        polls += 1;
        let settled = ctx
            .db
            .list_events(sctx.session_id, None, None, &filter, &[], None, None)
            .await?;
        if settled.len() >= expected {
            let event = &settled[expected - 1];
            return match event.event_type.as_str() {
                TURN_COMPLETED => Ok(()),
                TURN_CANCELLED => Err(anyhow::anyhow!("Turn was cancelled").into()),
                _ => {
                    let error_msg = event
                        .data
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .or_else(|| event.data.get("message").and_then(|m| m.as_str()))
                        .unwrap_or("turn failed");
                    Err(anyhow::anyhow!("Turn failed: {error_msg}").into())
                }
            };
        }
        if Instant::now() >= sctx.case_deadline {
            return Err(CaseError::Timeout);
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn collect_case_artifacts(
    db: &StorageBackend,
    session_uuid: Uuid,
    specs: Option<&[ArtifactSpec]>,
) -> Option<BTreeMap<String, String>> {
    let mut artifacts = BTreeMap::new();

    for spec in specs.unwrap_or(&[]) {
        match db.get_session_file(session_uuid, &spec.path).await {
            Ok(Some(file)) if !file.is_directory => {
                let content = file
                    .content
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .unwrap_or_default();
                artifacts.insert(spec.name.clone(), content);
            }
            Ok(Some(_)) => {
                tracing::warn!(
                    session_id = %session_uuid,
                    path = %spec.path,
                    "Skipping directory artifact path"
                );
            }
            Ok(None) => {
                tracing::warn!(
                    session_id = %session_uuid,
                    path = %spec.path,
                    "Configured eval artifact not found"
                );
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session_uuid,
                    path = %spec.path,
                    error = %error,
                    "Failed to read eval artifact"
                );
            }
        }
    }

    (!artifacts.is_empty()).then_some(artifacts)
}

pub(crate) fn extract_final_assistant_content(events: &[crate::storage::EventRow]) -> String {
    // The last thing the agent said; commentary and tool-only messages are not answers.
    events
        .iter()
        .rev()
        .find_map(|event| {
            everruns_core::conversation::said_in_event(&event.event_type, &event.data)
        })
        .unwrap_or_default()
}

/// Collect the citation annotations on the final assistant message. They ride
/// inline on the text content parts of the last `output.message.completed`
/// event, so they are already in the fetched events.
pub(crate) fn extract_final_assistant_annotations(
    events: &[crate::storage::EventRow],
) -> Vec<TextAnnotation> {
    for event in events.iter().rev() {
        if event.event_type == "output.message.completed"
            && let Some(message) = event.data.get("message")
            && let Some(content) = message.get("content")
            && let Some(parts) = content.as_array()
        {
            let mut annotations = Vec::new();
            for part in parts {
                if part.get("type").and_then(|t| t.as_str()) == Some("text")
                    && let Some(arr) = part.get("annotations").and_then(|a| a.as_array())
                {
                    for entry in arr {
                        if let Ok(ann) = serde_json::from_value::<TextAnnotation>(entry.clone()) {
                            annotations.push(ann);
                        }
                    }
                }
            }
            return annotations;
        }
    }
    Vec::new()
}

pub(crate) fn extract_tool_calls(events: &[crate::storage::EventRow]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.event_type == "tool.completed")
        .filter_map(|e| {
            e.data
                .get("tool_name")
                .and_then(|n| n.as_str())
                .map(String::from)
        })
        .collect()
}

pub(crate) fn extract_token_usage(events: &[crate::storage::EventRow]) -> (u64, u64) {
    let mut input = 0u64;
    let mut output = 0u64;
    for event in events {
        if event.event_type == "turn.completed"
            && let Some(usage) = event.data.get("usage")
        {
            input += usage
                .get("input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            output += usage
                .get("output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }
    }
    (input, output)
}

/// Grade every scorer of a case. A scorer that cannot run records a failing
/// score with the reason; it never errors the case. Returns the scores in
/// scorer order and the model usage of grading, when any.
async fn run_scorers(
    graders: &Graders,
    org_id: i64,
    scorers: &[Scorer],
    input: &GradeInput<'_>,
) -> (Vec<Score>, Option<GraderUsage>) {
    let mut scores = Vec::with_capacity(scorers.len());
    let mut usage: Option<GraderUsage> = None;
    for scorer in scorers {
        match graders.grade(org_id, scorer, input).await {
            Ok(graded) => {
                if let Some(spent) = graded.usage {
                    usage.get_or_insert_with(GraderUsage::default).add(&spent);
                }
                scores.push(graded.score);
            }
            Err(error) => scores.push(error.into_score()),
        }
    }
    (scores, usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateSessionFileRow, StorageBackend};

    #[tokio::test]
    async fn collect_case_artifacts_reads_named_files() {
        let db = StorageBackend::test_database();
        let session_uuid = db.create_test_session().await.uuid();

        db.create_session_file(CreateSessionFileRow {
            session_id: SessionId::from_uuid(session_uuid),
            path: "/workspace/fix.patch".to_string(),
            content: Some(b"diff --git a/file b/file\n".to_vec()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .unwrap();

        let artifacts = collect_case_artifacts(
            &db,
            session_uuid,
            Some(&[
                ArtifactSpec {
                    name: "patch".to_string(),
                    path: "/workspace/fix.patch".to_string(),
                },
                ArtifactSpec {
                    name: "missing".to_string(),
                    path: "/workspace/missing.txt".to_string(),
                },
            ]),
        )
        .await
        .unwrap();

        assert_eq!(
            artifacts,
            BTreeMap::from([(
                "patch".to_string(),
                "diff --git a/file b/file\n".to_string()
            )])
        );
    }
}
