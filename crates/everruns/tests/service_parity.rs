#![allow(clippy::unwrap_used, clippy::expect_used)]
use everruns::{Agent, ContentPart, Engine, LlmSimConfig, Model};
use everruns_llmsim::{SimToolCall, SimTurn};
use serde_json::{Value, json};
fn scripted(calls: Vec<(&str, Value)>) -> everruns::AgentBuilder {
    let mut turns = calls
        .into_iter()
        .enumerate()
        .map(|(i, (name, arguments))| {
            SimTurn::ToolCalls(vec![SimToolCall {
                name: name.into(),
                arguments,
                id: Some(format!("call_{i}")),
            }])
        })
        .collect::<Vec<_>>();
    turns.push(SimTurn::Assistant("Done.".into()));
    Agent::builder()
        .instructions("Use the configured tools.")
        .model(Model::simulated_with_config(LlmSimConfig::scripted(turns)))
}
async fn results(session: &everruns::Session) -> Vec<Value> {
    session
        .history()
        .page()
        .await
        .unwrap()
        .messages
        .into_iter()
        .flat_map(|m| m.content)
        .filter_map(|p| match p {
            ContentPart::ToolResult(t) => Some(json!({"result":t.result,"error":t.error})),
            _ => None,
        })
        .collect()
}
#[cfg(feature = "sql-database")]
#[tokio::test]
async fn sql_runs_through_engine_and_reopens_session_scoped_data() {
    use everruns::{SessionId, database::SqlDatabase, hosting::AgentBuild};
    let root = tempfile::tempdir().unwrap();
    let id = SessionId::new();
    let build = AgentBuild {
        agent: "sql".into(),
        build_id: "1".into(),
    };
    let agent=scripted(vec![("sql_execute",json!({"database":"notes","sql":"CREATE TABLE notes(text TEXT); INSERT INTO notes VALUES('retained')"})),("sql_query",json!({"database":"notes","sql":"SELECT text FROM notes"}))]).sql_database(SqlDatabase::open(root.path()).unwrap()).build().unwrap();
    let engine = Engine::builder().register(build.clone(), agent).build();
    let session = engine.create_registered(id, &build).await.unwrap();
    assert!(session.run("save").await.unwrap().success);
    let data = results(&session).await;
    assert_eq!(data.len(), 2);
    assert!(data.iter().all(|v| v["error"].is_null()), "{data:?}");
    assert_eq!(data[1]["result"]["rows"][0][0], "retained");
    drop(session);
    drop(engine);
    let agent = scripted(vec![
        (
            "sql_query",
            json!({"database":"notes","sql":"SELECT text FROM notes"}),
        ),
        (
            "sql_query",
            json!({"database":"notes","sql":"ATTACH DATABASE '/tmp/escape.sqlite' AS other"}),
        ),
    ])
    .sql_database(SqlDatabase::open(root.path()).unwrap())
    .build()
    .unwrap();
    let engine = Engine::builder().register(build.clone(), agent).build();
    let session = engine.create_registered(id, &build).await.unwrap();
    session.run("read").await.unwrap();
    let data = results(&session).await;
    assert_eq!(data[0]["result"]["rows"][0][0], "retained");
    assert!(!data[1]["error"].is_null());
    let other = engine
        .create_registered(SessionId::new(), &build)
        .await
        .unwrap();
    other.run("read").await.unwrap();
    assert!(!results(&other).await[0]["error"].is_null());
}
#[cfg(feature = "knowledge")]
#[tokio::test]
async fn knowledge_tools_return_local_evidence_and_citations() {
    use everruns::knowledge::{Document, Knowledge};
    let source = Knowledge::documents(
        "support",
        vec![
            Document::new(
                "Refund policy",
                "Refund requests are accepted for 30 days.",
                "https://example.com/refunds",
            )
            .at("policy/refunds.md"),
        ],
    );
    let collection = source.collection_id().to_string();
    let agent = scripted(vec![
        ("search_knowledge", json!({"query":"refund"})),
        ("browse_knowledge", json!({"collection":collection})),
        ("read_file",json!({"path":format!("/knowledge/{collection}/policy/refunds.md")})),
        ("write_file",json!({"path":format!("/knowledge/{collection}/policy/evil.md"),"content":"not allowed"})),
    ])
    .knowledge(source)
    .build()
    .unwrap();
    let session = Engine::new().create(agent);
    session.run("Find refund policy").await.unwrap();
    let data = results(&session).await;
    assert_eq!(data.len(), 4);
    assert!(data[..3].iter().all(|d| d["error"].is_null()), "{data:?}");
    assert!(!data[3]["error"].is_null());
    let text = serde_json::to_string(&data).unwrap();
    assert!(text.contains("Refund requests"));
    assert!(text.contains("https://example.com/refunds"));
    assert!(text.contains("version_id"));
}
#[tokio::test]
async fn read_only_named_memory_rejects_create_edit_delete_and_survives_new_session() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("policy.md"), "Retained memory").unwrap();
    let agent = scripted(vec![
        ("read_file", json!({"path":"/memory/policy/policy.md"})),
        (
            "write_file",
            json!({"path":"/memory/policy/new.md","content":"bad"}),
        ),
        ("delete_file", json!({"path":"/memory/policy/policy.md"})),
    ])
    .memory(everruns::MemoryDirectory::directory("policy", root.path()).read_only())
    .capability("session_file_system")
    .build()
    .unwrap();
    let engine = Engine::new();
    let session = engine.create(agent.clone());
    session.run("read and change").await.unwrap();
    let data = results(&session).await;
    assert_eq!(data.len(), 3);
    assert!(data[0]["error"].is_null(), "{data:?}");
    assert!(!data[1]["error"].is_null());
    assert!(!data[2]["error"].is_null());
    assert!(!root.path().join("new.md").exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("policy.md")).unwrap(),
        "Retained memory"
    );
    let session = engine.create(agent);
    session.run("read").await.unwrap();
    assert!(
        results(&session).await[0]
            .to_string()
            .contains("Retained memory")
    );
}
#[cfg(feature = "slack-actions")]
#[tokio::test]
async fn slack_native_actions_use_shared_operations_and_keep_credentials_out_of_history() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };
    let server = MockServer::start().await;
    for (endpoint, body) in [
        ("reactions.add", json!({"ok":true})),
        ("chat.update", json!({"ok":true})),
        (
            "users.info",
            json!({"ok":true,"user":{"profile":{"display_name":"Alice","email":"private@example.com"},"is_bot":false}}),
        ),
        (
            "files.getUploadURLExternal",
            json!({"ok":true,"upload_url":format!("{}/upload",server.uri()),"file_id":"F1"}),
        ),
        (
            "files.completeUploadExternal",
            json!({"ok":true,"files":[{"permalink":"https://slack.com/F1"}]}),
        ),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/{endpoint}")))
            .and(header("authorization", "Bearer sentinel-slack-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/upload"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let actions = everruns::slack::SlackActions::new("sentinel-slack-key", "C1", "1.0")
        .api_base(server.uri());
    assert!(!format!("{actions:?}").contains("sentinel-slack-key"));
    let agent = scripted(vec![
        (
            "slack_add_reaction",
            json!({"channel":"C1","timestamp":"1.0","name":"eyes"}),
        ),
        (
            "slack_update_message",
            json!({"channel":"C1","timestamp":"1.0","text":"Updated <!channel>"}),
        ),
        ("slack_lookup_user", json!({"user_id":"U1"})),
        (
            "slack_upload_file",
            json!({"filename":"note.txt","content":"Hello"}),
        ),
        (
            "slack_add_reaction",
            json!({"channel":"OTHER","timestamp":"1.0","name":"eyes"}),
        ),
    ])
    .slack_actions(actions)
    .build()
    .unwrap();
    let session = Engine::new().create(agent);
    session.run("Update this conversation").await.unwrap();
    let data = results(&session).await;
    assert_eq!(data.len(), 5);
    assert!(data[..4].iter().all(|v| v["error"].is_null()), "{data:?}");
    assert!(!data[4]["error"].is_null());
    let text = format!("{:?}", session.history().page().await.unwrap());
    assert!(!text.contains("sentinel-slack-key"));
    assert!(!text.contains("private@example.com"));
}
#[cfg(feature = "mcp-auth")]
#[test]
fn mcp_auth_is_explicit_endpoint_bound_and_credential_free() {
    use everruns::{McpServer, mcp::auth::McpAuth};
    let auth = McpAuth::new().bearer("docs", "https://docs.example/mcp", "mcp-secret");
    assert!(!format!("{auth:?}").contains("mcp-secret"));
    let agent = scripted(vec![])
        .mcp_server(McpServer::http("docs", "https://docs.example/mcp"))
        .mcp_auth(auth.clone())
        .build()
        .unwrap();
    assert!(!format!("{agent:?}").contains("mcp-secret"));
    assert!(
        scripted(vec![])
            .mcp_server(McpServer::http("docs", "https://other.example/mcp"))
            .mcp_auth(auth.clone())
            .build()
            .is_err()
    );
    assert!(scripted(vec![]).mcp_auth(auth).build().is_err());
}
#[cfg(feature = "evaluation")]
#[tokio::test]
async fn evaluations_reuse_rules_and_trace_projection_from_real_engine_events() {
    use everruns::evaluation::{Case, Evaluator, Scorer};
    use std::time::Duration;
    let evaluator = Evaluator::new(vec![
        Scorer::Contains {
            text: "Done".into(),
            weight: 1.0,
        },
        Scorer::TurnsWithin {
            max: 1,
            weight: 1.0,
        },
    ])
    .unwrap();
    let engine = Engine::new();
    let results = evaluator
        .run(
            &engine,
            scripted(vec![]).build().unwrap(),
            vec![Case {
                name: "answer".into(),
                inputs: vec!["hello".into()],
                timeout: Duration::from_secs(10),
            }],
            2,
        )
        .await
        .unwrap();
    assert!(results[0].error.is_none(), "{results:?}");
    assert!(results[0].scores.iter().all(|s| s.pass));
    assert_eq!(results[0].evidence.answer, "Done.");
    let session = engine
        .resume(results[0].session_id.parse().unwrap())
        .await
        .unwrap();
    let events = session.events_after(0).await.unwrap();
    let trace = everruns::trace::Trace::project(&events).unwrap();
    assert_eq!(trace.turns.len(), 1);
    assert_eq!(trace.turns[0].status, "completed");
    assert!(!trace.steps.is_empty());
    assert!(everruns::trace::Trace::project(events.iter().rev()).is_err());
}
#[cfg(feature = "triggers")]
#[tokio::test]
async fn triggers_retain_changes_dedup_and_route_across_reopen() {
    use everruns::{hosting::AgentBuild, triggers::*};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("outbox.sqlite");
    let build = AgentBuild {
        agent: "watcher".into(),
        build_id: "v1".into(),
    };
    let engine = Engine::builder()
        .register(build.clone(), scripted(vec![]).build().unwrap())
        .build();
    let trigger = Trigger {
        key: "ci/v1".into(),
        build,
        message: "CI: {{payload.status}}".into(),
        filter: Filter {
            conditions: vec![],
            changes_only: Some(ChangesOnly {
                path: "payload.status".into(),
            }),
        },
        routing: Routing::PerSubject,
    };
    let dispatcher =
        Dispatcher::new(&engine, SqliteStore::open(&path).unwrap(), trigger.clone()).unwrap();
    let event = |id: &str, status: &str| Event {
        id: id.into(),
        source: "test".into(),
        kind: "ci".into(),
        subject: Some("app/main".into()),
        context: json!({"payload":{"status":status}}),
    };
    assert!(matches!(
        dispatcher.dispatch(event("1", "failure")).await.unwrap(),
        Outcome::Filtered { .. }
    ));
    let Outcome::Dispatched { session, input } =
        dispatcher.dispatch(event("2", "success")).await.unwrap()
    else {
        panic!()
    };
    engine
        .resume(session)
        .await
        .unwrap()
        .admission(input)
        .await
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    drop(dispatcher);
    let dispatcher = Dispatcher::new(&engine, SqliteStore::open(&path).unwrap(), trigger).unwrap();
    assert_eq!(
        dispatcher.dispatch(event("2", "success")).await.unwrap(),
        Outcome::Duplicate
    );
    assert!(matches!(
        dispatcher.dispatch(event("3", "success")).await.unwrap(),
        Outcome::Filtered { .. }
    ));
    let Outcome::Dispatched {
        session: second,
        input,
    } = dispatcher.dispatch(event("4", "failure")).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(session, second);
    engine
        .resume(second)
        .await
        .unwrap()
        .admission(input)
        .await
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(dispatcher.recover().await.unwrap().is_empty());
}

#[cfg(feature = "triggers")]
#[tokio::test]
async fn trigger_recovery_retries_original_input_and_filtered_receipts_survive_pruning() {
    use everruns::{hosting::AgentBuild, triggers::*};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let store = SqliteStore::open(&path).unwrap();
    let build = AgentBuild {
        agent: "late".into(),
        build_id: "v1".into(),
    };
    let engine = Engine::new();
    let trigger = Trigger {
        key: "recovery/v1".into(),
        build: build.clone(),
        message: "Original {{payload.value}}".into(),
        filter: Filter {
            conditions: vec![Condition {
                path: "payload.value".into(),
                any_of: vec![json!("yes")],
            }],
            changes_only: None,
        },
        routing: Routing::Shared,
    };
    let dispatcher = Dispatcher::new(&engine, store.clone(), trigger.clone()).unwrap();
    let event = |id: &str, value: &str| Event {
        id: id.into(),
        source: "test".into(),
        kind: "change".into(),
        subject: None,
        context: json!({"payload":{"value":value}}),
    };
    assert!(matches!(
        dispatcher.dispatch(event("filtered", "no")).await.unwrap(),
        Outcome::Filtered { .. }
    ));
    let mut stored = store.load(&trigger.key).await.unwrap();
    stored.state.deliveries.clear();
    assert!(
        store
            .compare_exchange(&trigger.key, stored.revision, &stored.state, None)
            .await
            .unwrap()
    );
    assert_eq!(
        dispatcher.dispatch(event("filtered", "yes")).await.unwrap(),
        Outcome::Duplicate
    );
    assert!(dispatcher.dispatch(event("pending", "yes")).await.is_err());
    drop(dispatcher);
    engine
        .register_agent(build, scripted(vec![]).build().unwrap())
        .unwrap();
    let dispatcher = Dispatcher::new(&engine, SqliteStore::open(&path).unwrap(), trigger).unwrap();
    let recovered = dispatcher.recover().await.unwrap();
    assert_eq!(recovered.len(), 1);
    let Outcome::Dispatched { session, input } = recovered[0] else {
        panic!()
    };
    let session = engine.resume(session).await.unwrap();
    session
        .admission(input)
        .await
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    let history = session.history().page().await.unwrap();
    assert!(format!("{history:?}").contains("Original yes"));
    assert_eq!(
        dispatcher.dispatch(event("pending", "yes")).await.unwrap(),
        Outcome::Duplicate
    );
}
#[cfg(feature = "triggers")]
#[tokio::test]
async fn concurrent_trigger_deliveries_admit_one_canonical_input() {
    use everruns::{hosting::AgentBuild, triggers::*};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite");
    let build = AgentBuild {
        agent: "concurrent".into(),
        build_id: "1".into(),
    };
    let engine = Engine::builder()
        .register(build.clone(), scripted(vec![]).build().unwrap())
        .build();
    let spec = Trigger {
        key: "concurrent/v1".into(),
        build,
        message: "hello".into(),
        filter: Filter::default(),
        routing: Routing::Shared,
    };
    let first = Dispatcher::new(&engine, SqliteStore::open(&path).unwrap(), spec.clone()).unwrap();
    let second = Dispatcher::new(&engine, SqliteStore::open(&path).unwrap(), spec).unwrap();
    let event = Event {
        id: "same-delivery".into(),
        source: "test".into(),
        kind: "test".into(),
        subject: None,
        context: json!({}),
    };
    let (a, b) = tokio::join!(first.dispatch(event.clone()), second.dispatch(event));
    let outcomes = [a.unwrap(), b.unwrap()];
    let ids = outcomes
        .into_iter()
        .filter_map(|o| match o {
            Outcome::Dispatched { session, input } => Some((session, input)),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), 1);
    let (session, input) = ids.into_iter().next().unwrap();
    let session = engine.resume(session).await.unwrap();
    session
        .admission(input)
        .await
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(
        session
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .filter(|e| e.event_type() == "input.message")
            .count(),
        1
    );
}
#[cfg(feature = "triggers")]
#[test]
fn source_adapters_verify_raw_bytes_before_normalization() {
    use everruns::triggers::*;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    let body=br#"{"action":"opened","repository":{"full_name":"Acme/App"},"pull_request":{"number":7},"sender":{"login":"person"}}"#;
    let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
    mac.update(body);
    let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    let event = github(
        "secret",
        GitHubRequest {
            event: "pull_request",
            delivery: "d1",
            signature: &signature,
            body,
        },
        Some("agent[bot]"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(event.subject.as_deref(), Some("Acme/App#7"));
    assert_eq!(event.kind, "pull_request.opened");
    assert_eq!(event.context["github"]["repository_key"], "acme/app");
    let replay = github(
        "secret",
        GitHubRequest {
            event: "pull_request",
            delivery: "replaced-unsigned-header",
            signature: &signature,
            body,
        },
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(event.id, replay.id);
    assert_eq!(
        replay.context["github"]["delivery_id"],
        "replaced-unsigned-header"
    );
    assert!(
        github(
            "wrong",
            GitHubRequest {
                event: "pull_request",
                delivery: "d1",
                signature: &signature,
                body
            },
            None
        )
        .is_err()
    );
    assert!(
        github(
            "secret",
            GitHubRequest {
                event: "pull_request",
                delivery: "d1",
                signature: &signature,
                body: b"{}"
            },
            None
        )
        .is_err()
    );
    use everruns_integrations::standard_webhooks as standard;
    let secret = standard::generate_secret();
    let key = standard::decode_secret(&secret).unwrap();
    let body = br#"{"eventId":"e1","data":{"ticket":"T7"}}"#;
    let ts = "1700000000";
    let signature = standard::sign(&key, "delivery1", ts, body);
    let event = mcp(
        &secret,
        McpRequest {
            server: "tickets",
            event: "ticket.changed",
            expected_subscription: "s1",
            subscription: "s1",
            headers: SignedHeaders {
                id: Some("delivery1"),
                timestamp: Some(ts),
                signature: Some(&signature),
            },
            body,
            now: 1700000000,
            subject: Some("{{payload.ticket}}"),
        },
    )
    .unwrap();
    assert_eq!(event.subject.as_deref(), Some("T7"));
    assert!(
        mcp(
            &secret,
            McpRequest {
                server: "tickets",
                event: "ticket.changed",
                expected_subscription: "s2",
                subscription: "s1",
                headers: SignedHeaders {
                    id: Some("delivery1"),
                    timestamp: Some(ts),
                    signature: Some(&signature)
                },
                body,
                now: 1700000000,
                subject: None
            }
        )
        .is_err()
    );
    assert!(
        mcp(
            &secret,
            McpRequest {
                server: "tickets",
                event: "ticket.changed",
                expected_subscription: "s1",
                subscription: "s1",
                headers: SignedHeaders {
                    id: Some("delivery1"),
                    timestamp: Some(ts),
                    signature: Some(&signature)
                },
                body,
                now: i64::MAX,
                subject: None
            }
        )
        .is_err()
    );
}
#[cfg(feature = "evaluation")]
#[tokio::test]
async fn file_scoring_uses_shared_explanations_and_requires_retained_evidence() {
    use everruns::evaluation::{Evaluator, Evidence, Scorer};
    let evaluator = Evaluator::new(vec![Scorer::FileContains {
        path: "report.txt".into(),
        text: "ready".into(),
        weight: 1.0,
    }])
    .unwrap();
    let mut evidence = Evidence::default();
    evidence
        .files
        .insert("report.txt".into(), "Result: ready".into());
    let scores = evaluator.score(&evidence).await.unwrap();
    assert!(scores[0].pass);
    assert_eq!(scores[0].reason, "File 'report.txt' contains 'ready'");
    evidence.files.clear();
    assert!(
        evaluator
            .score(&evidence)
            .await
            .unwrap_err()
            .to_string()
            .contains("File evidence unavailable")
    );
}

#[cfg(feature = "evaluation")]
#[tokio::test]
async fn evaluation_file_evidence_shares_the_total_retention_budget() {
    use everruns::evaluation::{Evaluator, Evidence, Scorer};
    let evaluator = Evaluator::new(vec![Scorer::Contains {
        text: "answer".into(),
        weight: 1.0,
    }])
    .unwrap();
    let mut evidence = Evidence::default();
    evidence
        .files
        .insert("one".into(), "a".repeat(5 * 1024 * 1024));
    evidence
        .files
        .insert("two".into(), "b".repeat(5 * 1024 * 1024));
    assert!(
        evaluator
            .score(&evidence)
            .await
            .unwrap_err()
            .to_string()
            .contains("retention limit")
    );
}
#[cfg(feature = "evaluation")]
#[tokio::test]
async fn online_scoring_uses_complete_evidence_and_flushes_results() {
    use everruns::{capability::async_trait, evaluation::*};
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    type RecordedScores = Arc<Mutex<Vec<Result<Vec<Score>, String>>>>;
    #[derive(Clone, Default)]
    struct Sink(RecordedScores);
    #[async_trait]
    impl ScoreSink for Sink {
        async fn record(&self, _: &str, _: &str, result: Result<Vec<Score>, Error>) {
            self.0
                .lock()
                .unwrap()
                .push(result.map_err(|e| e.to_string()));
        }
    }
    let sink = Sink::default();
    let evaluator = Evaluator::new(vec![Scorer::Contains {
        text: "Done".into(),
        weight: 1.0,
    }])
    .unwrap();
    let engine = Engine::builder()
        .listener(OnlineScoring::new(evaluator, sink.clone()))
        .build();
    let session = engine.create(scripted(vec![]).build().unwrap());
    session.run("hi").await.unwrap();
    engine.shutdown(Duration::from_secs(5)).await;
    let values = sink.0.lock().unwrap();
    assert_eq!(values.len(), 1, "{values:?}");
    assert!(values[0].as_ref().unwrap()[0].pass);
}
#[cfg(feature = "evaluation")]
#[tokio::test]
async fn evaluation_models_are_explicit_and_grader_failures_are_errors() {
    use everruns::evaluation::*;
    let mut evidence = Evidence::default();
    evidence.answer = "42".into();
    let rules = vec![Scorer::Judge {
        rubric: "Answer arithmetic correctly".into(),
        model_id: None,
        pass_threshold: 0.8,
        weight: 1.0,
    }];
    assert!(
        Evaluator::new(rules.clone())
            .unwrap()
            .score(&evidence)
            .await
            .is_err()
    );
    let scores = Evaluator::new(rules)
        .unwrap()
        .judge(Model::simulated(
            "```json\n{\"value\":0.9,\"reasoning\":\"correct\"}\n```",
        ))
        .score(&evidence)
        .await
        .unwrap();
    assert!(scores[0].pass);
    assert_eq!(scores[0].value, 0.9);
    assert!(
        Evaluator::new(vec![Scorer::Regex {
            pattern: "[".into(),
            weight: 1.0
        }])
        .is_err()
    );
}

#[cfg(all(feature = "channels", feature = "triggers"))]
mod advanced_slack {
    use super::*;
    use everruns::channels::*;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    fn signed(mut request: ChannelRequest) -> ChannelRequest {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();
        let mut mac = Hmac::<Sha256>::new_from_slice(b"signing-secret").unwrap();
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(&request.body);
        request = request
            .header("x-slack-request-timestamp", timestamp)
            .header(
                "x-slack-signature",
                format!("v0={}", hex::encode(mac.finalize().into_bytes())),
            );
        request
    }
    fn message(kind: &str, text: &str) -> ChannelRequest {
        signed(ChannelRequest::json(
            &json!({"type":"event_callback","team_id":"T1","event":{"type":kind,"channel":"C1","user":"U1","ts":"2.0","thread_ts":"1.0","text":text}}),
        ))
    }
    fn interaction(key: &str, user: &str) -> ChannelRequest {
        let payload = json!({"type":"block_actions","team":{"id":"T1"},"user":{"id":user},"channel":{"id":"C1"},"actions":[{"action_id":"everruns_approve","value":key}]});
        let body = format!(
            "payload={}",
            payload
                .to_string()
                .replace('%', "%25")
                .replace('&', "%26")
                .replace('+', "%2B")
        );
        signed(ChannelRequest {
            path: String::new(),
            headers: vec![(
                "content-type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
            body: body.into_bytes(),
        })
    }
    #[tokio::test]
    async fn signatures_workspace_scope_thread_binding_and_participation_are_enforced() {
        let slack = Slack::new("bot-secret", "signing-secret")
            .workspace("T1")
            .agent_pane()
            .participation(SlackAdmission::Direct);
        let debug = format!("{slack:?}");
        assert!(!debug.contains("bot-secret"));
        assert!(!debug.contains("signing-secret"));
        let Inbound::Message(message) = slack
            .receive(&message("message", "ambient chat"))
            .await
            .unwrap()
        else {
            panic!()
        };
        assert!(!slack.admit(&message, None).await.unwrap());
        assert_eq!(
            slack.binding(&message.route(), SessionBinding::Shared),
            SessionBinding::Thread
        );
        let Inbound::Message(mention) = slack
            .receive(&self::message("app_mention", "<@B1> help"))
            .await
            .unwrap()
        else {
            panic!()
        };
        assert!(slack.admit(&mention, None).await.unwrap());
        let bad = ChannelRequest::json(&json!({"type":"event_callback"}));
        assert!(slack.receive(&bad).await.is_err());
        let wrong = signed(ChannelRequest::json(
            &json!({"type":"event_callback","team_id":"T2","event":{"type":"message"}}),
        ));
        assert!(slack.receive(&wrong).await.is_err());
        let extreme = ChannelRequest::json(&json!({}))
            .header("x-slack-request-timestamp", i64::MIN.to_string())
            .header("x-slack-signature", "v0=00");
        assert!(slack.receive(&extreme).await.is_err());
    }
    #[tokio::test]
    async fn approval_cards_bind_requester_are_one_use_and_are_superseded_by_new_input() {
        let api = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat.postMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"3.0"})))
            .mount(&api)
            .await;
        let slack = Slack::new("bot", "signing-secret")
            .workspace("T1")
            .approval_buttons()
            .api_base(api.uri());
        let Inbound::Message(input) = slack
            .receive(&message("app_mention", "help"))
            .await
            .unwrap()
        else {
            panic!()
        };
        let context = slack.delivery_context(&input.reply_to);
        let prompt = ApprovalPrompt {
            action: "Delete the draft".into(),
            question: None,
            turn_id: Some("turn_asked".into()),
        };
        assert!(matches!(
            slack
                .approvals()
                .unwrap()
                .prompt(everruns::SessionId::new(), &prompt, &context)
                .await,
            DeliveryResult::Ok
        ));
        let requests = api.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        let key = body["blocks"][1]["elements"][0]["value"].as_str().unwrap();
        assert!(slack.receive(&interaction(key, "OTHER")).await.is_err());
        let Inbound::Message(answer) = slack.receive(&interaction(key, "U1")).await.unwrap() else {
            panic!()
        };
        assert!(answer.event.text.contains("Delete the draft"));
        assert_eq!(answer.event.actor.actor_id, "U1");
        assert!(matches!(
            slack.receive(&interaction(key, "U1")).await.unwrap(),
            Inbound::Ignore
        ));
        slack
            .approvals()
            .unwrap()
            .prompt(everruns::SessionId::new(), &prompt, &context)
            .await;
        let requests = api.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[1].body).unwrap();
        let key = body["blocks"][1]["elements"][0]["value"].as_str().unwrap();
        slack
            .receive(&message("message", "Never mind"))
            .await
            .unwrap();
        assert!(matches!(
            slack.receive(&interaction(key, "U1")).await.unwrap(),
            Inbound::Ignore
        ));
    }
    #[tokio::test]
    async fn shared_stream_surface_retracts_after_stopping_and_bounds_thread_history() {
        let api = MockServer::start().await;
        for endpoint in [
            "chat.startStream",
            "chat.appendStream",
            "chat.stopStream",
            "chat.update",
            "agents.sessions.setStatus",
            "agents.sessions.rename",
        ] {
            Mock::given(method("POST"))
                .and(path(format!("/{endpoint}")))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"3.0"})),
                )
                .mount(&api)
                .await;
        }
        Mock::given(method("POST")).and(path("/conversations.replies")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"messages":[{"ts":"1.0","user":"U0","text":"root"},{"ts":"1.5","user":"U0","text":"earlier"},{"ts":"2.0","user":"U1","text":"current"},{"ts":"3.0","bot_id":"B1","text":"bot"}]}))).mount(&api).await;
        let slack = Slack::new("bot", "signing-secret")
            .agent_pane()
            .thread_history(2)
            .api_base(api.uri());
        let Inbound::Message(input) = slack
            .receive(&message("app_mention", "help"))
            .await
            .unwrap()
        else {
            panic!()
        };
        let history = slack.backfill(&input).await.unwrap();
        assert_eq!(history.len(), 2);
        assert!(
            history
                .iter()
                .all(|m| m.text != "current" && m.text != "bot")
        );
        let context = slack.delivery_context(&input.reply_to);
        let stream = slack.streaming().unwrap();
        let handle = stream.start(&context).await.unwrap();
        assert!(matches!(
            stream.append(&handle, "Hello <!channel>", &context).await,
            DeliveryResult::Ok
        ));
        assert!(matches!(
            stream.replace(&handle, "Retracted", &context).await,
            DeliveryResult::Ok
        ));
        assert!(matches!(
            slack
                .agent_surface()
                .unwrap()
                .set_status("working", &context)
                .await,
            DeliveryResult::Ok
        ));
        assert!(matches!(
            slack
                .agent_surface()
                .unwrap()
                .set_title(&"t".repeat(300), &context)
                .await,
            DeliveryResult::Ok
        ));
        let requests = api.received_requests().await.unwrap();
        let methods = requests.iter().map(|r| r.url.path()).collect::<Vec<_>>();
        let stop = methods
            .iter()
            .position(|p| *p == "/chat.stopStream")
            .unwrap();
        let update = methods.iter().position(|p| *p == "/chat.update").unwrap();
        assert!(stop < update);
        let append = requests
            .iter()
            .find(|r| r.url.path() == "/chat.appendStream")
            .unwrap();
        assert!(!String::from_utf8_lossy(&append.body).contains("<!channel>"));
        let rename = requests
            .iter()
            .find(|r| r.url.path() == "/agents.sessions.rename")
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&rename.body).unwrap()["title"]
                .as_str()
                .unwrap()
                .len(),
            250
        );
    }
}

#[cfg(all(feature = "local", feature = "channels", feature = "evaluation"))]
#[tokio::test]
async fn registered_channels_and_evaluations_run_on_explicit_sqlite_engine_profiles() {
    use everruns::{
        channels::{ChannelRequest, Channels, Webhook},
        evaluation::{Case, Evaluator, Scorer},
        hosting::{AgentBuild, SqliteHosting},
    };
    use std::time::Duration;
    let root = tempfile::tempdir().unwrap();
    let build = AgentBuild {
        agent: "hosted-service".into(),
        build_id: "v1".into(),
    };
    let profile = SqliteHosting::open(root.path().join("authority.sqlite")).unwrap();
    let engine = Engine::builder()
        .hosting(profile)
        .register(build.clone(), scripted(vec![]).build().unwrap())
        .build();
    let channels = Channels::builder(&engine)
        .channel_registered("web", Webhook::new(), build.clone())
        .build();
    let response = channels
        .handle(
            "web",
            &ChannelRequest::json(&json!({"text":"hello","thread":"one"})),
        )
        .await;
    assert_eq!(response.status, 200, "{:?}", response.body);
    let evaluator = Evaluator::new(vec![Scorer::Contains {
        text: "Done".into(),
        weight: 1.0,
    }])
    .unwrap();
    let results = evaluator
        .run_registered(
            &engine,
            build.clone(),
            vec![Case {
                name: "hosted".into(),
                inputs: vec!["hello".into()],
                timeout: Duration::from_secs(10),
            }],
            1,
        )
        .await
        .unwrap();
    assert!(results[0].error.is_none(), "{results:?}");
    assert!(results[0].scores[0].pass);
    assert_eq!(results[0].build.as_ref(), Some(&build));
}
