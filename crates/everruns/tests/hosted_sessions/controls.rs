//! Pending tools, protocol controls and snapshot conformance.
use super::*;

async fn parked_client_calls_reopen_and_resolve(profile: impl SessionHosting + Clone) {
    let completions = Arc::new(AtomicUsize::new(0));
    let counted = completions.clone();
    let agent = Agent::builder()
        .instructions("Use the client tools.")
        .model(Model::simulated_with_config(
            everruns::LlmSimConfig::fixed("Both resolved.").with_tool_call_sequence(vec![
                vec![
                    everruns::ToolCall {
                        id: "call_one".into(),
                        name: "client_one".into(),
                        arguments: serde_json::json!({}),
                    },
                    everruns::ToolCall {
                        id: "call_two".into(),
                        name: "client_two".into(),
                        arguments: serde_json::json!({}),
                    },
                ],
                vec![],
            ]),
        ))
        .client_tool(
            "client_one",
            "First external operation",
            serde_json::json!({"type":"object","properties":{}}),
        )
        .client_tool(
            "client_two",
            "Second external operation",
            serde_json::json!({"type":"object","properties":{}}),
        )
        .on_completion(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            async {}
        })
        .build()
        .unwrap();
    let first = engine(profile.clone(), agent.clone());
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    let message = MessageId::new();
    session.send_with_id(message, "call both").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if session.pending_tools().await.unwrap().len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(session);
    drop(first);
    let replacement = engine(profile, agent);
    let session = replacement.resume(id).await.unwrap();
    let admission = session.admission(message).await.unwrap().unwrap();
    assert!(
        matches!(admission.state().await.unwrap(),AdmissionState::Waiting{calls,..} if calls.len()==2)
    );
    assert_eq!(completions.load(Ordering::SeqCst), 0);
    assert!(session.interrupted_turn().await.unwrap().is_none());
    assert!(session.resume_interrupted_turn().await.unwrap().is_none());
    let turn_id = replacement
        .session_status(id)
        .await
        .unwrap()
        .open_turn
        .unwrap();
    let command = uuid::Uuid::new_v4();
    let first = everruns::ToolAnswer {
        tool_call_id: "call_one".into(),
        tool_name: "client_one".into(),
        result: serde_json::json!(1),
        error: None,
    };
    session
        .resolve_tools(command, turn_id, vec![first.clone()])
        .await
        .unwrap();
    session
        .resolve_tools(command, turn_id, vec![first])
        .await
        .unwrap();
    assert_eq!(session.pending_tools().await.unwrap().len(), 1);
    let wrong = everruns::ToolAnswer {
        tool_call_id: "call_two".into(),
        tool_name: "wrong".into(),
        result: serde_json::json!(2),
        error: None,
    };
    assert!(
        session
            .resolve_tools(uuid::Uuid::new_v4(), turn_id, vec![wrong])
            .await
            .is_err()
    );
    let second = everruns::ToolAnswer {
        tool_call_id: "call_two".into(),
        tool_name: "client_two".into(),
        result: serde_json::json!(2),
        error: None,
    };
    session
        .resolve_tools(uuid::Uuid::new_v4(), turn_id, vec![second])
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), admission.wait())
            .await
            .unwrap()
            .unwrap()
            .response,
        "Both resolved."
    );
    assert_eq!(completions.load(Ordering::SeqCst), 1);
    let events = session.events_after(0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "turn.started")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "tool.completed")
            .count(),
        2
    );
}

#[tokio::test]
async fn memory_durable_client_calls_reopen_and_partial_resolution_stays_parked() {
    parked_client_calls_reopen_and_resolve(MemoryHosting::default()).await;
}

#[tokio::test(start_paused = true)]
async fn explicitly_idempotent_completion_recovers_under_the_same_invocation_identity() {
    let profile = MemoryHosting::default();
    let seen = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    let attempts = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let effects = seen.clone();
    let counted = attempts.clone();
    let signal = entered.clone();
    let agent = Agent::builder()
        .instructions("Answer.")
        .model(Model::simulated("done"))
        .on_completion_with_recovery(everruns::HookRecovery::Idempotent, move |context| {
            let effects = effects.clone();
            let counted = counted.clone();
            let signal = signal.clone();
            async move {
                effects.lock().unwrap().insert(context.turn.turn_id);
                if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                    signal.notify_one();
                    std::future::pending::<()>().await;
                }
            }
        })
        .build()
        .unwrap();
    let make = || engine(External(profile.clone()), agent.clone());
    let first = make();
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    let message = MessageId::new();
    session.send_with_id(message, "hello").await.unwrap();
    let task = tokio::spawn(async move { first.activate(id).await });
    entered.notified().await;
    task.abort();
    let _ = task.await;
    tokio::time::advance(std::time::Duration::from_secs(11)).await;
    let replacement = make();
    replacement.activate(id).await.unwrap();
    let turn = replacement
        .resume(id)
        .await
        .unwrap()
        .admission(message)
        .await
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(turn.hook_failures.is_empty());
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn reconstructed_outcome_preserves_iteration_limit_and_shared_reply_projection() {
    let profile = MemoryHosting::default();
    let agent = Agent::builder()
        .instructions("Use tools.")
        .max_iterations(1)
        .model(Model::simulated_with_config(
            everruns::LlmSimConfig::scripted(vec![everruns::SimTurn::Mixed {
                text: "Checking.".into(),
                tool_calls: vec![everruns::SimToolCall {
                    name: "external".into(),
                    arguments: serde_json::json!({}),
                    id: Some("call_limit".into()),
                }],
            }]),
        ))
        .client_tool(
            "external",
            "External operation",
            serde_json::json!({"type":"object","properties":{}}),
        )
        .build()
        .unwrap();
    let first = engine(profile.clone(), agent.clone());
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    let message = MessageId::new();
    let expected = session
        .send_with_id(message, "hello")
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(
        expected.stop_reason,
        everruns::TurnStopReason::MaxTurnRequests
    );
    assert_eq!(
        expected.response, "",
        "working commentary is excluded by the shared reply projection"
    );
    drop(session);
    drop(first);
    let recovered = engine(profile, agent)
        .resume(id)
        .await
        .unwrap()
        .admission(message)
        .await
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(recovered.stop_reason, expected.stop_reason);
    assert_eq!(recovered.response, expected.response);
}

#[cfg(feature = "local")]
#[tokio::test]
async fn native_durable_client_calls_reopen_and_partial_resolution_stays_parked() {
    let directory = tempfile::tempdir().unwrap();
    parked_client_calls_reopen_and_resolve(
        everruns::hosting::SqliteHosting::open(directory.path().join("authority.db")).unwrap(),
    )
    .await;
}

#[cfg(feature = "builtins")]
async fn approval_survives_frontend_loss(profile: impl SessionHosting + Clone) {
    let effects = Arc::new(AtomicUsize::new(0));
    let counted = effects.clone();
    let agent = Agent::builder()
        .instructions("Request the action, then retry only after approval.")
        .model(Model::simulated_with_config(
            everruns::LlmSimConfig::fixed("Approved action done.").with_tool_call_sequence(vec![
                vec![everruns::ToolCall {
                    id: "call_approval_first".into(),
                    name: "mutate".into(),
                    arguments: serde_json::json!({}),
                }],
                vec![everruns::ToolCall {
                    id: "call_approval_retry".into(),
                    name: "mutate".into(),
                    arguments: serde_json::json!({}),
                }],
                vec![],
            ]),
        ))
        .tool(
            everruns::FunctionTool::new(
                "mutate",
                "Change one external value",
                serde_json::json!({"type":"object","properties":{}}),
                move |_: serde_json::Value| {
                    counted.fetch_add(1, Ordering::SeqCst);
                    async { Ok::<_, String>(serde_json::json!({"done":true})) }
                },
            )
            .always_needs_approval(),
        )
        .durable_approvals()
        .build()
        .unwrap();
    let first = engine(profile.clone(), agent.clone());
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    let message = MessageId::new();
    session.send_with_id(message, "do it").await.unwrap();
    let request = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(call) = session
                .pending_tools()
                .await
                .unwrap()
                .into_iter()
                .find(|call| call.name == "approve_tool_call")
            {
                break call;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    drop(session);
    drop(first);
    let replacement = engine(profile, agent);
    let session = replacement.resume(id).await.unwrap();
    let turn_id = replacement
        .session_status(id)
        .await
        .unwrap()
        .open_turn
        .unwrap();
    let bypass = everruns::ToolAnswer {
        tool_call_id: request.id.clone(),
        tool_name: request.name.clone(),
        result: serde_json::json!({"approved":true}),
        error: None,
    };
    assert!(
        session
            .resolve_tools(uuid::Uuid::new_v4(), turn_id, vec![bypass])
            .await
            .is_err()
    );
    // A stale member must not consume the valid permission before the whole
    // batch is validated, including on the volatile authority.
    assert!(
        session
            .approve_tools(
                uuid::Uuid::new_v4(),
                vec![
                    everruns::hosting::ApprovalCommit {
                        turn_id,
                        request_id: request.id.clone(),
                        decision: everruns::hosting::ToolApprovalDecision::Allow
                    },
                    everruns::hosting::ApprovalCommit {
                        turn_id,
                        request_id: "missing-request".into(),
                        decision: everruns::hosting::ToolApprovalDecision::Allow
                    },
                ]
            )
            .await
            .is_err()
    );
    assert_eq!(session.pending_tools().await.unwrap().len(), 1);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    let command = uuid::Uuid::new_v4();
    let decisions = vec![everruns_contracts::execution_api::ToolApprovalAnswer {
        tool_call_id: request.id.clone(),
        decision: everruns::hosting::ToolApprovalDecision::Allow,
    }];
    let receipt = session
        .submit_approvals(command, turn_id, decisions.clone())
        .await
        .unwrap();
    assert_eq!(receipt.resolved.len(), 1);
    assert_eq!(
        receipt,
        session
            .submit_approvals(command, turn_id, decisions)
            .await
            .unwrap()
    );
    assert!(
        session
            .submit_approvals(
                command,
                turn_id,
                vec![everruns_contracts::execution_api::ToolApprovalAnswer {
                    tool_call_id: request.id.clone(),
                    decision: everruns::hosting::ToolApprovalDecision::Reject
                }]
            )
            .await
            .is_err()
    );
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        session.admission(message).await.unwrap().unwrap().wait(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(outcome.success);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert!(session.pending_tools().await.unwrap().is_empty());
}

#[cfg(feature = "builtins")]
#[tokio::test]
async fn memory_approval_resolves_after_frontend_loss_without_duplicating_effects() {
    approval_survives_frontend_loss(MemoryHosting::default()).await;
}

#[cfg(feature = "builtins")]
async fn mixed_wait_resolution_is_atomic(profile: impl SessionHosting + Clone) {
    let effects = Arc::new(AtomicUsize::new(0));
    let counted = effects.clone();
    let agent = Agent::builder()
        .instructions("Use the tools, then finish.")
        .model(Model::simulated_with_config(
            everruns::LlmSimConfig::fixed("done").with_tool_call_sequence(vec![
                vec![
                    everruns::ToolCall {
                        id: "mutate_first".into(),
                        name: "mutate".into(),
                        arguments: serde_json::json!({}),
                    },
                    everruns::ToolCall {
                        id: "render_first".into(),
                        name: "render".into(),
                        arguments: serde_json::json!({}),
                    },
                ],
                vec![everruns::ToolCall {
                    id: "mutate_retry".into(),
                    name: "mutate".into(),
                    arguments: serde_json::json!({}),
                }],
                vec![],
            ]),
        ))
        .client_tool(
            "render",
            "Render the result",
            serde_json::json!({"type":"object"}),
        )
        .tool(
            everruns::FunctionTool::new(
                "mutate",
                "Change one value",
                serde_json::json!({"type":"object"}),
                move |_: serde_json::Value| {
                    let counted = counted.clone();
                    async move {
                        counted.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, String>(serde_json::json!({"done":true}))
                    }
                },
            )
            .always_needs_approval(),
        )
        .durable_approvals()
        .build()
        .unwrap();
    let owner = engine(profile.clone(), agent);
    let id = SessionId::new();
    let session = owner.create_registered(id, &build()).await.unwrap();
    let sent = session.send("work").await.unwrap();
    let waiting = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let status = profile.status(id).await.unwrap();
            if status.waiting.len() == 2 {
                break status;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let turn = waiting.open_turn.unwrap();
    let approval = waiting
        .waiting
        .iter()
        .find(|call| call.name == "approve_tool_call")
        .unwrap();
    let decisions = vec![everruns::hosting::ApprovalCommit {
        turn_id: turn,
        request_id: approval.id.clone(),
        decision: everruns::hosting::ToolApprovalDecision::Allow,
    }];
    let before = session.events_after(0).await.unwrap().len();
    let result = everruns::ToolAnswer {
        tool_call_id: "render_first".into(),
        tool_name: "render".into(),
        result: serde_json::json!({"rendered":true}),
        error: None,
    };
    let mut invalid = result.clone();
    invalid.tool_call_id = "not_pending".into();
    assert!(
        session
            .resolve_waits(uuid::Uuid::new_v4(), turn, decisions.clone(), vec![invalid])
            .await
            .is_err()
    );
    assert_eq!(session.events_after(0).await.unwrap().len(), before);
    assert_eq!(session.pending_tools().await.unwrap().len(), 2);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    let command = uuid::Uuid::new_v4();
    session
        .resolve_waits(command, turn, decisions.clone(), vec![result.clone()])
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(3), sent.wait())
            .await
            .unwrap()
            .unwrap()
            .success
    );
    let after = session.events_after(0).await.unwrap().len();
    session
        .resolve_waits(command, turn, decisions, vec![result])
        .await
        .unwrap();
    assert_eq!(session.events_after(0).await.unwrap().len(), after);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "builtins")]
#[tokio::test]
async fn memory_mixed_wait_resolution_is_atomic() {
    mixed_wait_resolution_is_atomic(MemoryHosting::default()).await;
}

#[cfg(all(feature = "builtins", feature = "local"))]
#[tokio::test]
async fn sqlite_mixed_wait_resolution_is_atomic() {
    let dir = tempfile::tempdir().unwrap();
    mixed_wait_resolution_is_atomic(
        everruns::hosting::SqliteHosting::open(dir.path().join("authority.db")).unwrap(),
    )
    .await;
}

#[cfg(all(feature = "builtins", feature = "local"))]
#[tokio::test]
async fn native_approval_resolves_after_frontend_loss_without_duplicating_effects() {
    let directory = tempfile::tempdir().unwrap();
    approval_survives_frontend_loss(
        everruns::hosting::SqliteHosting::open(directory.path().join("authority.db")).unwrap(),
    )
    .await;
}

#[cfg(feature = "local")]
#[tokio::test]
async fn retained_environment_reopens_attachments_and_rejects_unconfigured_workspace_before_creation()
 {
    let directory = tempfile::tempdir().unwrap();
    let authority =
        everruns::hosting::SqliteHosting::open(directory.path().join("authority.db")).unwrap();
    let configured = Agent::builder()
        .model(Model::simulated("Hello!"))
        .instructions("Read the attached report.")
        .workspace(directory.path().join("workspace"))
        .build()
        .unwrap();
    let unsupported = engine(authority.clone(), configured.clone());
    let rejected = SessionId::new();
    assert!(
        unsupported
            .create_registered(rejected, &build())
            .await
            .is_err()
    );
    assert!(unsupported.session_binding(rejected).await.is_err());
    let first = Engine::builder().hosting(authority.clone()).build();
    first
        .register_agent_with_environment(
            build(),
            configured.clone(),
            None,
            "workspace-v1",
            configured.environment_factory(),
        )
        .unwrap();
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    let part = session
        .attach(
            "report.txt",
            Some("text/plain"),
            b"retained report".to_vec(),
        )
        .await
        .unwrap();
    session.start().await.unwrap();
    let head = session.workspace_head().unwrap().binding().clone();
    assert_eq!(
        first
            .session_binding(id)
            .await
            .unwrap()
            .environment
            .as_deref(),
        Some("workspace-v1")
    );
    drop(session);
    drop(first);
    let rebuilt = Agent::builder()
        .model(Model::simulated("Hello!"))
        .instructions("Read the attached report.")
        .workspace(directory.path().join("workspace"))
        .build()
        .unwrap();
    let replacement = Engine::builder().hosting(authority.clone()).build();
    replacement
        .register_agent_with_environment(
            build(),
            rebuilt.clone(),
            None,
            "workspace-v1",
            rebuilt.environment_factory(),
        )
        .unwrap();
    let session = replacement.resume(id).await.unwrap();
    session.start().await.unwrap();
    assert_eq!(session.workspace_head().unwrap().binding(), &head);
    let everruns::ContentPart::File(file) = part else {
        panic!("expected a file reference")
    };
    let path = everruns_contracts::runtime::attachments::attachment_path(
        &file.file_id.to_string(),
        file.filename.as_deref(),
        "file",
    );
    let reopened = session.workspace_head().unwrap();
    let files = everruns_contracts::runtime::mount_fs::scoped_prompt_file_store(
        reopened.file_system(),
        reopened.workspace_id(),
    );
    let stored = files.read_file(id, &path).await.unwrap().unwrap();
    assert_eq!(stored.content.as_deref(), Some("retained report"));
    assert!(stored.is_readonly);
    // A changed reference cannot silently widen permissions or run newer
    // environment configuration, while canonical history remains readable.
    let changed = Engine::builder().hosting(authority).build();
    changed
        .register_agent_with_environment(
            build(),
            rebuilt.clone(),
            None,
            "workspace-v2",
            rebuilt.environment_factory(),
        )
        .unwrap();
    let reader = changed.resume(id).await.unwrap();
    assert!(reader.events_after(0).await.unwrap().is_empty());
    assert!(reader.start().await.is_err());
}

#[cfg(feature = "local")]
#[tokio::test]
async fn native_startup_discovers_a_parked_approval_deadline_without_client_wakeup() {
    use everruns_core::events::{
        EventContext, EventRequest, ToolCallRequestedData, TurnStartedData,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.db");
    let id = SessionId::new();
    let message = MessageId::new();
    let turn = everruns::TurnId::new();
    {
        let profile = everruns::hosting::SqliteHosting::open(&path).unwrap();
        let first = engine(External(profile.clone()), agent());
        let session = first.create_registered(id, &build()).await.unwrap();
        session
            .send_with_id(message, "wait for approval")
            .await
            .unwrap();
        // Simulate the canonical boundary committed by an owner before loss.
        let activation = profile
            .claim(id, "previous-owner", std::time::Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        let epoch = activation.epoch;
        let log = epoch.backends().event_log;
        let context = EventContext::turn(turn, message);
        log.append(EventRequest::new(
            id,
            context.clone(),
            serde_json::from_value::<TurnStartedData>(
                serde_json::json!({"turn_id":turn,"input_message_id":message}),
            )
            .unwrap(),
        ))
        .await
        .unwrap();
        let now = everruns_contracts::rt::utc_now();
        // The deadline must still be in the future at the pre-restart `runnable` assertion:
        // any authority transaction at or past `wake_at` marks the session runnable. A
        // 150ms margin was exhausted by setup on contended CI runners (EVE-1294).
        let deadline = now + chrono::Duration::seconds(3);
        let request = everruns::ToolCall {
            id: "tool_approval_original".into(),
            name: "approve_tool_call".into(),
            arguments: serde_json::json!({"code":"tool_approval_required","error":"waiting","tool_call_id":"original","tool":"mutate","arguments":{},"fingerprint":"test","risk":"policy","mode":"normal","asked_at":now.to_rfc3339(),"expires_at":deadline.to_rfc3339()}),
        };
        let paused = log
            .append(EventRequest::new(
                id,
                context,
                ToolCallRequestedData::with_definitions(&[request], &[]),
            ))
            .await
            .unwrap();
        epoch
            .commit(
                &format!("assignment/{message}"),
                serde_json::json!({"turn_id":turn,"steered":false}),
                true,
            )
            .await
            .unwrap();
        epoch
            .schedule(Some(deadline.timestamp_millis()))
            .await
            .unwrap();
        epoch
            .actor_store()
            .release_actor(
                id,
                "previous-owner",
                activation.actor.fence,
                i64::from(paused.sequence.unwrap()),
            )
            .await
            .unwrap();
        assert!(!profile.status(id).await.unwrap().runnable);
    }
    let profile = everruns::hosting::SqliteHosting::open(&path).unwrap();
    let replacement = engine(profile.clone(), agent());
    let stopped = everruns::CancellationToken::new();
    let stop = stopped.clone();
    let host = replacement.clone();
    let supervisor = tokio::spawn(async move { profile.supervise(host, stop).await });
    let session = replacement.resume(id).await.unwrap();
    let admission = session.admission(message).await.unwrap().unwrap();
    // Covers the 3s deadline plus supervised recovery on a slow runner.
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), admission.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(outcome.success);
    let events = session.events_after(0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "turn.started")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "tool.completed"
                && event.canonical_json()["data"]["status"] == "timeout"
                && event.canonical_json()["data"]["error"]
                    .as_str()
                    .is_some_and(|error| error.contains("expired")))
            .count(),
        1
    );
    assert!(session.pending_tools().await.unwrap().is_empty());
    stopped.cancel();
    supervisor.await.unwrap().unwrap();
}

/// One process of `native_process_loss_preserves_admission_and_finalization`
/// (frontend, owner or replacement), run by that test in a child copy of this
/// executable.
#[cfg(feature = "local")]
async fn native_process_role() {
    let directory =
        std::path::PathBuf::from(std::env::var("EVERRUNS_PROCESS_FIXTURE_DIR").unwrap());
    let mode = std::env::var("EVERRUNS_PROCESS_FIXTURE_MODE").unwrap();
    let id: SessionId = std::env::var("EVERRUNS_PROCESS_FIXTURE_SESSION")
        .unwrap()
        .parse()
        .unwrap();
    let message: MessageId = std::env::var("EVERRUNS_PROCESS_FIXTURE_MESSAGE")
        .unwrap()
        .parse()
        .unwrap();
    let marker = directory.join("completion-effects");
    let definition = Agent::builder()
        .instructions("Answer briefly.")
        .model(Model::simulated("finished"))
        .on_completion(move |_| {
            let marker = marker.clone();
            async move {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(marker)
                    .unwrap();
                file.write_all(b"effect\n").unwrap();
                file.sync_all().unwrap();
                std::future::pending::<()>().await;
            }
        })
        .build()
        .unwrap();
    let profile = everruns::hosting::SqliteHosting::open(directory.join("authority.db")).unwrap();
    let host = engine(External(profile.clone()), definition);
    if mode == "frontend" {
        let session = host.create_registered(id, &build()).await.unwrap();
        session.send_with_id(message, "finish").await.unwrap();
        std::fs::write(directory.join("accepted"), b"accepted").unwrap();
        std::future::pending::<()>().await;
    } else if mode == "owner" {
        host.activate(id).await.unwrap();
        panic!(
            "owner returned before blocked completion: {:?}",
            profile.status(id).await.unwrap()
        );
    } else {
        profile
            .supervise(host, everruns::CancellationToken::new())
            .await
            .unwrap();
    }
}

#[cfg(feature = "local")]
#[tokio::test]
async fn native_process_loss_preserves_admission_and_finalization() {
    // The test is its own child: run with this variable, it plays one of the
    // processes instead. It is the parent-to-child channel, not a switch that
    // skips anything; the parent always runs.
    if std::env::var_os("EVERRUNS_PROCESS_FIXTURE_MODE").is_some() {
        return native_process_role().await;
    }
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let id = SessionId::new();
    let message = MessageId::new();
    let spawn = |mode: &str| {
        let log = std::fs::File::create(directory.path().join(format!("{mode}.log"))).unwrap();
        Child(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "controls::native_process_loss_preserves_admission_and_finalization",
                    "--nocapture",
                ])
                .env("EVERRUNS_PROCESS_FIXTURE_DIR", directory.path())
                .env("EVERRUNS_PROCESS_FIXTURE_MODE", mode)
                .env("EVERRUNS_PROCESS_FIXTURE_SESSION", id.to_string())
                .env("EVERRUNS_PROCESS_FIXTURE_MESSAGE", message.to_string())
                .stdout(std::process::Stdio::from(log.try_clone().unwrap()))
                .stderr(std::process::Stdio::from(log))
                .spawn()
                .unwrap(),
        )
    };
    let wait_for = |name: &'static str| {
        let path = directory.path().join(name);
        async move {
            let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while !path.exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await;
            if result.is_err() {
                let parent = path.parent().unwrap();
                let diagnostics = ["frontend", "owner", "replacement"]
                    .into_iter()
                    .map(|mode| {
                        format!(
                            "{mode}: {}",
                            std::fs::read_to_string(parent.join(format!("{mode}.log")))
                                .unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                panic!("missing {name}: {diagnostics}");
            }
        }
    };
    let frontend = spawn("frontend");
    wait_for("accepted").await;
    drop(frontend);
    let owner = spawn("owner");
    wait_for("completion-effects").await;
    drop(owner);
    let replacement = spawn("replacement");
    // No client calls at all while the independent startup supervisor discovers
    // the expired lease (default 10s TTL) and finalizes the outcome after a real
    // process kill. Poll read-only state instead of guessing a fixed sleep: a 12s
    // sleep left ~2s for process startup and supervision on a contended runner.
    let profile =
        everruns::hosting::SqliteHosting::open(directory.path().join("authority.db")).unwrap();
    let reader = Engine::builder()
        .hosting(profile)
        .build()
        .resume(id)
        .await
        .unwrap();
    let admission = reader.admission(message).await.unwrap().unwrap();
    let state = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let state = admission.state().await.unwrap();
            if matches!(state, AdmissionState::Complete { .. }) {
                break state;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("replacement process did not finalize independently");
    let AdmissionState::Complete { turn: outcome } = state else {
        unreachable!()
    };
    assert_eq!(outcome.response, "finished");
    assert!(outcome.success);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("completion-effects")).unwrap(),
        "effect\n"
    );
    let events = reader.events_after(0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "input.message")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "reason.started")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "turn.completed")
            .count(),
        1
    );
    drop(replacement);
}

#[tokio::test]
async fn canonical_filesystem_and_shell_capabilities_require_retained_environments_before_creation()
{
    for capability in ["session_file_system", "bashkit_shell", "virtual_bash"] {
        let owner = engine(
            MemoryHosting::default(),
            Agent::builder()
                .instructions("Use the configured workspace.")
                .model(Model::simulated("done"))
                .capability(capability)
                .build()
                .unwrap(),
        );
        let id = SessionId::new();
        let error = match owner.create_registered(id, &build()).await {
            Ok(_) => panic!("{capability} must not silently select an ephemeral workspace"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("retained Environment"),
            "{error}"
        );
        assert!(owner.session_binding(id).await.is_err());
    }
}
