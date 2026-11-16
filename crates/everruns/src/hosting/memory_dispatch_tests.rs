use super::*;
use everruns_core::session_commands::{ClientDispatchClaimReply, ClientDispatchRecord};
#[tokio::test]
async fn client_dispatch_memory_claim_is_exclusive_and_completion_atomic() {
    let profile = MemoryHosting::default();
    let id = SessionId::new();
    let turn = TurnId::new();
    let binding = SessionBinding {
        framework: 1,
        behavior: AgentBuild::new("dispatch", "v1"),
        harness: None,
        environment: None,
        metadata: json!(null),
    };
    profile.create(id, &binding).await.unwrap();
    let call = everruns_contracts::tool_types::ToolCall {
        id: "external".into(),
        name: "browser_handoff".into(),
        arguments: json!({"tab":1}),
    };
    {
        let session = profile.session(id);
        let mut state = session.control.lock().await;
        state.open = Some(turn);
        state.open_input = Some(MessageId::new());
        state.pending = vec![call.clone()];
    }
    let owner = uuid::Uuid::new_v4();
    let resolution_id = uuid::Uuid::new_v4();
    let begin = SessionCommand::BeginClientDispatch {
        id: uuid::Uuid::new_v4().to_string(),
        turn,
        call,
        owner,
        resolution_id,
        deadline_ms: everruns_contracts::rt::utc_now().timestamp_millis() + 60_000,
    };
    let mut competitor = begin.clone();
    if let SessionCommand::BeginClientDispatch { id, .. } = &mut competitor {
        *id = uuid::Uuid::new_v4().to_string();
    }
    let (first, second) = tokio::join!(
        profile.execute_command(id, begin.clone()),
        profile.execute_command(id, competitor.clone())
    );
    assert_ne!(first.is_ok(), second.is_ok());
    let (value, winning_command) = match first {
        Ok(value) => (value, begin),
        Err(_) => (second.unwrap(), competitor),
    };
    let reply: ClientDispatchClaimReply = serde_json::from_value(value).unwrap();
    assert!(reply.fresh);
    let replay: ClientDispatchClaimReply =
        serde_json::from_value(profile.execute_command(id, winning_command).await.unwrap())
            .unwrap();
    assert!(!replay.fresh);
    let record = reply.record;
    let cmd = SessionCommand::CompleteClientDispatch {
        id: resolution_id.to_string(),
        turn,
        call_id: record.call.id.clone(),
        owner,
        fence: record.fence,
        result: everruns_core::events::ToolCompletedData::success(
            record.call.id.clone(),
            record.call.name.clone(),
            vec![],
            None,
        ),
    };
    let result = profile.execute_command(id, cmd.clone()).await.unwrap();
    assert_eq!(profile.execute_command(id, cmd).await.unwrap(), result);
    let retained: Option<ClientDispatchRecord> = serde_json::from_value(
        profile
            .execute_command(
                id,
                SessionCommand::InspectClientDispatch {
                    turn,
                    call_id: record.call.id,
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(retained.unwrap().resolution_applied);
    assert!(profile.session(id).control.lock().await.pending.is_empty());
}

#[tokio::test]
async fn client_dispatch_memory_blocked_or_failed_append_never_partially_commits() {
    let profile = MemoryHosting::default();
    let id = SessionId::new();
    let session = profile.session(id);
    let mut state = session.control.lock().await;
    let call = everruns_contracts::tool_types::ToolCall {
        id: "pending".into(),
        name: "browser".into(),
        arguments: json!({}),
    };
    state.pending = vec![call.clone()];
    let plan = || session_commands::CommandPlan {
        key: Some("client_dispatch_resolution:test".into()),
        fingerprint: json!({}),
        reply: json!({"state":"completed"}),
        events: vec![],
        objects: vec![session_commands::ObjectWrite {
            kind: "framework",
            key: "client_dispatch/test".into(),
            value: json!({"state":"completed"}),
        }],
        control: ControlUpdate::Resume,
        replay: false,
    };
    let (send, receive) = tokio::sync::oneshot::channel::<()>();
    let mut commit = Box::pin(commit_dispatch(&mut state, &session, 0, plan(), async {
        let _ = receive.await;
        Ok(None)
    }));
    assert!(futures::poll!(commit.as_mut()).is_pending());
    drop(commit);
    drop(send);
    assert!(!state.records.contains_key("client_dispatch/test"));
    assert!(state.commands.is_empty());
    assert_eq!(state.pending, vec![call.clone()]);
    assert!(!state.runnable);
    assert_eq!(state.resume_generation, 0);
    assert!(
        commit_dispatch(&mut state, &session, 0, plan(), async {
            Err(AgentLoopError::store("append failed"))
        })
        .await
        .is_err()
    );
    assert!(!state.records.contains_key("client_dispatch/test"));
    assert!(state.commands.is_empty());
    assert_eq!(state.pending, vec![call]);
    assert!(!state.runnable);
}
