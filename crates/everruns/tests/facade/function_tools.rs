//! Consumer-facing test for function tools (EVE-828).
//!
//! Proves the "done when" criterion: a library user registers a custom tool by
//! passing an async closure, importing only the `everruns` facade — never
//! `CapabilityRegistry`, `ToolDefinition`, `ToolContextServices`, or
//! `PlatformStore`.

use everruns::{Agent, BuildError, FunctionTool, Model, ToolResponse};
use serde_json::{Value, json};

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "city": { "type": "string" } },
        "required": ["city"],
    })
}

#[test]
fn registers_function_and_capability_via_separate_public_entrypoints() {
    let weather = FunctionTool::new(
        "get_weather",
        "Look up the weather for a city.",
        schema(),
        |args: Value| async move {
            let city = args["city"].as_str().unwrap_or("unknown");
            if city.is_empty() {
                return Ok(ToolResponse::error("city must not be empty"));
            }
            Ok::<_, String>(ToolResponse::json(json!({ "city": city, "sky": "clear" })))
        },
    );

    // Function tools and capability references remain distinct public inputs.
    let agent = Agent::builder()
        .instructions("You are a helpful weather assistant.")
        .model(Model::simulated("Clear skies."))
        .capability("current_time")
        .tool(weather)
        .build();

    assert!(
        agent.is_ok(),
        "public-API registration must build: {agent:?}"
    );
}

#[test]
fn duplicate_tool_names_are_rejected_at_build() {
    let make = || {
        FunctionTool::new("lookup", "desc", schema(), |_: Value| async move {
            Ok::<_, String>(json!({}))
        })
    };

    let err = Agent::builder()
        .instructions("Concise.")
        .model(Model::simulated("ok"))
        .tool(make())
        .tool(make())
        .build()
        .expect_err("duplicate names must fail");

    assert_eq!(
        err,
        BuildError::DuplicateTool {
            name: "lookup".to_string()
        }
    );
}

// --- Call context and per-tool approval ---------------------------------

mod context_and_approval {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use everruns::approval::{ApprovalDecision, ToolApprover, async_trait};
    use everruns::{
        Agent, BuildError, FunctionTool, InMemoryEngine, LlmSimConfig, Model, SessionEvent,
        SessionEventKind, SessionId, ToolCall, ToolCallContext, ToolDefinition,
    };
    use serde_json::{Value, json};

    fn calling(tool: &str, arguments: Value) -> Model {
        Model::simulated_with_config(LlmSimConfig::fixed("Done.").with_tool_call_sequence(vec![
            vec![ToolCall {
                id: "call_1".to_string(),
                name: tool.to_string(),
                arguments,
            }],
            vec![],
        ]))
    }

    fn drain(stream: &mut everruns::EventStream) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        while let Some(event) = stream.try_recv().expect("lossless") {
            events.push(event);
        }
        events
    }

    fn tool_completed(events: &[SessionEvent]) -> (bool, String) {
        events
            .iter()
            .find_map(|event| match &event.kind {
                SessionEventKind::ToolCompleted { success, .. } => {
                    Some((*success, event.canonical_json().to_string()))
                }
                _ => None,
            })
            .expect("a tool.completed event")
    }

    #[tokio::test]
    async fn context_tool_sees_its_call_and_reports_progress() {
        type Seen = Option<(SessionId, Option<String>, String)>;
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let recorder = seen.clone();
        let tool = FunctionTool::with_context(
            "export",
            "Export a report.",
            json!({ "type": "object", "properties": {} }),
            move |ctx: ToolCallContext, _args: Value| {
                let recorder = recorder.clone();
                async move {
                    ctx.progress("rendering page 1").await;
                    *recorder.lock().unwrap() = Some((
                        ctx.session_id(),
                        ctx.turn_id(),
                        ctx.tool_call_id().to_string(),
                    ));
                    Ok::<_, String>(json!({ "ok": true }))
                }
            },
        );
        let agent = Agent::builder()
            .instructions("Export when asked.")
            .model(calling("export", json!({})))
            .tool(tool)
            .build()
            .expect("valid agent");
        let session = InMemoryEngine::new().create(agent);
        let mut stream = session.events();

        let turn = session.run("export").await.expect("turn runs");
        assert!(turn.success, "{:?}", turn.error);

        let (session_id, turn_id, call_id) = seen.lock().unwrap().clone().expect("handler ran");
        assert_eq!(session_id, session.session_id());
        assert_eq!(call_id, "call_1");
        let events = drain(&mut stream);
        let progress = events
            .iter()
            .find_map(|event| match &event.kind {
                SessionEventKind::ToolProgress {
                    tool_call_id,
                    tool_name,
                    message,
                } => Some((
                    tool_call_id.clone(),
                    tool_name.clone(),
                    message.clone(),
                    event.turn_id.clone(),
                )),
                _ => None,
            })
            .expect("a tool.progress event");
        assert_eq!(progress.0, "call_1");
        assert_eq!(progress.1, "export");
        assert_eq!(progress.2, "rendering page 1");
        assert!(turn_id.is_some(), "the handler sees the turn");
        assert_eq!(progress.3, turn_id, "progress is correlated with the turn");
    }

    #[derive(Clone)]
    struct Scripted {
        decision: ApprovalDecision,
        asked: Arc<Mutex<Vec<(SessionId, String, Value)>>>,
    }

    impl Scripted {
        fn new(decision: ApprovalDecision) -> Self {
            Self {
                decision,
                asked: Arc::default(),
            }
        }
    }

    #[async_trait]
    impl ToolApprover for Scripted {
        async fn approve(
            &self,
            session_id: SessionId,
            call: &ToolCall,
            _definition: &ToolDefinition,
        ) -> ApprovalDecision {
            self.asked
                .lock()
                .unwrap()
                .push((session_id, call.id.clone(), call.arguments.clone()));
            self.decision
        }
    }

    fn gated_sql(runs: Arc<AtomicUsize>) -> FunctionTool {
        FunctionTool::new(
            "run_sql",
            "Run SQL.",
            json!({ "type": "object", "properties": { "sql": { "type": "string" } } }),
            move |_args: Value| {
                let runs = runs.clone();
                async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, String>(json!({ "rows": 0 }))
                }
            },
        )
        .needs_approval(|args| {
            args["sql"]
                .as_str()
                .is_some_and(|sql| sql.to_lowercase().contains("drop"))
        })
    }

    async fn run_sql(sql: &str, approver: Scripted) -> (bool, String, usize) {
        let runs = Arc::new(AtomicUsize::new(0));
        let agent = Agent::builder()
            .instructions("Run SQL when asked.")
            .model(calling("run_sql", json!({ "sql": sql })))
            .tool(gated_sql(runs.clone()))
            .approver(approver)
            .build()
            .expect("valid agent");
        let session = InMemoryEngine::new().create(agent);
        let mut stream = session.events();
        let turn = session.run("go").await.expect("turn runs");
        assert!(turn.success, "{:?}", turn.error);
        let (success, completed) = tool_completed(&drain(&mut stream));
        (success, completed, runs.load(Ordering::SeqCst))
    }

    #[tokio::test]
    async fn approved_call_runs_and_the_approver_can_route_it() {
        let approver = Scripted::new(ApprovalDecision::Allow);
        let (success, _, runs) = run_sql("DROP TABLE t", approver.clone()).await;
        assert!(success);
        assert_eq!(runs, 1);
        let asked = approver.asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].1, "call_1", "the approver sees the tool call id");
        assert_eq!(asked[0].2, json!({ "sql": "DROP TABLE t" }));
    }

    #[tokio::test]
    async fn rejected_call_never_runs_and_the_model_is_told() {
        let approver = Scripted::new(ApprovalDecision::Reject);
        let (success, completed, runs) = run_sql("DROP TABLE t", approver.clone()).await;
        assert!(!success);
        assert_eq!(runs, 0, "a rejected handler must not run");
        assert!(completed.contains("rejected"), "{completed}");
        assert_eq!(approver.asked.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn predicate_false_skips_the_approver() {
        let approver = Scripted::new(ApprovalDecision::Reject);
        let (success, _, runs) = run_sql("SELECT 1", approver.clone()).await;
        assert!(success);
        assert_eq!(runs, 1);
        assert!(approver.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn a_gated_tool_without_an_approver_fails_to_build() {
        let error = Agent::builder()
            .instructions("Run SQL.")
            .model(Model::simulated("ok"))
            .tool(gated_sql(Arc::default()))
            .build()
            .expect_err("gated tool without approver");
        assert_eq!(
            error,
            BuildError::MissingApprover {
                tool: "run_sql".to_string()
            }
        );

        let always = FunctionTool::new(
            "wipe",
            "Wipe.",
            json!({ "type": "object" }),
            |_: Value| async move { Ok::<_, String>(json!({})) },
        )
        .always_needs_approval();
        assert!(matches!(
            Agent::builder()
                .instructions("Wipe.")
                .model(Model::simulated("ok"))
                .tool(always)
                .build(),
            Err(BuildError::MissingApprover { .. })
        ));
    }

    #[test]
    fn an_approver_without_gated_tools_builds() {
        let agent = Agent::builder()
            .instructions("Concise.")
            .model(Model::simulated("ok"))
            .approver(Scripted::new(ApprovalDecision::Reject))
            .build();
        assert!(agent.is_ok(), "{agent:?}");
    }
}
