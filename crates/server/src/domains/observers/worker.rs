// Observer scoring worker — drains the trace_scores queue and runs scorers.
//
// Dual-mode by construction: the queue (claim_trace_scores) is implemented by
// both the PostgreSQL and in-memory backends, so the same worker loop runs in
// full mode (durable, multi-instance-safe via FOR UPDATE SKIP LOCKED) and in
// dev mode (in-memory, single process). It is woken immediately after enqueue
// and also polls on an interval for catch-up.

use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

use everruns_core::typed_id::SessionId;
use everruns_platform::eval::Score;
use everruns_platform::observer::{ObserverScorerConfig, ScorerMethod};
use tracing::{debug, error, warn};

use crate::domains::evals::runner::{extract_final_assistant_content, extract_tool_calls};
use crate::domains::evals::scoring::score_rule;
use crate::domains::observers::judge::{JudgeClient, TurnEvidence};
use crate::storage::StorageBackend;
use crate::storage::models::{CompleteTraceScoreRow, EventRow, ListEventsParams, TraceScoreRow};

/// Dependencies the worker loop needs to score: storage plus an optional judge
/// client. `judge` is `None` when no LLM path is available (e.g. dev without
/// providers); llm_judge scores are then skipped with a clear reason.
#[derive(Clone)]
pub struct ObserverWorkerDeps {
    pub db: Arc<StorageBackend>,
    pub judge: Option<Arc<dyn JudgeClient>>,
}

/// Worker tuning. Conservative defaults; bounded judge fan-out.
#[derive(Clone, Copy, Debug)]
pub struct ObserverWorkerConfig {
    /// Max scores claimed per drain iteration.
    pub batch_size: i64,
    /// Catch-up poll interval (also heals missed wakeups).
    pub poll_interval: Duration,
    /// A `scoring` row older than this is considered abandoned and reclaimed.
    pub stale_after: Duration,
    /// Max scoring attempts before a score is finalized as errored.
    pub max_attempts: i32,
}

impl Default for ObserverWorkerConfig {
    fn default() -> Self {
        Self {
            batch_size: 32,
            poll_interval: Duration::from_secs(5),
            stale_after: Duration::from_secs(120),
            max_attempts: 3,
        }
    }
}

/// Spawn the background scoring worker. Returns immediately; the loop runs
/// until the process exits.
pub fn spawn_observer_worker(
    deps: ObserverWorkerDeps,
    wake: Arc<tokio::sync::Notify>,
    config: ObserverWorkerConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        tracing::info!("Observer scoring worker started");
        loop {
            // Drain until a claim returns fewer than a full batch.
            loop {
                match drain_once(&deps, config).await {
                    Ok(0) => break,
                    Ok(n) => {
                        debug!(scored = n, "Observer scoring batch processed");
                        if (n as i64) < config.batch_size {
                            break;
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Observer scoring batch failed");
                        break;
                    }
                }
            }
            // Wait for the next wakeup or the catch-up poll.
            tokio::select! {
                _ = wake.notified() => {}
                _ = tokio::time::sleep(config.poll_interval) => {}
            }
        }
    })
}

/// Claim and score one batch. Returns the number of scores processed.
async fn drain_once(
    deps: &ObserverWorkerDeps,
    config: ObserverWorkerConfig,
) -> anyhow::Result<usize> {
    let claimed = deps
        .db
        .claim_trace_scores(
            config.batch_size,
            config.stale_after.as_secs() as i64,
            config.max_attempts,
        )
        .await?;
    let count = claimed.len();
    for score in claimed {
        if let Err(e) = process_score(deps, &score).await {
            // Leave the row in `scoring`; a later claim retries it (or
            // finalizes it errored once attempts are exhausted).
            warn!(score_id = %score.public_id, error = %e, "Failed to process trace score");
        }
    }
    Ok(count)
}

async fn process_score(deps: &ObserverWorkerDeps, score: &TraceScoreRow) -> anyhow::Result<()> {
    let db = &deps.db;
    // Load the observer to recover the scorer config for this key.
    let Some(observer_row) = db.get_observer(score.observer_id).await? else {
        // Observer deleted between enqueue and scoring — skip.
        finalize_skipped(db, score, "observer no longer exists").await?;
        return Ok(());
    };
    let scorers: Vec<ObserverScorerConfig> = serde_json::from_value(observer_row.scorers)?;
    let Some(scorer) = scorers.into_iter().find(|s| s.key == score.scorer_key) else {
        finalize_skipped(db, score, "scorer key no longer configured").await?;
        return Ok(());
    };

    // Load this turn's trace slice.
    let session_id = SessionId::from_uuid(score.session_id);
    let msg_events =
        list_turn_events(db, session_id, &score.turn_id, "output.message.completed").await?;
    let tool_events = list_turn_events(db, session_id, &score.turn_id, "tool.completed").await?;
    let turn_events = list_turn_events(db, session_id, &score.turn_id, "turn.completed").await?;

    let final_content = extract_final_assistant_content(&msg_events);
    let tool_calls = extract_tool_calls(&tool_events);

    let completion = match scorer.method {
        ScorerMethod::Rule { rule } => {
            let iterations = extract_iterations(&turn_events);
            let Some(Score {
                pass,
                value,
                reason,
            }) = score_rule(&rule, &final_content, &tool_calls, iterations)
            else {
                // file_contains is rejected at create time, so this shouldn't
                // happen; mark skipped to avoid a retry loop.
                finalize_skipped(db, score, "scorer rule unsupported for observers").await?;
                return Ok(());
            };
            CompleteTraceScoreRow {
                status: "completed".to_string(),
                pass: Some(pass),
                value: Some(value),
                reason: Some(reason),
                ..Default::default()
            }
        }
        ScorerMethod::LlmJudge(judge_config) => {
            let Some(judge) = deps.judge.clone() else {
                finalize_skipped(db, score, "llm judge unavailable (no model provider)").await?;
                return Ok(());
            };
            let evidence = TurnEvidence {
                input_message: extract_input_content(&turn_events),
                final_answer: final_content,
                tool_names: tool_calls,
            };
            let Some(result) = judge
                .judge(
                    score.org_id,
                    judge_config.model_id,
                    &judge_config.rubric,
                    &evidence,
                )
                .await?
            else {
                // Org has no judge model configured — a permanent config
                // condition, not a transient failure. Skip without retrying.
                finalize_skipped(db, score, "no judge model configured for org").await?;
                return Ok(());
            };
            CompleteTraceScoreRow {
                status: "completed".to_string(),
                pass: Some(result.value >= judge_config.pass_threshold),
                value: Some(result.value),
                label: result.label,
                reason: Some(result.reasoning),
                judge_input_tokens: result.input_tokens.map(|t| t as i64),
                judge_output_tokens: result.output_tokens.map(|t| t as i64),
                judge_cost_usd: result.cost_usd,
                error_message: None,
            }
        }
    };

    db.complete_trace_score(score.id, completion).await?;
    Ok(())
}

async fn finalize_skipped(
    db: &StorageBackend,
    score: &TraceScoreRow,
    reason: &str,
) -> anyhow::Result<()> {
    db.complete_trace_score(
        score.id,
        CompleteTraceScoreRow {
            status: "skipped".to_string(),
            reason: Some(reason.to_string()),
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// Fetch events of one type belonging to a single turn.
async fn list_turn_events(
    db: &StorageBackend,
    session_id: SessionId,
    turn_id: &str,
    event_type: &str,
) -> anyhow::Result<Vec<EventRow>> {
    let params = ListEventsParams {
        session_id,
        turn_id: Some(turn_id.to_string()),
        filter_types: vec![event_type.to_string()],
        ..Default::default()
    };
    db.list_events_advanced(&params).await
}

/// Iteration count from this turn's `turn.completed` event (turn-scope
/// equivalent of "turns" for the TurnsWithin rule).
fn extract_iterations(turn_events: &[EventRow]) -> u32 {
    turn_events
        .iter()
        .rev()
        .find(|e| e.event_type == "turn.completed")
        .and_then(|e| e.data.get("iterations"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32
}

/// The user's input text for this turn, carried on `turn.completed`.
fn extract_input_content(turn_events: &[EventRow]) -> String {
    turn_events
        .iter()
        .rev()
        .find(|e| e.event_type == "turn.completed")
        .and_then(|e| e.data.get("input_content"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::observers::judge::{JudgeResult, TurnEvidence};
    use crate::storage::models::{
        CreateEventRow, CreateObserverRow, CreateSessionRow, CreateTraceScoreRow,
    };
    use everruns_core::typed_id::{
        AgentId, HarnessId, ModelId, ObserverId, PrincipalId, TraceScoreId,
    };
    use everruns_platform::observer::{
        LlmJudgeConfig, ObserverScope, ObserverScorerConfig, ScorerMethod,
    };
    use uuid::Uuid;

    const ORG: i64 = 1;

    fn deps(db: &Arc<StorageBackend>) -> ObserverWorkerDeps {
        ObserverWorkerDeps {
            db: db.clone(),
            judge: None,
        }
    }

    /// Fake judge returning a fixed grade (or `None` for "no model"), for
    /// worker dispatch tests.
    struct FakeJudge {
        value: f64,
        label: Option<String>,
        /// When false, simulates "no judge model configured" (`Ok(None)`).
        has_model: bool,
    }

    #[async_trait::async_trait]
    impl JudgeClient for FakeJudge {
        async fn judge(
            &self,
            _org_id: i64,
            _model_id: Option<ModelId>,
            _rubric: &str,
            _evidence: &TurnEvidence,
        ) -> anyhow::Result<Option<JudgeResult>> {
            if !self.has_model {
                return Ok(None);
            }
            Ok(Some(JudgeResult {
                value: self.value,
                label: self.label.clone(),
                reasoning: "fake".to_string(),
                input_tokens: Some(42),
                output_tokens: Some(7),
                cost_usd: Some(0.001),
            }))
        }
    }

    fn session_row(agent: AgentId, harness: HarnessId, tags: Vec<String>) -> CreateSessionRow {
        CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: ORG,
            app_id: None,
            harness_id: Some(harness),
            agent_id: Some(agent),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            parent_session_id: None,
            budget_root_session_id: None,
            owner_principal_id: PrincipalId::from_uuid(Uuid::nil()),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags,
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
        }
    }

    async fn seed_turn(
        db: &StorageBackend,
        session_id: SessionId,
        turn_id: &str,
        final_text: &str,
    ) {
        let ctx = serde_json::json!({ "turn_id": turn_id });
        db.create_event(CreateEventRow {
            session_id,
            event_type: "output.message.completed".to_string(),
            ts: chrono::Utc::now(),
            context: ctx.clone(),
            data: serde_json::json!({
                "message": { "content": [{ "type": "text", "text": final_text }] }
            }),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
        db.create_event(CreateEventRow {
            session_id,
            event_type: "turn.completed".to_string(),
            ts: chrono::Utc::now(),
            context: ctx,
            data: serde_json::json!({ "turn_id": turn_id, "iterations": 2 }),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }

    fn observer_with_scorer(agent: AgentId, scorer: ObserverScorerConfig) -> CreateObserverRow {
        CreateObserverRow {
            public_id: ObserverId::from_uuid(Uuid::now_v7()).to_string(),
            name: "test".to_string(),
            description: None,
            match_config: serde_json::to_value(everruns_platform::observer::ObserverMatch {
                agent_ids: Some(vec![agent]),
                ..Default::default()
            })
            .unwrap(),
            sampling_rate: 1.0,
            scorers: serde_json::to_value(vec![scorer]).unwrap(),
        }
    }

    fn contains_observer(agent: AgentId, key: &str, text: &str) -> CreateObserverRow {
        observer_with_scorer(
            agent,
            ObserverScorerConfig {
                key: key.to_string(),
                scope: ObserverScope::Turn,
                method: ScorerMethod::Rule {
                    rule: everruns_platform::eval::Scorer::Contains {
                        text: text.to_string(),
                        weight: 1.0,
                    },
                },
            },
        )
    }

    async fn enqueue(
        db: &StorageBackend,
        observer_id: Uuid,
        key: &str,
        session_id: SessionId,
        turn_id: &str,
        agent: AgentId,
    ) {
        db.enqueue_trace_scores(
            ORG,
            &[CreateTraceScoreRow {
                public_id: TraceScoreId::from_uuid(Uuid::now_v7()).to_string(),
                observer_id,
                scorer_key: key.to_string(),
                session_id: session_id.uuid(),
                turn_id: turn_id.to_string(),
                agent_id: Some(agent.uuid()),
                agent_version_id: None,
                harness_id: None,
            }],
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn drains_and_scores_a_matching_turn() {
        let db = Arc::new(StorageBackend::in_memory());
        let agent = AgentId::new();
        let harness = HarnessId::new();
        let turn_id = "turn_01933b5a000070008000000000000abc";

        let session = db
            .create_session(session_row(agent, harness, vec!["prod".into()]))
            .await
            .unwrap();
        seed_turn(&db, session.id, turn_id, "hello world from the agent").await;

        let observer = db
            .create_observer(ORG, contains_observer(agent, "greet", "hello"))
            .await
            .unwrap();
        enqueue(&db, observer.id, "greet", session.id, turn_id, agent).await;

        let processed = drain_once(&deps(&db), ObserverWorkerConfig::default())
            .await
            .unwrap();
        assert_eq!(processed, 1);

        let scores = db
            .list_trace_scores(
                ORG,
                crate::storage::models::ListTraceScoresParams {
                    observer_id: observer.id,
                    session_id: None,
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(scores.len(), 1);
        assert_eq!(scores[0].status, "completed");
        assert_eq!(scores[0].pass, Some(true));
        assert_eq!(scores[0].value, Some(1.0));
    }

    #[tokio::test]
    async fn scores_failing_rule_as_not_passed() {
        let db = Arc::new(StorageBackend::in_memory());
        let agent = AgentId::new();
        let harness = HarnessId::new();
        let turn_id = "turn_01933b5a000070008000000000000def";

        let session = db
            .create_session(session_row(agent, harness, vec![]))
            .await
            .unwrap();
        seed_turn(&db, session.id, turn_id, "goodbye").await;

        let observer = db
            .create_observer(ORG, contains_observer(agent, "greet", "hello"))
            .await
            .unwrap();
        enqueue(&db, observer.id, "greet", session.id, turn_id, agent).await;

        drain_once(&deps(&db), ObserverWorkerConfig::default())
            .await
            .unwrap();

        let scores = db
            .list_trace_scores(
                ORG,
                crate::storage::models::ListTraceScoresParams {
                    observer_id: observer.id,
                    session_id: None,
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(scores[0].status, "completed");
        assert_eq!(scores[0].pass, Some(false));
        assert_eq!(scores[0].value, Some(0.0));
    }

    #[tokio::test]
    async fn skips_score_when_scorer_key_removed() {
        let db = Arc::new(StorageBackend::in_memory());
        let agent = AgentId::new();
        let harness = HarnessId::new();
        let turn_id = "turn_01933b5a000070008000000000000fed";

        let session = db
            .create_session(session_row(agent, harness, vec![]))
            .await
            .unwrap();
        seed_turn(&db, session.id, turn_id, "hello").await;

        let observer = db
            .create_observer(ORG, contains_observer(agent, "greet", "hello"))
            .await
            .unwrap();
        enqueue(&db, observer.id, "greet", session.id, turn_id, agent).await;
        // Remove the scorer config so the worker cannot find the key.
        db.update_observer(
            ORG,
            observer.id,
            crate::storage::models::UpdateObserverRow {
                scorers: Some(serde_json::json!([])),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        drain_once(&deps(&db), ObserverWorkerConfig::default())
            .await
            .unwrap();

        let scores = db
            .list_trace_scores(
                ORG,
                crate::storage::models::ListTraceScoresParams {
                    observer_id: observer.id,
                    session_id: None,
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(scores[0].status, "skipped");
    }

    fn judge_observer(agent: AgentId, key: &str) -> CreateObserverRow {
        observer_with_scorer(
            agent,
            ObserverScorerConfig {
                key: key.to_string(),
                scope: ObserverScope::Turn,
                method: ScorerMethod::LlmJudge(LlmJudgeConfig {
                    rubric: "Did the agent greet the user?".to_string(),
                    model_id: None,
                    pass_threshold: 0.5,
                }),
            },
        )
    }

    async fn run_judge_observer(judge: Option<Arc<dyn JudgeClient>>) -> Vec<TraceScoreRow> {
        let db = Arc::new(StorageBackend::in_memory());
        let agent = AgentId::new();
        let harness = HarnessId::new();
        let turn_id = "turn_01933b5a0000700080000000000000aa";
        let session = db
            .create_session(session_row(agent, harness, vec![]))
            .await
            .unwrap();
        seed_turn(&db, session.id, turn_id, "hello there").await;
        let observer = db
            .create_observer(ORG, judge_observer(agent, "greeted"))
            .await
            .unwrap();
        enqueue(&db, observer.id, "greeted", session.id, turn_id, agent).await;

        let deps = ObserverWorkerDeps {
            db: db.clone(),
            judge,
        };
        drain_once(&deps, ObserverWorkerConfig::default())
            .await
            .unwrap();
        db.list_trace_scores(
            ORG,
            crate::storage::models::ListTraceScoresParams {
                observer_id: observer.id,
                session_id: None,
                limit: 10,
                offset: 0,
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn scores_llm_judge_turn_via_judge_client() {
        let judge = Arc::new(FakeJudge {
            value: 0.9,
            label: Some("greeted".to_string()),
            has_model: true,
        });
        let scores = run_judge_observer(Some(judge)).await;
        assert_eq!(scores.len(), 1);
        assert_eq!(scores[0].status, "completed");
        assert_eq!(scores[0].pass, Some(true));
        assert_eq!(scores[0].value, Some(0.9));
        assert_eq!(scores[0].label.as_deref(), Some("greeted"));
        assert_eq!(scores[0].reason.as_deref(), Some("fake"));
        assert_eq!(scores[0].judge_input_tokens, Some(42));
        assert_eq!(scores[0].judge_output_tokens, Some(7));
    }

    #[tokio::test]
    async fn llm_judge_below_threshold_does_not_pass() {
        let judge = Arc::new(FakeJudge {
            value: 0.2,
            label: None,
            has_model: true,
        });
        let scores = run_judge_observer(Some(judge)).await;
        assert_eq!(scores[0].status, "completed");
        assert_eq!(scores[0].pass, Some(false));
    }

    #[tokio::test]
    async fn llm_judge_skipped_when_no_judge_client() {
        let scores = run_judge_observer(None).await;
        assert_eq!(scores[0].status, "skipped");
    }

    #[tokio::test]
    async fn llm_judge_skipped_when_org_has_no_model() {
        // Judge client present, but the org has no model configured (Ok(None)).
        let judge = Arc::new(FakeJudge {
            value: 0.0,
            label: None,
            has_model: false,
        });
        let scores = run_judge_observer(Some(judge)).await;
        assert_eq!(scores[0].status, "skipped");
        assert_eq!(
            scores[0].reason.as_deref(),
            Some("no judge model configured for org")
        );
    }
}
