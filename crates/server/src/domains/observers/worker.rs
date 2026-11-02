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

use crate::domains::evals::graders::{GradeError, GradeInput, Graders};
use crate::domains::evals::record::Scorer;
use crate::domains::observers::record::{ObserverScorerConfig, ScorerMethod};
use everruns_contracts::typed_id::SessionId;
use tracing::{debug, warn};

use crate::storage::StorageBackend;
use crate::storage::{CompleteTraceScoreRow, TraceScoreRow};

/// Dependencies the worker loop needs to score: storage plus the graders
/// shared with eval runs. Without a model stack (dev without providers) the
/// model-backed scorers are skipped with a clear reason.
#[derive(Clone)]
pub struct ObserverWorkerDeps {
    pub db: Arc<StorageBackend>,
    pub graders: Graders,
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
                        everruns_core::log_database_failure(
                            "server.observers.scoring",
                            "Observer scoring batch failed",
                            &e.to_string(),
                        );
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

    // Load the slice this score grades: a turn, the session or one call.
    let Some(evidence) = super::evidence::gather(db, score).await? else {
        finalize_skipped(db, score, "tool call is not in the trace").await?;
        return Ok(());
    };

    // An llm_judge scorer is the shared `judge` grader with the observer's
    // own pass threshold.
    let rule = match scorer.method {
        ScorerMethod::Rule { rule } => rule,
        ScorerMethod::LlmJudge(judge) => Scorer::Judge {
            rubric: judge.rubric,
            model_id: judge.model_id,
            pass_threshold: judge.pass_threshold,
            weight: 1.0,
        },
    };
    let input = GradeInput {
        input_message: &evidence.input_message,
        final_answer: &evidence.final_answer,
        tool_calls: &evidence.tool_calls,
        tool_arguments: &evidence.tool_arguments,
        transcript: &evidence.transcript,
        turns: evidence.turns,
        tokens: evidence.tokens,
        ..Default::default()
    };
    let graded = match deps.graders.grade(score.org_id, &rule, &input).await {
        Ok(graded) => graded,
        // A lasting condition (no model configured, unsupported rule): skip
        // rather than retry into an error.
        Err(GradeError::Unavailable(reason)) => {
            finalize_skipped(db, score, &reason).await?;
            return Ok(());
        }
        Err(GradeError::Failed(error)) => return Err(error),
    };
    let usage = graded.usage.unwrap_or_default();
    let completion = CompleteTraceScoreRow {
        status: "completed".to_string(),
        pass: Some(graded.score.pass),
        value: Some(graded.score.value),
        label: graded.label,
        reason: Some(graded.score.reason),
        judge_input_tokens: graded.usage.map(|_| usage.input_tokens as i64),
        judge_output_tokens: graded.usage.map(|_| usage.output_tokens as i64),
        judge_cost_usd: usage.cost_usd,
        error_message: None,
    };

    db.complete_trace_score(score.id, completion).await?;
    if let Some(usage) = graded.usage {
        crate::domains::budgets::BudgetService::new(db.clone())
            .meter_observer_grading(
                SessionId::from_uuid(score.session_id),
                score.id,
                usage.input_tokens,
                usage.output_tokens,
                usage.cost_usd,
            )
            .await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::observers::judge::{JudgeClient, JudgeResult, TurnEvidence};
    use crate::domains::observers::record::{
        LlmJudgeConfig, ObserverScope, ObserverScorerConfig, ScorerMethod,
    };
    use crate::storage::{CreateEventRow, CreateSessionRow};
    use crate::storage::{CreateObserverRow, CreateTraceScoreRow};
    use everruns_contracts::typed_id::{
        AgentId, HarnessId, ModelId, ObserverId, PrincipalId, TraceScoreId,
    };
    use uuid::Uuid;

    const ORG: i64 = 1;

    fn deps(db: &Arc<StorageBackend>) -> ObserverWorkerDeps {
        ObserverWorkerDeps {
            db: db.clone(),
            graders: Graders::new(db.clone()),
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
            playground_user_id: None,
            source: crate::domains::sessions::record::SessionSource::Api,
            workspace_id: None,
            org_id: ORG,
            app_id: None,
            channel_id: None,
            trigger_id: None,
            harness_id: Some(harness),
            agent_id: Some(agent),
            agent_revision: None,
            virtual_user_id: None,
            parent_session_id: None,
            budget_root_session_id: None,
            owner_principal_id: PrincipalId::from_seed(1),
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
            match_config: serde_json::to_value(crate::domains::observers::record::ObserverMatch {
                agent_ids: Some(vec![agent]),
                ..Default::default()
            })
            .unwrap(),
            sampling_rate: 1.0,
            scorers: serde_json::to_value(vec![scorer]).unwrap(),
            alert: None,
        }
    }

    fn contains_observer(agent: AgentId, key: &str, text: &str) -> CreateObserverRow {
        observer_with_scorer(
            agent,
            ObserverScorerConfig {
                key: key.to_string(),
                scope: ObserverScope::Turn,
                tools: None,
                method: ScorerMethod::Rule {
                    rule: crate::domains::evals::record::Scorer::Contains {
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
        enqueue_scoped(
            db,
            observer_id,
            key,
            session_id,
            turn_id,
            agent,
            "turn",
            None,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn enqueue_scoped(
        db: &StorageBackend,
        observer_id: Uuid,
        key: &str,
        session_id: SessionId,
        turn_id: &str,
        agent: AgentId,
        scope: &str,
        not_before: Option<chrono::DateTime<chrono::Utc>>,
    ) -> u64 {
        db.enqueue_trace_scores(
            ORG,
            &[CreateTraceScoreRow {
                public_id: TraceScoreId::from_uuid(Uuid::now_v7()).to_string(),
                observer_id,
                scorer_key: key.to_string(),
                session_id: session_id.uuid(),
                turn_id: turn_id.to_string(),
                agent_id: Some(agent.uuid()),
                harness_id: None,
                scope: scope.to_string(),
                not_before,
            }],
        )
        .await
        .unwrap() as u64
    }

    async fn seed_event(
        db: &StorageBackend,
        session_id: SessionId,
        turn_id: &str,
        event_type: &str,
        data: serde_json::Value,
    ) {
        db.create_event(CreateEventRow {
            session_id,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({ "turn_id": turn_id }),
            data,
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }

    fn rule_scorer(key: &str, scope: ObserverScope, rule: Scorer) -> ObserverScorerConfig {
        ObserverScorerConfig {
            key: key.to_string(),
            scope,
            tools: None,
            method: ScorerMethod::Rule { rule },
        }
    }

    async fn observer_with(
        db: &StorageBackend,
        agent: AgentId,
        scorers: Vec<ObserverScorerConfig>,
    ) -> crate::storage::ObserverRow {
        let mut row = observer_with_scorer(agent, scorers[0].clone());
        row.scorers = serde_json::to_value(scorers).unwrap();
        db.create_observer(ORG, row).await.unwrap()
    }

    async fn scores_of(db: &StorageBackend, observer_id: Uuid) -> Vec<TraceScoreRow> {
        db.list_trace_scores(
            ORG,
            crate::storage::ListTraceScoresParams {
                observer_id,
                session_id: None,
                review_queue: false,
                limit: 50,
                offset: 0,
            },
        )
        .await
        .unwrap()
    }

    async fn fixture() -> (Arc<StorageBackend>, AgentId, SessionId) {
        let db = Arc::new(StorageBackend::test_database());
        let agent = AgentId::from_uuid(db.create_test_agent(ORG, Uuid::now_v7()).await);
        let harness = HarnessId::from_uuid(Uuid::now_v7());
        db.create_test_harness(ORG, harness.uuid()).await;
        let session = db
            .create_session(session_row(agent, harness, vec![]))
            .await
            .unwrap();
        (db, agent, session.id)
    }

    #[tokio::test]
    async fn scores_a_session_on_the_whole_conversation() {
        let (db, agent, session) = fixture().await;
        seed_turn(&db, session, "turn_1", "Which order was charged twice?").await;
        seed_turn(
            &db,
            session,
            "turn_2",
            "Ticket T-1042 is open for ORD-48213.",
        )
        .await;
        let observer = observer_with(
            &db,
            agent,
            vec![
                rule_scorer(
                    "answered",
                    ObserverScope::Session,
                    Scorer::Contains {
                        text: "T-1042".into(),
                        weight: 1.0,
                    },
                ),
                rule_scorer(
                    "one_turn",
                    ObserverScope::Session,
                    Scorer::TurnsWithin {
                        max: 1,
                        weight: 1.0,
                    },
                ),
            ],
        )
        .await;
        for key in ["answered", "one_turn"] {
            enqueue_scoped(
                &db,
                observer.id,
                key,
                session,
                "session",
                agent,
                "session",
                None,
            )
            .await;
        }

        let processed = drain_once(&deps(&db), ObserverWorkerConfig::default())
            .await
            .unwrap();
        assert_eq!(processed, 2);
        let scores = scores_of(&db, observer.id).await;
        let by_key = |key: &str| scores.iter().find(|s| s.scorer_key == key).unwrap();
        assert_eq!(by_key("answered").scope, "session");
        assert_eq!(
            by_key("answered").pass,
            Some(true),
            "the last answer is graded"
        );
        // Two turns in the conversation: a session scorer counts turns, not
        // the iterations of one turn.
        assert_eq!(by_key("one_turn").pass, Some(false));
    }

    #[tokio::test]
    async fn scores_one_tool_call() {
        let (db, agent, session) = fixture().await;
        for (id, status) in [("call_1", "failed"), ("call_2", "open")] {
            seed_event(
                &db,
                session,
                "turn_1",
                "tool.started",
                serde_json::json!({"tool_call": {"id": id, "name": "create_ticket", "arguments": {"order": id}}}),
            )
            .await;
            seed_event(
                &db,
                session,
                "turn_1",
                "tool.completed",
                serde_json::json!({"tool_call_id": id, "tool_name": "create_ticket", "success": true,
                    "status": "success", "result": [{"type": "text", "text": format!("status: {status}")}]}),
            )
            .await;
        }
        let observer = observer_with(
            &db,
            agent,
            vec![rule_scorer(
                "opened",
                ObserverScope::Tool,
                Scorer::Contains {
                    text: "open".into(),
                    weight: 1.0,
                },
            )],
        )
        .await;
        enqueue_scoped(
            &db,
            observer.id,
            "opened",
            session,
            "call_2",
            agent,
            "tool",
            None,
        )
        .await;
        enqueue_scoped(
            &db,
            observer.id,
            "opened",
            session,
            "call_1",
            agent,
            "tool",
            None,
        )
        .await;
        enqueue_scoped(
            &db,
            observer.id,
            "opened",
            session,
            "call_9",
            agent,
            "tool",
            None,
        )
        .await;

        drain_once(&deps(&db), ObserverWorkerConfig::default())
            .await
            .unwrap();
        let scores = scores_of(&db, observer.id).await;
        let of = |id: &str| scores.iter().find(|s| s.turn_id == id).unwrap();
        assert_eq!(of("call_2").pass, Some(true));
        assert_eq!(
            of("call_1").pass,
            Some(false),
            "each call is graded on its own result"
        );
        assert_eq!(
            of("call_9").status,
            "skipped",
            "a call not in the trace is skipped"
        );
    }

    #[tokio::test]
    async fn a_session_score_waits_out_its_settle_delay_and_rearms() {
        let (db, agent, session) = fixture().await;
        seed_turn(&db, session, "turn_1", "done").await;
        let observer = observer_with(
            &db,
            agent,
            vec![rule_scorer(
                "done",
                ObserverScope::Session,
                Scorer::Contains {
                    text: "done".into(),
                    weight: 1.0,
                },
            )],
        )
        .await;
        let later = chrono::Utc::now() + chrono::Duration::seconds(600);
        enqueue_scoped(
            &db,
            observer.id,
            "done",
            session,
            "session",
            agent,
            "session",
            Some(later),
        )
        .await;
        let config = ObserverWorkerConfig::default();
        assert_eq!(drain_once(&deps(&db), config).await.unwrap(), 0);

        // The session idled again: the same score is re-armed, not duplicated.
        let past = chrono::Utc::now() - chrono::Duration::seconds(1);
        enqueue_scoped(
            &db,
            observer.id,
            "done",
            session,
            "session",
            agent,
            "session",
            Some(past),
        )
        .await;
        assert_eq!(scores_of(&db, observer.id).await.len(), 1);
        assert_eq!(drain_once(&deps(&db), config).await.unwrap(), 1);
        assert_eq!(scores_of(&db, observer.id).await[0].pass, Some(true));

        // A later idle re-arms the completed score for another look.
        enqueue_scoped(
            &db,
            observer.id,
            "done",
            session,
            "session",
            agent,
            "session",
            Some(past),
        )
        .await;
        let rearmed = scores_of(&db, observer.id).await;
        assert_eq!(rearmed.len(), 1);
        assert_eq!(rearmed[0].status, "pending");
    }

    #[tokio::test]
    async fn drains_and_scores_a_matching_turn() {
        let db = Arc::new(StorageBackend::test_database());
        let agent = AgentId::from_uuid(db.create_test_agent(ORG, Uuid::now_v7()).await);
        let harness = HarnessId::from_uuid(Uuid::now_v7());
        db.create_test_harness(ORG, harness.uuid()).await;
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
                crate::storage::ListTraceScoresParams {
                    observer_id: observer.id,
                    session_id: None,
                    limit: 10,
                    offset: 0,
                    review_queue: false,
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
        let db = Arc::new(StorageBackend::test_database());
        let agent = AgentId::from_uuid(db.create_test_agent(ORG, Uuid::now_v7()).await);
        let harness = HarnessId::from_uuid(Uuid::now_v7());
        db.create_test_harness(ORG, harness.uuid()).await;
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
                crate::storage::ListTraceScoresParams {
                    observer_id: observer.id,
                    session_id: None,
                    limit: 10,
                    offset: 0,
                    review_queue: false,
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
        let db = Arc::new(StorageBackend::test_database());
        let agent = AgentId::from_uuid(db.create_test_agent(ORG, Uuid::now_v7()).await);
        let harness = HarnessId::from_uuid(Uuid::now_v7());
        db.create_test_harness(ORG, harness.uuid()).await;
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
            crate::storage::UpdateObserverRow {
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
                crate::storage::ListTraceScoresParams {
                    observer_id: observer.id,
                    session_id: None,
                    limit: 10,
                    offset: 0,
                    review_queue: false,
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
                tools: None,
                method: ScorerMethod::LlmJudge(LlmJudgeConfig {
                    rubric: "Did the agent greet the user?".to_string(),
                    model_id: None,
                    pass_threshold: 0.5,
                }),
            },
        )
    }

    async fn run_judge_observer(judge: Option<Arc<dyn JudgeClient>>) -> Vec<TraceScoreRow> {
        let db = Arc::new(StorageBackend::test_database());
        let agent = AgentId::from_uuid(db.create_test_agent(ORG, Uuid::now_v7()).await);
        let harness = HarnessId::from_uuid(Uuid::now_v7());
        db.create_test_harness(ORG, harness.uuid()).await;
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

        let graders = match judge {
            Some(judge) => Graders::new(db.clone()).with_judge(judge),
            None => Graders::new(db.clone()),
        };
        let deps = ObserverWorkerDeps {
            db: db.clone(),
            graders,
        };
        drain_once(&deps, ObserverWorkerConfig::default())
            .await
            .unwrap();
        db.list_trace_scores(
            ORG,
            crate::storage::ListTraceScoresParams {
                observer_id: observer.id,
                session_id: None,
                limit: 10,
                offset: 0,
                review_queue: false,
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
        // Decisions client present, but the org has no model configured (Ok(None)).
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

    #[tokio::test]
    async fn decision_rule_routes_through_the_shared_graders() {
        // No decision model on this deployment: the shared grader says so and
        // the observer skips the score instead of retrying it.
        let db = Arc::new(StorageBackend::test_database());
        let agent = AgentId::from_uuid(db.create_test_agent(ORG, Uuid::now_v7()).await);
        let harness = HarnessId::from_uuid(Uuid::now_v7());
        db.create_test_harness(ORG, harness.uuid()).await;
        let turn_id = "turn_01933b5a0000700080000000000000dc";
        let session = db
            .create_session(session_row(agent, harness, vec![]))
            .await
            .unwrap();
        seed_turn(&db, session.id, turn_id, "hello there").await;
        let observer = db
            .create_observer(
                ORG,
                observer_with_scorer(
                    agent,
                    ObserverScorerConfig {
                        key: "polite".to_string(),
                        scope: ObserverScope::Turn,
                        tools: None,
                        method: ScorerMethod::Rule {
                            rule: Scorer::Decision {
                                question: "Is the answer polite?".to_string(),
                                yes: None,
                                no: None,
                                pass_threshold: 0.5,
                                weight: 1.0,
                            },
                        },
                    },
                ),
            )
            .await
            .unwrap();
        enqueue(&db, observer.id, "polite", session.id, turn_id, agent).await;

        drain_once(&deps(&db), ObserverWorkerConfig::default())
            .await
            .unwrap();
        let scores = db
            .list_trace_scores(
                ORG,
                crate::storage::ListTraceScoresParams {
                    observer_id: observer.id,
                    session_id: None,
                    limit: 10,
                    offset: 0,
                    review_queue: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(scores[0].status, "skipped");
        assert_eq!(
            scores[0].reason.as_deref(),
            Some("decision model unavailable on this deployment")
        );
    }
}
