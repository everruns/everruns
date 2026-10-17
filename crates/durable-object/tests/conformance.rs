//! Engine control conformance against actual workerd storage, without a local authority fallback.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use everruns::hosting::{AgentBuild, SessionHosting};
use everruns::{Agent, Engine, Model, SessionId};
use everruns_durable_object::{AuthorityClient, ObjectHosting};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
#[ignore = "requires the local workerd authority on port 8789"]
async fn object_waits_reopen_and_mixed_resolution_is_atomic() {
    let profile = ObjectHosting::new(
        AuthorityClient::new(
            "http://127.0.0.1:8789",
            "test",
            "authority-test-token-at-least-32-bytes",
        )
        .unwrap(),
    );
    let build = AgentBuild::new("conformance", uuid::Uuid::new_v4().to_string());
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
    let engine = |agent: Agent| {
        Engine::builder()
            .hosting(profile.clone())
            .register(build.clone(), agent)
            .build()
    };
    let owner = engine(agent.clone());
    let id = SessionId::new();
    let session = owner.create_registered(id, &build).await.unwrap();
    let message = everruns::MessageId::new();
    session.send_with_id(message, "work").await.unwrap();
    owner.activate(id).await.unwrap();
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
    drop(session);
    drop(owner);
    let replacement = engine(agent);
    let session = replacement.resume(id).await.unwrap();
    assert!(
        matches!(session.admission(message).await.unwrap().unwrap().state().await.unwrap(),
        everruns::hosting::AdmissionState::Waiting { calls, .. } if calls.len() == 2)
    );
    assert!(session.interrupted_turn().await.unwrap().is_none());
    assert!(session.resume_interrupted_turn().await.unwrap().is_none());
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
    assert!(
        session
            .resolve_waits(
                uuid::Uuid::new_v4(),
                everruns::TurnId::new(),
                decisions.clone(),
                vec![result.clone()]
            )
            .await
            .is_err()
    );
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
    replacement.activate(id).await.unwrap();
    let sent = session.admission(message).await.unwrap().unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(3), sent.wait())
            .await
            .unwrap()
            .unwrap()
            .success
    );
    let canonical = session.events_after(0).await.unwrap();
    let first = &canonical[0];
    assert_eq!(
        session.event_sequence(&first.event_id).await.unwrap(),
        first.sequence()
    );
    let after = canonical.len();
    session
        .resolve_waits(command, turn, decisions, vec![result])
        .await
        .unwrap();
    assert_eq!(session.events_after(0).await.unwrap().len(), after);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}
