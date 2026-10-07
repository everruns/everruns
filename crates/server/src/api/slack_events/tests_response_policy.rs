use super::*;
use std::sync::{Arc, Mutex};

use crate::api::slack_events::tests_support::{NoopRunner, test_app, test_config, test_event};
use crate::storage::{StorageBackend, models::CreateAgentRow};
use async_trait::async_trait;
use everruns_core::{DecisionAnswer, DecisionOutcome};

struct Judge {
    probability: Option<f64>,
    fail: bool,
    delay: Duration,
    requests: Mutex<Vec<DecisionRequest>>,
}

impl Judge {
    fn probability(value: f64) -> Self {
        Self {
            probability: Some(value),
            fail: false,
            delay: Duration::ZERO,
            requests: Mutex::new(vec![]),
        }
    }
}

#[async_trait]
impl DecisionsService for Judge {
    fn is_configured(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "test judge"
    }
    async fn evaluate(
        &self,
        request: DecisionRequest,
    ) -> everruns_contracts::error::Result<DecisionOutcome> {
        self.requests.lock().unwrap().push(request);
        tokio::time::sleep(self.delay).await;
        if self.fail {
            return Err(everruns_contracts::error::AgentLoopError::llm(
                "judge unavailable",
            ));
        }
        Ok(DecisionOutcome {
            calibrated: true,
            answers: self
                .probability
                .map(|probability| {
                    (
                        "should_respond".into(),
                        DecisionAnswer::Noul { probability },
                    )
                })
                .into_iter()
                .collect(),
            ..Default::default()
        })
    }
}

async fn fixture(
    judge: Option<Arc<Judge>>,
) -> (
    SlackState,
    super::super::tests_support::TestIngress,
    SlackChannelConfig,
    SlackEvent,
) {
    let db = Arc::new(StorageBackend::test_database());
    let mut app = test_app();
    let agent = db
        .create_agent(
            app.org_id,
            CreateAgentRow {
                public_id: everruns_contracts::typed_id::AgentId::new().to_string(),
                name: "dad-jokes".into(),
                display_name: None,
                description: Some("Dad Jokes Agent".into()),
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: "Tell dad jokes when requested.".into(),
                default_model_id: None,
                harness_id: app.harness_id,
                tags: vec![],
                initial_files: json!([]),
                tools: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    app.agent_id = Some(agent.id);
    app.agent_internal_id = agent.id.uuid();
    let mut state = SlackState::new(
        db,
        None,
        Arc::new(NoopRunner),
        None,
        false,
        crate::event_delivery::EventDelivery::in_memory(),
        "https://example.com/api".into(),
    );
    if let Some(judge) = judge {
        state.decisions = judge;
    }
    let mut config = test_config(everruns_core::channel::SessionBinding::Thread);
    config.response_policy = SlackResponsePolicy::RelevantMessages;
    let mut event = test_event("C1", Some("1.0"), None);
    event.text = Some("Unrelated note to myself".into());
    (state, app, config, event)
}

#[tokio::test]
async fn only_a_positive_decision_starts_unmentioned_messages() {
    for (probability, expected) in [
        (0.01, false),
        (0.5, false),
        (0.899, false),
        (0.9, true),
        (0.99, true),
    ] {
        let judge = Arc::new(Judge::probability(probability));
        let (state, app, config, event) = fixture(Some(judge.clone())).await;
        assert_eq!(
            should_process_message(&state, &app, &app.channels[0], &config, &event).await,
            expected
        );
        let request = judge.requests.lock().unwrap();
        assert_eq!(
            request[0].state["latest_message"]["text"],
            "Unrelated note to myself"
        );
        assert_eq!(
            request[0].state["agent"]["purpose"],
            "Tell dad jokes when requested."
        );
        assert_eq!(request[0].metadata["source"], "slack_response_policy");
    }
}

#[tokio::test]
async fn mentions_and_dms_bypass_the_judge_even_when_unavailable() {
    let judge = Arc::new(Judge {
        fail: true,
        ..Judge::probability(0.0)
    });
    let (state, app, config, mut event) = fixture(Some(judge.clone())).await;
    event.event_type = "app_mention".into();
    assert!(should_process_message(&state, &app, &app.channels[0], &config, &event).await);
    event.event_type = "message".into();
    event.channel_type = Some("im".into());
    assert!(should_process_message(&state, &app, &app.channels[0], &config, &event).await);
    event.channel_type = None;
    event.channel = Some("D1".into());
    assert!(should_process_message(&state, &app, &app.channels[0], &config, &event).await);
    assert!(judge.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn all_messages_preserves_existing_behavior_without_a_judge() {
    let (state, app, mut config, event) = fixture(None).await;
    config.response_policy = SlackResponsePolicy::AllMessages;
    assert!(should_process_message(&state, &app, &app.channels[0], &config, &event).await);
}

#[tokio::test]
async fn missing_failed_and_invalid_decisions_stay_silent() {
    let (state, app, config, event) = fixture(None).await;
    assert!(!should_process_message(&state, &app, &app.channels[0], &config, &event).await);
    for judge in [
        Judge {
            fail: true,
            ..Judge::probability(1.0)
        },
        Judge {
            probability: None,
            ..Judge::probability(1.0)
        },
        Judge::probability(f64::NAN),
        Judge::probability(1.1),
    ] {
        let (state, app, config, event) = fixture(Some(Arc::new(judge))).await;
        assert!(!should_process_message(&state, &app, &app.channels[0], &config, &event).await);
    }
}

#[tokio::test]
async fn judge_timeout_stays_silent() {
    let judge = Arc::new(Judge {
        delay: DECISION_TIMEOUT + Duration::from_millis(100),
        ..Judge::probability(1.0)
    });
    let (state, app, config, event) = fixture(Some(judge)).await;
    assert!(!should_process_message(&state, &app, &app.channels[0], &config, &event).await);
}

#[tokio::test]
async fn ignored_messages_do_not_create_sessions_or_start_work() {
    for policy in [
        SlackResponsePolicy::RelevantMessages,
        SlackResponsePolicy::MentionsOnly,
    ] {
        let judge = Arc::new(Judge::probability(0.01));
        let (state, app, mut config, event) = fixture(Some(judge)).await;
        config.response_policy = policy;
        config.reply_mode = crate::records::SlackReplyMode::ToolOnly;
        super::super::process_slack_message(&state, &app, &app.channels[0], &config, &event, None)
            .await
            .unwrap();
        let tags = build_session_tags(
            &app,
            &app.channels[0],
            &config,
            &event,
            crate::slack_delivery::SlackSurface::Channel,
        )
        .unwrap();
        assert!(
            find_slack_session(&state, &app, &app.channels[0], &tags)
                .await
                .unwrap()
                .is_none()
        );
    }
}

fn history_event(
    kind: &str,
    id: &str,
    input: &str,
    channel: &str,
    thread: &str,
    text: &str,
) -> EventRow {
    EventRow {
        id: everruns_contracts::typed_id::EventId::new(),
        session_id: everruns_contracts::typed_id::SessionId::new(),
        sequence: 1,
        event_type: kind.into(),
        ts: chrono::Utc::now(),
        created_at: chrono::Utc::now(),
        metadata: None,
        tags: None,
        context: json!({"input_message_id": input}),
        data: json!({"message": {"id": id, "role": if kind == "input.message" { "user" } else { "assistant" },
            "metadata": {"slack_channel": channel, "slack_thread_ts": thread},
            "content": [{"type": "text", "text": text}]}}),
    }
}

#[test]
fn history_is_scoped_to_the_thread_and_correlated_outputs() {
    let events = vec![
        history_event("input.message", "a", "", "C1", "1", "Tell me a joke"),
        history_event("output.message.completed", "b", "a", "", "", "A dad joke"),
        history_event(
            "tool.completed",
            "tool",
            "a",
            "",
            "",
            "Tool output is not conversation context",
        ),
        history_event("input.message", "c", "", "C1", "2", "Unrelated thread"),
        history_event(
            "output.message.completed",
            "d",
            "c",
            "",
            "",
            "Unrelated answer",
        ),
        history_event("input.message", "e", "", "C2", "1", "Another channel"),
    ];
    assert_eq!(
        thread_history(&events, "C1", "1"),
        vec![
            json!({"role": "user", "text": "Tell me a joke"}),
            json!({"role": "assistant", "text": "A dad joke"})
        ]
    );
    assert!(thread_history(&events, "C1", "unknown").is_empty());
}

#[tokio::test]
async fn state_and_history_are_bounded_without_splitting_unicode() {
    let (state, app, config, mut event) = fixture(None).await;
    event.text = Some("🦀".repeat(4000));
    let value = decision_state(&state, &app, &app.channels[0], &config, &event)
        .await
        .unwrap();
    assert!(value["latest_message"]["text"].as_str().unwrap().len() <= 2048);
    assert!(value.to_string().len() < 12_000);
    let events: Vec<_> = (0..100)
        .map(|_| history_event("input.message", "a", "", "C1", "1", &"🦀".repeat(500)))
        .collect();
    let history = thread_history(&events, "C1", "1");
    assert_eq!(history.len(), HISTORY_LIMIT);
    assert!(
        history
            .iter()
            .all(|message| message["text"].as_str().unwrap().len() <= 512)
    );
}

#[tokio::test]
async fn the_judge_uses_the_endpoints_pinned_purpose() {
    let (mut state, mut app, config, event) = fixture(None).await;
    state.agent_versions_enabled = true;
    let id = everruns_contracts::typed_id::AgentVersionId::new();
    state.db.create_agent_version(crate::storage::models::CreateAgentVersionRow {
        id, public_id: id.to_string(), org_id: app.org_id, agent_id: app.agent_id.unwrap(),
        version_number: 1, semver_major: 1, semver_minor: 0, semver_patch: 0,
        version: "1.0.0".into(), is_published: true, parent_version_id: None, source_version_id: None,
        created_by_principal_id: None, change_kind: "manual".into(), summary: None, config_hash: "test".into(),
        authored_config: json!({"name": "pinned-agent", "description": "Pinned purpose", "system_prompt": "Answer questions about invoices."}),
        resolved_config: json!({}),
    }).await.unwrap();
    app.agent_version_policy = crate::records::AgentVersionPolicy::Pinned;
    app.agent_version_id = Some(id);
    let context = decision_state(&state, &app, &app.channels[0], &config, &event)
        .await
        .unwrap();
    assert_eq!(
        context["agent"]["purpose"],
        "Answer questions about invoices."
    );
    assert_eq!(context["agent"]["name"], "pinned-agent");
}

/// Run explicitly with the deployment-owned UTILITY_TYPESAFE_API_KEY.
#[tokio::test]
#[ignore = "requires a live TypeSafe utility key"]
async fn live_jev_relevance_matches_intent_examples() {
    let service = everruns_integrations::typesafe::SystemDecisionsConfig::from_env()
        .into_driver()
        .expect("UTILITY_TYPESAFE_API_KEY must be configured");
    let (state, app, config, mut event) = fixture(None).await;
    for (message, history, expected) in [
        ("Any more dad jokes?", vec![], true),
        ("My dad tells terrible jokes", vec![], false),
        ("Unrelated note to myself", vec![], false),
        (
            "Another one!",
            vec![
                json!({"role": "user", "text": "Tell me a dad joke"}),
                json!({"role": "assistant", "text": "Why did the scarecrow win an award? He was outstanding in his field."}),
            ],
            true,
        ),
    ] {
        event.text = Some(message.into());
        let mut context = decision_state(&state, &app, &app.channels[0], &config, &event)
            .await
            .unwrap();
        context["recent_thread_messages"] = json!(history);
        let started = std::time::Instant::now();
        let actual = evaluate_relevance(&service, context, "live-test")
            .await
            .unwrap();
        println!(
            "{message:?}: respond={actual}, latency={:?}",
            started.elapsed()
        );
        assert_eq!(actual, expected, "{message}");
    }
}

async fn answer_with_org_model(state: &mut SlackState, org_id: i64) {
    state
        .db
        .patch_organization_settings(
            org_id,
            crate::storage::models::UpdateOrganizationSettings {
                system_decisions: Some(crate::storage::SystemDecisions::Organization),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    state.org_decisions = Some(super::super::SlackOrgDecisions {
        provider_resolver: Arc::new(crate::services::ProviderResolverService::new(
            state.db.clone(),
            None,
        )),
        budget_service: Arc::new(crate::domains::budgets::BudgetService::new(
            state.db.clone(),
        )),
        egress: Arc::new(everruns_core::host::DirectEgressService::default()),
    });
}

// THREAT[TM-LLM-037]: an org that answers these checks itself is never
// answered by the deployment, even when its own model is missing.
#[tokio::test]
async fn an_org_without_a_usable_model_stays_silent_instead_of_using_the_deployment() {
    let judge = Arc::new(Judge::probability(0.99));
    let (mut state, app, config, event) = fixture(Some(judge.clone())).await;
    answer_with_org_model(&mut state, app.org_id).await;
    assert!(!should_process_message(&state, &app, &app.channels[0], &config, &event).await);
    // Unwired org decisions are silent too.
    state.org_decisions = None;
    assert!(!should_process_message(&state, &app, &app.channels[0], &config, &event).await);
    assert!(judge.requests.lock().unwrap().is_empty());
    // Mentions never needed a decision.
    let mut mention = event.clone();
    mention.event_type = "app_mention".into();
    assert!(should_process_message(&state, &app, &app.channels[0], &config, &mention).await);
}

#[tokio::test]
async fn an_org_that_leaves_it_to_the_deployment_keeps_the_deployment_judge() {
    let judge = Arc::new(Judge::probability(0.99));
    let (mut state, app, config, event) = fixture(Some(judge.clone())).await;
    answer_with_org_model(&mut state, app.org_id).await;
    state
        .db
        .patch_organization_settings(
            app.org_id,
            crate::storage::models::UpdateOrganizationSettings {
                system_decisions: Some(crate::storage::SystemDecisions::Deployment),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(should_process_message(&state, &app, &app.channels[0], &config, &event).await);
    assert_eq!(judge.requests.lock().unwrap().len(), 1);
}
