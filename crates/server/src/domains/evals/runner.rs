// Eval run execution — background task that drives eval cases through sessions.
//
// Design: Each eval case creates a real session, sends conversation + post messages,
// waits for turn completion, runs scorers, and records results. Bounded concurrency
// prevents overwhelming the system. This is NOT a durable workflow — it's a fire-and-forget
// background task. If the server crashes mid-run, the run stays "running" and can be
// manually cancelled. Durable execution is a future enhancement.

use crate::api::messages::{CreateMessageRequest, InputMessage};
use crate::api::sessions::CreateSessionRequest;
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::storage::StorageBackend;
use crate::storage::models::UpdateEvalCaseResultRow;
use anyhow::Result;
use everruns_core::events::{TURN_COMPLETED, TURN_FAILED};
use everruns_core::message::{TextAnnotation, VerificationStatus};
use everruns_platform::eval::*;
use everruns_provider::typed_id::SessionId;
use std::collections::BTreeMap;
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
    /// LLM judge for judged scorers (e.g. `citation_judged`). `None` when the
    /// deployment has no model provider stack; judged scorers then report the
    /// judge as unavailable rather than erroring the run.
    pub judge: Option<Arc<dyn crate::domains::observers::JudgeClient>>,
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
    let artifact_specs: Option<Vec<ArtifactSpec>> =
        case_row.artifacts.map(serde_json::from_value).transpose()?;
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
        EvalTarget::External { .. } => {
            anyhow::bail!("External targets are imported results and are not executable");
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
            agent_id.map(everruns_provider::typed_id::AgentId::from_uuid),
            everruns_platform::SessionSource::Eval,
            CreateSessionRequest {
                source: None,
                workspace_id: None,
                harness_id: None, // Already resolved
                harness_name: None,
                agent_id: None,
                agent_name: None,
                agent_identity_id: None,
                title: Some(format!("Eval: {}", case_row.name)),
                goal: None,
                locale: None,
                tags: vec!["eval".to_string()],
                model_id: model_id.and_then(|m| m.parse().ok()),
                capabilities: vec![],
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

    // Extract the final message's citation annotations (for citation scorers)
    let final_annotations = extract_final_assistant_annotations(&msg_events);

    // Extract tool calls
    let tool_calls = extract_tool_calls(&tool_events);

    // Extract token usage
    let (input_tokens, output_tokens) = extract_token_usage(&turn_events);

    // Run scorers
    let scores = run_scorers(
        &scorers,
        &final_assistant_content,
        &final_annotations,
        &tool_calls,
        turn_count,
        ctx,
        org_id,
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
    let artifacts = collect_case_artifacts(&ctx.db, session_uuid, artifact_specs.as_deref()).await;

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
                artifacts: artifacts.map(serde_json::to_value).transpose()?,
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
        addressed_participant_id: None,
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

pub(crate) fn extract_final_assistant_content(
    events: &[crate::storage::models::EventRow],
) -> String {
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

/// Collect the citation annotations on the final assistant message. They ride
/// inline on the text content parts of the last `output.message.completed`
/// event (see `knowledge/runtime-resources/citations.md`), so they are already in the fetched events.
pub(crate) fn extract_final_assistant_annotations(
    events: &[crate::storage::models::EventRow],
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

pub(crate) fn extract_tool_calls(events: &[crate::storage::models::EventRow]) -> Vec<String> {
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
        Scorer::CitationFaithful { weight, .. } => *weight,
        Scorer::CitationJudged { weight, .. } => *weight,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_scorers(
    scorers: &[Scorer],
    final_content: &str,
    annotations: &[TextAnnotation],
    tool_calls: &[String],
    turns: u32,
    ctx: &EvalRunContext,
    org_id: i64,
    session_uuid: Uuid,
) -> Vec<Score> {
    let mut scores = Vec::with_capacity(scorers.len());
    for scorer in scorers {
        let score = run_single_scorer(
            scorer,
            final_content,
            annotations,
            tool_calls,
            turns,
            ctx,
            org_id,
            session_uuid,
        )
        .await;
        scores.push(score);
    }
    scores
}

#[allow(clippy::too_many_arguments)]
async fn run_single_scorer(
    scorer: &Scorer,
    final_content: &str,
    annotations: &[TextAnnotation],
    tool_calls: &[String],
    turns: u32,
    ctx: &EvalRunContext,
    org_id: i64,
    session_uuid: Uuid,
) -> Score {
    if let Some(score) =
        crate::domains::evals::scoring::score_rule(scorer, final_content, tool_calls, turns)
    {
        return score;
    }
    match scorer {
        Scorer::CitationFaithful {
            min_citations,
            pass_threshold,
            ..
        } => score_citation_faithful(annotations, *min_citations, *pass_threshold),
        Scorer::CitationJudged {
            rubric,
            model_id,
            pass_threshold,
            ..
        } => {
            score_citation_judged(
                ctx,
                org_id,
                final_content,
                annotations,
                rubric.as_deref(),
                *model_id,
                *pass_threshold,
            )
            .await
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
        // score_rule covers every other variant.
        _ => unreachable!("score_rule handles all non-file scorer rules"),
    }
}

/// Grade citation faithfulness from the answer's annotations: coverage (enough
/// citations) plus faithfulness (fraction verified `entailed`). Verdicts are
/// stamped by the `citation_verification` capability; without it, citations
/// are unverified and score 0 — the reason string says so.
fn score_citation_faithful(
    annotations: &[TextAnnotation],
    min_citations: u32,
    pass_threshold: f64,
) -> Score {
    let total = annotations.len();
    if total < min_citations as usize {
        return Score {
            pass: false,
            value: 0.0,
            reason: format!("{total} citations (min {min_citations})"),
        };
    }
    if total == 0 {
        // min_citations is 0 here, so coverage is satisfied vacuously.
        return Score {
            pass: true,
            value: 1.0,
            reason: "no citations required or present".to_string(),
        };
    }
    let with_verdict = annotations.iter().filter(|a| a.verified.is_some()).count();
    let entailed = annotations
        .iter()
        .filter(|a| {
            a.verified
                .as_ref()
                .is_some_and(|v| v.status == VerificationStatus::Entailed)
        })
        .count();
    let fraction = entailed as f64 / total as f64;
    let pass = fraction >= pass_threshold;
    let reason = if with_verdict == 0 {
        format!(
            "{entailed}/{total} citations entailed — none verified (enable citation_verification)"
        )
    } else {
        format!(
            "{entailed}/{total} citations entailed ({with_verdict} verified); \
             faithfulness {fraction:.2} (threshold {pass_threshold:.2})"
        )
    };
    Score {
        pass,
        value: fraction,
        reason,
    }
}

const CITATION_JUDGE_RUBRIC: &str = "You are judging citation faithfulness. Each numbered item is a CLAIM the agent \
     made and the SOURCE it cited. Judge whether each SOURCE actually supports its \
     CLAIM. `value` is the fraction of items whose source supports the claim \
     (1.0 = all supported, 0.0 = none). Penalize claims whose source is unrelated \
     or contradicts them.";

/// Grade citation faithfulness with the org's LLM judge, reusing the observer
/// judge (`observers::judge::JudgeClient`). Fails open to a clear reason when no
/// judge/model is available so a judge outage never errors the whole run.
async fn score_citation_judged(
    ctx: &EvalRunContext,
    org_id: i64,
    final_content: &str,
    annotations: &[TextAnnotation],
    rubric: Option<&str>,
    model_id: Option<everruns_provider::typed_id::ModelId>,
    pass_threshold: f64,
) -> Score {
    if annotations.is_empty() {
        return Score {
            pass: true,
            value: 1.0,
            reason: "no citations to judge".to_string(),
        };
    }
    let Some(judge) = ctx.judge.as_ref() else {
        return Score {
            pass: false,
            value: 0.0,
            reason: "citation judge unavailable (no model provider)".to_string(),
        };
    };
    let evidence = crate::domains::observers::TurnEvidence {
        input_message: String::new(),
        final_answer: build_citation_pairs(final_content, annotations),
        tool_names: Vec::new(),
    };
    let rubric = rubric.unwrap_or(CITATION_JUDGE_RUBRIC);
    match judge.judge(org_id, model_id, rubric, &evidence).await {
        Ok(Some(result)) => Score {
            pass: result.value >= pass_threshold,
            value: result.value,
            reason: result.reasoning,
        },
        Ok(None) => Score {
            pass: false,
            value: 0.0,
            reason: "no judge model configured for org".to_string(),
        },
        Err(e) => Score {
            pass: false,
            value: 0.0,
            reason: format!("citation judge error: {e}"),
        },
    }
}

/// Render each annotation as a numbered CLAIM/SOURCE pair for the judge. The
/// claim is the answer span the citation is attached to; the source is the
/// cited snippet (and uri).
fn build_citation_pairs(text: &str, annotations: &[TextAnnotation]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    for (i, ann) in annotations.iter().enumerate() {
        let claim: String = chars
            .get(ann.start..ann.end.min(chars.len()))
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        let source = ann.source.snippet.as_deref().unwrap_or("(no snippet)");
        out.push_str(&format!(
            "[{}] CLAIM: {}\n    SOURCE ({}): {}\n",
            i + 1,
            claim.trim(),
            ann.source.uri,
            source.trim(),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateSessionFileRow, StorageBackend};
    use everruns_core::message::{AnnotationSource, VerificationVerdict};

    fn verified_annotation(status: VerificationStatus) -> TextAnnotation {
        TextAnnotation {
            start: 0,
            end: 5,
            origin: "citation_retrieval".to_string(),
            source: AnnotationSource {
                uri: "u".to_string(),
                title: None,
                snippet: None,
                location: None,
            },
            external_id: None,
            verified: Some(VerificationVerdict {
                status,
                score: Some(0.9),
            }),
        }
    }

    #[test]
    fn citation_faithful_passes_when_all_entailed() {
        let anns = vec![
            verified_annotation(VerificationStatus::Entailed),
            verified_annotation(VerificationStatus::Entailed),
        ];
        let score = score_citation_faithful(&anns, 1, 0.8);
        assert!(score.pass);
        assert!((score.value - 1.0).abs() < 1e-9);
    }

    #[test]
    fn citation_faithful_fails_below_threshold() {
        let anns = vec![
            verified_annotation(VerificationStatus::Entailed),
            verified_annotation(VerificationStatus::Unsupported),
        ];
        let score = score_citation_faithful(&anns, 1, 0.8);
        assert!(!score.pass);
        assert!((score.value - 0.5).abs() < 1e-9);
    }

    #[test]
    fn citation_faithful_fails_on_insufficient_coverage() {
        let anns = vec![verified_annotation(VerificationStatus::Entailed)];
        let score = score_citation_faithful(&anns, 3, 0.5);
        assert!(!score.pass);
        assert_eq!(score.value, 0.0);
    }

    #[test]
    fn citation_faithful_notes_missing_verdicts() {
        let mut ann = verified_annotation(VerificationStatus::Entailed);
        ann.verified = None;
        let score = score_citation_faithful(&[ann], 1, 0.8);
        assert!(!score.pass);
        assert!(score.reason.contains("enable citation_verification"));
    }

    #[test]
    fn citation_pairs_render_claim_and_source() {
        let text = "The sky is blue. Grass is green.";
        let anns = vec![TextAnnotation {
            start: 0,
            end: 16,
            origin: "citation_retrieval".to_string(),
            source: AnnotationSource {
                uri: "doc://a".to_string(),
                title: None,
                snippet: Some("The sky appears blue due to Rayleigh scattering.".to_string()),
                location: None,
            },
            external_id: None,
            verified: None,
        }];
        let pairs = build_citation_pairs(text, &anns);
        assert!(pairs.contains("[1] CLAIM: The sky is blue."));
        assert!(pairs.contains("SOURCE (doc://a): The sky appears blue"));
    }

    #[tokio::test]
    async fn collect_case_artifacts_reads_named_files() {
        let db = StorageBackend::in_memory();
        let session_uuid = Uuid::now_v7();

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
