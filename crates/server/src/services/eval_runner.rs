// Eval run execution — background task that drives eval cases through sessions.
//
// Design: Each eval case creates a real session, sends conversation + post messages,
// waits for turn completion, runs scorers, and records results. Bounded concurrency
// prevents overwhelming the system. This is NOT a durable workflow — it's a fire-and-forget
// background task. If the server crashes mid-run, the run stays "running" and can be
// manually cancelled. Durable execution is a future enhancement.

use crate::api::messages::{CreateMessageRequest, InputMessage};
use crate::api::sessions::CreateSessionRequest;
use crate::services::MessageService;
use crate::services::SessionService;
use crate::services::message::CreateMessageContext;
use crate::storage::StorageBackend;
use crate::storage::models::UpdateEvalCaseResultRow;
use anyhow::Result;
use everruns_core::eval::*;
use everruns_core::events::{TURN_COMPLETED, TURN_FAILED};
use everruns_core::typed_id::SessionId;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;
use uuid::Uuid;

const MAX_CONCURRENCY: usize = 5;
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const MAX_POLL_DURATION: std::time::Duration = std::time::Duration::from_secs(600);

/// Context needed to execute an eval run.
pub struct EvalRunContext {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
}

/// Spawn the eval run execution in the background.
pub fn spawn_eval_run(ctx: Arc<EvalRunContext>, org_id: i64, run_id: Uuid) {
    tokio::spawn(async move {
        if let Err(e) = execute_eval_run(ctx.clone(), org_id, run_id).await {
            tracing::error!(run_id = %run_id, error = %e, "Eval run failed");
            // Mark run as failed so it doesn't stay "running" indefinitely
            if let Err(update_err) = ctx.db.update_eval_run_status(run_id, "failed", None).await {
                tracing::error!(
                    run_id = %run_id, error = %update_err,
                    "Failed to mark eval run as failed"
                );
            }
        }
    });
}

async fn execute_eval_run(ctx: Arc<EvalRunContext>, org_id: i64, run_id: Uuid) -> Result<()> {
    // Mark run as running
    ctx.db
        .update_eval_run_status(run_id, "running", None)
        .await?;

    let case_results = ctx.db.list_eval_case_results(run_id).await?;
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENCY));

    let mut handles = Vec::with_capacity(case_results.len());

    for case_result in case_results {
        let ctx = ctx.clone();
        let sem = semaphore.clone();

        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            execute_single_case(
                ctx,
                org_id,
                case_result.id,
                case_result.eval_case_id,
                &case_result.target,
            )
            .await
        });
        handles.push(handle);
    }

    // Collect all results
    let mut total = 0u32;
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut errored = 0u32;
    let mut total_score = 0.0f64;
    let mut total_turns = 0.0f64;
    let mut total_latency = 0u64;
    let mut total_input_tokens = 0u64;
    let mut total_output_tokens = 0u64;

    for handle in handles {
        total += 1;
        match handle.await {
            Ok(Ok(metrics)) => {
                total_score += metrics.avg_score;
                total_turns += metrics.turns as f64;
                total_latency += metrics.latency_ms;
                total_input_tokens += metrics.input_tokens;
                total_output_tokens += metrics.output_tokens;
                if metrics.passed {
                    passed += 1;
                } else {
                    failed += 1;
                }
            }
            Ok(Err(e)) => {
                tracing::error!(error = %e, "Eval case execution errored");
                errored += 1;
            }
            Err(e) => {
                tracing::error!(error = %e, "Eval case task panicked");
                errored += 1;
            }
        }
    }

    let summary = RunSummary {
        total,
        passed,
        failed,
        errored,
        pass_rate: if total > 0 {
            passed as f64 / total as f64
        } else {
            0.0
        },
        avg_score: if total > 0 {
            total_score / total as f64
        } else {
            0.0
        },
        avg_turns: if total > 0 {
            total_turns / total as f64
        } else {
            0.0
        },
        avg_latency_ms: if total > 0 {
            total_latency / total as u64
        } else {
            0
        },
        total_input_tokens,
        total_output_tokens,
    };

    ctx.db
        .update_eval_run_status(run_id, "completed", Some(serde_json::to_value(&summary)?))
        .await?;

    tracing::info!(
        run_id = %run_id,
        total, passed, failed, errored,
        pass_rate = summary.pass_rate,
        "Eval run completed"
    );

    Ok(())
}

struct CaseMetrics {
    passed: bool,
    avg_score: f64,
    turns: u32,
    latency_ms: u64,
    input_tokens: u64,
    output_tokens: u64,
}

async fn execute_single_case(
    ctx: Arc<EvalRunContext>,
    org_id: i64,
    result_id: Uuid,
    case_id: Uuid,
    target_json: &Option<serde_json::Value>,
) -> Result<CaseMetrics> {
    let start = Instant::now();

    // Mark case as running
    ctx.db
        .update_eval_case_result(
            result_id,
            UpdateEvalCaseResultRow {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .await?;

    let result = execute_case_inner(&ctx, org_id, result_id, case_id, target_json).await;

    let latency_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(mut metrics) => {
            metrics.latency_ms = latency_ms;
            Ok(metrics)
        }
        Err(e) => {
            // Mark case as errored
            ctx.db
                .update_eval_case_result(
                    result_id,
                    UpdateEvalCaseResultRow {
                        status: Some("errored".to_string()),
                        error_message: Some(e.to_string()),
                        latency_ms: Some(latency_ms as i64),
                        ..Default::default()
                    },
                )
                .await?;
            Err(e)
        }
    }
}

async fn execute_case_inner(
    ctx: &EvalRunContext,
    org_id: i64,
    result_id: Uuid,
    case_id: Uuid,
    target_json: &Option<serde_json::Value>,
) -> Result<CaseMetrics> {
    // Load case
    let case_row = ctx
        .db
        .get_eval_case(case_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("eval case {case_id} not found"))?;

    let conversation: Vec<EvalInputMessage> = serde_json::from_value(case_row.conversation)?;
    let post: Option<Vec<EvalInputMessage>> =
        case_row.post.map(serde_json::from_value).transpose()?;
    let scorers: Vec<Scorer> = serde_json::from_value(case_row.scorers)?;
    let timeout_secs = case_row.timeout_seconds.map(|v| v as u64).unwrap_or(120);
    let case_deadline = Instant::now()
        + std::time::Duration::from_secs(timeout_secs.min(MAX_POLL_DURATION.as_secs()));

    // Resolve target
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
                anyhow::bail!("eval target must specify harness_id or harness_name");
            };
            (
                hid,
                agent_id.as_ref().map(|a| a.uuid()),
                model_id.clone(),
                system_prompt.clone(),
                *max_iterations,
            )
        }
        EvalTarget::App { .. } => {
            anyhow::bail!("App targets not yet supported in eval execution");
        }
    };

    // Internal caller bypasses policy checks (eval execution is system-initiated)
    let caller = everruns_core::Caller::internal(org_id);

    // Create session
    let session = ctx
        .session_service
        .create(
            &caller,
            harness_id,
            agent_id,
            agent_id.map(everruns_core::typed_id::AgentId::from_uuid),
            CreateSessionRequest {
                harness_id: None, // Already resolved
                harness_name: None,
                agent_id: None,
                agent_identity_id: None,
                title: Some(format!("Eval: {}", case_row.name)),
                locale: None,
                tags: vec!["eval".to_string()],
                model_id: model_id.and_then(|m| m.parse().ok()),
                capabilities: vec![],
                tools: vec![],
                system_prompt,
                initial_files: vec![],
                hints: None,
                network_access: None,
                max_iterations,
            },
        )
        .await?;

    let session_uuid = session.id.uuid();
    let session_id = session.id;

    // Record session_id on case result
    ctx.db
        .update_eval_case_result(
            result_id,
            UpdateEvalCaseResultRow {
                session_id: Some(session_uuid),
                ..Default::default()
            },
        )
        .await?;

    let sctx = SessionCtx {
        org_id,
        session_uuid,
        session_id,
        harness_id,
        agent_id,
        case_deadline,
    };

    // Send conversation messages
    let mut turn_count = 0u32;
    for msg in &conversation {
        send_message_and_wait(ctx, &sctx, &msg.content).await?;
        turn_count += 1;
    }

    // Send post messages (if any)
    if let Some(post_msgs) = &post {
        for msg in post_msgs {
            send_message_and_wait(ctx, &sctx, &msg.content).await?;
            turn_count += 1;
        }
    }

    // Collect events for scoring using filtered queries to avoid 10k row cap
    let msg_filter = vec!["output.message.completed".to_string()];
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

    // Extract tool calls
    let tool_calls = extract_tool_calls(&tool_events);

    // Extract token usage
    let (input_tokens, output_tokens) = extract_token_usage(&turn_events);

    // Run scorers
    let scores = run_scorers(
        &scorers,
        &final_assistant_content,
        &tool_calls,
        turn_count,
        ctx,
        session_uuid,
    )
    .await;
    let all_passed = scores.iter().all(|s| s.pass);

    // Calculate weighted average score
    let total_weight: f64 = scorers.iter().map(scorer_weight).sum();
    let weighted_sum: f64 = scores
        .iter()
        .zip(scorers.iter())
        .map(|(score, scorer)| score.value * scorer_weight(scorer))
        .sum();
    let avg_score = if total_weight > 0.0 {
        weighted_sum / total_weight
    } else {
        0.0
    };

    let status = if all_passed { "passed" } else { "failed" };

    ctx.db
        .update_eval_case_result(
            result_id,
            UpdateEvalCaseResultRow {
                status: Some(status.to_string()),
                scores: Some(serde_json::to_value(&scores)?),
                turns: Some(turn_count as i32),
                latency_ms: None, // Set by caller
                input_tokens: Some(input_tokens as i64),
                output_tokens: Some(output_tokens as i64),
                ..Default::default()
            },
        )
        .await?;

    Ok(CaseMetrics {
        passed: all_passed,
        avg_score,
        turns: turn_count,
        latency_ms: 0, // Set by caller
        input_tokens,
        output_tokens,
    })
}

struct SessionCtx {
    org_id: i64,
    session_uuid: Uuid,
    session_id: SessionId,
    harness_id: Uuid,
    agent_id: Option<Uuid>,
    case_deadline: Instant,
}

async fn send_message_and_wait(
    ctx: &EvalRunContext,
    sctx: &SessionCtx,
    content: &str,
) -> Result<()> {
    // Get current latest sequence via count (no row materialization)
    let last_seq = ctx.db.count_events(sctx.session_id, &[]).await.unwrap_or(0) as i32;

    // Send message
    let msg_ctx = CreateMessageContext {
        org_id: sctx.org_id,
        user_id: None,
        harness_id: sctx.harness_id,
        agent_id: sctx.agent_id,
        session_id: sctx.session_uuid,
        event_metadata: None,
        request_id: None,
    };
    let msg_req = CreateMessageRequest {
        message: InputMessage {
            role: crate::api::messages::MessageRole::User,
            content: vec![everruns_core::InputContentPart::text(content)],
        },
        controls: None,
        metadata: None,
        tags: None,
        external_actor: None,
    };
    ctx.message_service.create(msg_ctx, msg_req).await?;

    // Poll for turn completion against the case-level deadline
    loop {
        if Instant::now() >= sctx.case_deadline {
            anyhow::bail!("Case timeout exceeded");
        }

        tokio::time::sleep(POLL_INTERVAL).await;

        let turn_filter = vec![TURN_COMPLETED.to_string(), TURN_FAILED.to_string()];
        let new_events = ctx
            .db
            .list_events(
                sctx.session_id,
                Some(last_seq),
                None,
                &turn_filter,
                &[],
                None,
                None,
            )
            .await?;

        for event in &new_events {
            if event.event_type == TURN_COMPLETED {
                return Ok(());
            }
            if event.event_type == TURN_FAILED {
                let error_msg = event
                    .data
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .or_else(|| event.data.get("message").and_then(|m| m.as_str()))
                    .unwrap_or("turn failed");
                anyhow::bail!("Turn failed: {error_msg}");
            }
        }
    }
}

fn extract_final_assistant_content(events: &[crate::storage::models::EventRow]) -> String {
    // Find the last output.message.completed event.
    // Event data format: { "message": { "content": [{ "type": "text", "text": "..." }] } }
    for event in events.iter().rev() {
        if event.event_type == "output.message.completed"
            && let Some(message) = event.data.get("message")
            && let Some(content) = message.get("content")
            && let Some(parts) = content.as_array()
        {
            let text_parts: Vec<&str> = parts
                .iter()
                .filter_map(|p| {
                    if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                        p.get("text").and_then(|t| t.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            return text_parts.join("\n");
        }
    }
    String::new()
}

fn extract_tool_calls(events: &[crate::storage::models::EventRow]) -> Vec<String> {
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

fn extract_token_usage(events: &[crate::storage::models::EventRow]) -> (u64, u64) {
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

fn scorer_weight(scorer: &Scorer) -> f64 {
    match scorer {
        Scorer::Contains { weight, .. } => *weight,
        Scorer::NotContains { weight, .. } => *weight,
        Scorer::Regex { weight, .. } => *weight,
        Scorer::ToolCalled { weight, .. } => *weight,
        Scorer::ToolNotCalled { weight, .. } => *weight,
        Scorer::ToolCallCount { weight, .. } => *weight,
        Scorer::TurnsWithin { weight, .. } => *weight,
        Scorer::FileContains { weight, .. } => *weight,
        Scorer::JsonSchema { weight, .. } => *weight,
    }
}

async fn run_scorers(
    scorers: &[Scorer],
    final_content: &str,
    tool_calls: &[String],
    turns: u32,
    ctx: &EvalRunContext,
    session_uuid: Uuid,
) -> Vec<Score> {
    let mut scores = Vec::with_capacity(scorers.len());
    for scorer in scorers {
        let score =
            run_single_scorer(scorer, final_content, tool_calls, turns, ctx, session_uuid).await;
        scores.push(score);
    }
    scores
}

async fn run_single_scorer(
    scorer: &Scorer,
    final_content: &str,
    tool_calls: &[String],
    turns: u32,
    ctx: &EvalRunContext,
    session_uuid: Uuid,
) -> Score {
    match scorer {
        Scorer::Contains { text, .. } => {
            let pass = final_content.contains(text.as_str());
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: if pass {
                    format!("Output contains '{text}'")
                } else {
                    format!("Output does not contain '{text}'")
                },
            }
        }
        Scorer::NotContains { text, .. } => {
            let pass = !final_content.contains(text.as_str());
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: if pass {
                    format!("Output does not contain '{text}'")
                } else {
                    format!("Output contains '{text}'")
                },
            }
        }
        Scorer::Regex { pattern, .. } => {
            let pass = regex::Regex::new(pattern)
                .map(|re| re.is_match(final_content))
                .unwrap_or(false);
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: if pass {
                    format!("Output matches pattern '{pattern}'")
                } else {
                    format!("Output does not match pattern '{pattern}'")
                },
            }
        }
        Scorer::ToolCalled { tool, min, .. } => {
            let count = tool_calls.iter().filter(|t| t == &tool).count() as u32;
            let pass = count >= *min;
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: format!("Tool '{tool}' called {count} times (min: {min})"),
            }
        }
        Scorer::ToolNotCalled { tool, .. } => {
            let called = tool_calls.iter().any(|t| t == tool);
            let pass = !called;
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: if pass {
                    format!("Tool '{tool}' was not called")
                } else {
                    format!("Tool '{tool}' was called")
                },
            }
        }
        Scorer::ToolCallCount { min, max, .. } => {
            let count = tool_calls.len() as u32;
            let pass_min = min.map(|m| count >= m).unwrap_or(true);
            let pass_max = max.map(|m| count <= m).unwrap_or(true);
            let pass = pass_min && pass_max;
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: format!(
                    "Total tool calls: {count} (min: {}, max: {})",
                    min.map(|v| v.to_string()).unwrap_or("-".into()),
                    max.map(|v| v.to_string()).unwrap_or("-".into())
                ),
            }
        }
        Scorer::TurnsWithin { max, .. } => {
            let pass = turns <= *max;
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: format!("Turns: {turns} (max: {max})"),
            }
        }
        Scorer::FileContains { path, text, .. } => {
            let file = ctx.db.get_session_file(session_uuid, path).await;
            let pass = match file {
                Ok(Some(f)) => {
                    let content_str = f
                        .content
                        .as_deref()
                        .and_then(|bytes| std::str::from_utf8(bytes).ok())
                        .unwrap_or("");
                    content_str.contains(text.as_str())
                }
                _ => false,
            };
            Score {
                pass,
                value: if pass { 1.0 } else { 0.0 },
                reason: if pass {
                    format!("File '{path}' contains '{text}'")
                } else {
                    format!("File '{path}' does not contain '{text}' or not found")
                },
            }
        }
        Scorer::JsonSchema { schema: _, .. } => {
            // JSON schema validation requires a jsonschema crate dependency.
            // For now, verify the output is valid JSON.
            let is_json = serde_json::from_str::<serde_json::Value>(final_content).is_ok();
            Score {
                pass: is_json,
                value: if is_json { 1.0 } else { 0.0 },
                reason: if is_json {
                    "Output is valid JSON (full schema validation not yet implemented)".to_string()
                } else {
                    "Output is not valid JSON".to_string()
                },
            }
        }
    }
}
