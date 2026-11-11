use super::*;
use std::sync::Mutex;

use crate::domains::health_issues::relevance_fallback;
use crate::storage::{CreateAgentRow, CreateHarnessRow};
use everruns_contracts::driver_registry::LlmCompletionMetadata;
use everruns_core::{DEFAULT_ORG_ID, DecisionAnswer};

/// A chat model that answers every check the same way.
struct Scripted {
    reply: String,
    prompts: Mutex<Vec<String>>,
}

#[async_trait]
impl UtilityLlmService for Scripted {
    fn is_configured(&self) -> bool {
        true
    }

    async fn chat_completion(
        &self,
        request: UtilityLlmRequest,
    ) -> everruns_contracts::error::Result<LlmResponse> {
        let prompt = request
            .messages
            .iter()
            .map(|message| message.content_as_text())
            .collect::<Vec<_>>()
            .join("\n");
        self.prompts.lock().unwrap().push(prompt);
        Ok(LlmResponse {
            text: self.reply.clone(),
            reasoning: Vec::new(),
            tool_calls: None,
            metadata: LlmCompletionMetadata::default(),
        })
    }

    async fn chat_completion_stream(
        &self,
        _request: UtilityLlmRequest,
    ) -> everruns_contracts::error::Result<LlmResponseStream> {
        unreachable!("relevance checks never stream")
    }
}

struct Fallback(Option<Arc<Scripted>>);

#[async_trait]
impl FallbackModels for Fallback {
    async fn for_agent(
        &self,
        _org_id: i64,
        _agent: &AgentRow,
    ) -> anyhow::Result<Option<FallbackModel>> {
        Ok(self.0.clone().map(|llm| FallbackModel {
            llm,
            model: "agent-model".into(),
        }))
    }
}

/// A decision model that answers with one probability.
struct Deployment(f64);

#[async_trait]
impl DecisionsService for Deployment {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(
        &self,
        _request: DecisionRequest,
    ) -> everruns_contracts::error::Result<DecisionOutcome> {
        Ok(DecisionOutcome {
            calibrated: true,
            model: "decider".into(),
            answers: [(
                "should_respond".to_string(),
                DecisionAnswer::Noul {
                    probability: self.0,
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        })
    }
}

fn scripted(answer: &str) -> Arc<Scripted> {
    Arc::new(Scripted {
        reply: format!(r#"{{"answers": {{"q1": "{answer}"}}}}"#),
        prompts: Mutex::new(Vec::new()),
    })
}

async fn agent(db: &StorageBackend) -> AgentRow {
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                localizations: Default::default(),
                name: "relevance-harness".into(),
                display_name: None,
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: Some(String::new()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                embedder_metadata: json!({}),
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    db.create_agent(
        DEFAULT_ORG_ID,
        CreateAgentRow {
            completion_check: Default::default(),
            localizations: Default::default(),
            public_id: everruns_contracts::typed_id::AgentId::new().to_string(),
            name: "srebot".into(),
            display_name: None,
            description: Some("Answers incident questions".into()),
            intro_markdown: None,
            short_description: None,
            starters: json!([]),
            system_prompt: format!("Investigate production incidents. {}", "x".repeat(3000)),
            default_model_id: None,
            harness_id: harness.id,
            tags: vec![],
            initial_files: json!([]),
            tools: json!([]),
            mcp_servers: json!({}),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            max_output_tokens: None,
            communication: Default::default(),
            environments: None,
            is_built_in: false,
        },
    )
    .await
    .unwrap()
}

fn checker(
    db: &Arc<StorageBackend>,
    deployment: Arc<dyn DecisionsService>,
    fallback: Option<Arc<Scripted>>,
) -> RelevanceChecker {
    RelevanceChecker {
        db: db.clone(),
        deployment,
        org: None,
        events: None,
        fallback: Some(Arc::new(Fallback(fallback))),
        reports: Default::default(),
    }
}

fn question<'a>(agent: &'a AgentRow) -> RelevanceQuestion<'a> {
    let recent = (0..10)
        .map(|i| json!({"author": "rui", "role": "user", "text": format!("earlier {i}")}))
        .collect();
    RelevanceQuestion {
        org_id: DEFAULT_ORG_ID,
        agent,
        state: question_state(
            agent,
            Some("Watch #incidents."),
            json!({"text": "checkout is throwing 5xx since 2:10 PM", "author": "chad"}),
            recent,
        ),
        session: None,
        source: "room_wake",
        subject: ("room_id", "room_test"),
    }
}

async fn fallback_issue(db: &StorageBackend) -> Option<crate::storage::HealthIssueRow> {
    for _ in 0..100 {
        let rows = db
            .list_health_issues(DEFAULT_ORG_ID, 0, 10, None)
            .await
            .unwrap();
        if let Some(row) = rows
            .into_iter()
            .find(|row| row.code == relevance_fallback::CODE)
        {
            return Some(row);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    None
}

#[tokio::test]
async fn the_state_keeps_the_latest_history_and_the_room_role() {
    let db = StorageBackend::test_database();
    let mut agent = agent(&db).await;
    agent.system_prompt = "Purpose.".into();
    let recent = (0..12).map(|i| json!({"text": i})).collect();
    let state = question_state(&agent, Some("Room role."), json!({}), recent);
    assert_eq!(state["agent"]["purpose"], "Purpose.\n\nRoom role.");
    let kept = state["recent_thread_messages"].as_array().unwrap();
    assert_eq!(kept.len(), HISTORY_LIMIT);
    assert_eq!(kept[0]["text"], 4);

    let limited = limited_state(state);
    let kept = limited["recent_thread_messages"].as_array().unwrap();
    assert_eq!(kept.len(), FALLBACK_HISTORY_LIMIT);
    assert_eq!(kept.last().unwrap()["text"], 11);
}

#[tokio::test]
async fn a_decision_model_answers_when_one_is_configured() {
    let db = Arc::new(StorageBackend::test_database());
    let agent = agent(&db).await;
    let fallback = scripted("yes");
    let checker = checker(&db, Arc::new(Deployment(0.95)), Some(fallback.clone()));
    let verdict = checker.decide(question(&agent)).await.unwrap();
    assert!(verdict.respond);
    assert_eq!(verdict.judge, Judge::Deployment);
    assert!(verdict.calibrated);
    assert!(fallback.prompts.lock().unwrap().is_empty());

    let checker = self::checker(&db, Arc::new(Deployment(0.5)), None);
    assert!(!checker.decide(question(&agent)).await.unwrap().respond);
}

#[tokio::test]
async fn without_a_decision_model_the_agents_model_answers_with_less_context() {
    let db = Arc::new(StorageBackend::test_database());
    let agent = agent(&db).await;
    let fallback = scripted("yes");
    let checker = checker(
        &db,
        Arc::new(everruns_core::DisabledDecisionsService),
        Some(fallback.clone()),
    );
    let verdict = checker.decide(question(&agent)).await.unwrap();
    assert!(verdict.respond);
    assert_eq!(verdict.judge, Judge::AgentModel);
    assert!(!verdict.calibrated);
    assert_eq!(verdict.model, "agent-model");

    let prompt = fallback.prompts.lock().unwrap()[0].clone();
    assert!(prompt.contains("checkout is throwing 5xx"));
    assert!(prompt.contains("earlier 9") && !prompt.contains("earlier 5"));
    assert!(!prompt.contains(&"x".repeat(FALLBACK_PURPOSE_BYTES)));

    // The org is told, so someone sets up a decision model.
    let issue = fallback_issue(&db).await.expect("health issue opened");
    assert_eq!(issue.status, "open");

    // A "no" from the agent's model is silence.
    let checker = self::checker(
        &db,
        Arc::new(everruns_core::DisabledDecisionsService),
        Some(scripted("no")),
    );
    assert!(!checker.decide(question(&agent)).await.unwrap().respond);
}

#[tokio::test]
async fn no_decision_model_and_no_agent_model_is_silence() {
    let db = Arc::new(StorageBackend::test_database());
    let agent = agent(&db).await;
    let checker = checker(&db, Arc::new(everruns_core::DisabledDecisionsService), None);
    assert!(checker.decide(question(&agent)).await.is_err());
    // Still reported: the agents set to answer relevant messages never do.
    assert!(fallback_issue(&db).await.is_some());
}

#[tokio::test]
async fn a_decision_model_answering_again_resolves_the_issue() {
    let db = Arc::new(StorageBackend::test_database());
    let agent = agent(&db).await;
    let reports: Arc<relevance_fallback::FallbackReports> = Default::default();
    let mut without = checker(
        &db,
        Arc::new(everruns_core::DisabledDecisionsService),
        Some(scripted("yes")),
    );
    without.reports = reports.clone();
    without.decide(question(&agent)).await.unwrap();
    let issue = fallback_issue(&db).await.unwrap();
    assert_eq!(issue.status, "open");

    let mut with = checker(&db, Arc::new(Deployment(0.95)), None);
    with.reports = reports;
    with.decide(question(&agent)).await.unwrap();
    for _ in 0..100 {
        let row = db.get_health_issue(DEFAULT_ORG_ID, issue.id).await.unwrap();
        if row.is_some_and(|row| row.status == "resolved") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the fallback issue stayed open after a decision model answered");
}
