use super::*;
use everruns_core::{
    events::{ToolCallRequestedData, ToolCompletedData, TurnStartedData},
    session_commands::{ClientDispatchClaimReply, ClientDispatchRecord, SessionCommand},
};
#[tokio::test]
async fn client_dispatch_sqlite_concurrent_claim_reopen_and_atomic_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dispatch.db");
    let hosting = SqliteHosting::open(&path).unwrap();
    let session = SessionId::new();
    let turn = TurnId::new();
    let input = MessageId::new();
    hosting
        .create(
            session,
            &SessionBinding {
                framework: 1,
                behavior: AgentBuild::new("dispatch", "v1"),
                harness: None,
                environment: None,
                metadata: Value::Null,
            },
        )
        .await
        .unwrap();
    let call = crate::ToolCall {
        id: "external".into(),
        name: "browser_handoff".into(),
        arguments: json!({"tab":42}),
    };
    hosting
        .transaction(session, |tx, state| {
            let context = EventContext::turn(turn, input);
            append(
                tx,
                state,
                EventRequest::new(
                    session,
                    context.clone(),
                    TurnStartedData {
                        turn_id: turn,
                        input_message_id: input,
                        input_content: None,
                        agent_id: None,
                        agent_name: None,
                        agent_description: None,
                        origin: None,
                    },
                ),
                None,
            )?;
            append(
                tx,
                state,
                EventRequest::new(
                    session,
                    context,
                    ToolCallRequestedData::with_definitions(std::slice::from_ref(&call), &[]),
                ),
                None,
            )?;
            Ok(())
        })
        .unwrap();
    let begin = SessionCommand::BeginClientDispatch {
        id: uuid::Uuid::new_v4().to_string(),
        turn,
        call,
        owner: uuid::Uuid::new_v4(),
        resolution_id: uuid::Uuid::new_v4(),
        deadline_ms: everruns_contracts::rt::utc_now().timestamp_millis() + 60_000,
    };
    let mut competing = begin.clone();
    if let SessionCommand::BeginClientDispatch { id, .. } = &mut competing {
        *id = uuid::Uuid::new_v4().to_string();
    }
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let jobs: Vec<_> = [begin.clone(), competing]
        .into_iter()
        .map(|command| {
            let db = SqliteHosting::open(&path).unwrap();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                db.execute_command(session, command)
            })
        })
        .collect();
    let replies: Vec<_> = jobs.into_iter().map(|job| job.join().unwrap()).collect();
    assert_eq!(replies.iter().filter(|r| r.is_ok()).count(), 1);
    let claim: ClientDispatchClaimReply =
        serde_json::from_value(replies.into_iter().find_map(Result::ok).unwrap()).unwrap();
    drop(hosting);
    let hosting = SqliteHosting::open(&path).unwrap();
    let record = claim.record;
    let inspect = SessionCommand::InspectClientDispatch {
        turn,
        call_id: record.call.id.clone(),
    };
    let retained: Option<ClientDispatchRecord> =
        serde_json::from_value(hosting.execute_command(session, inspect.clone()).unwrap()).unwrap();
    assert_eq!(retained.unwrap().fence, record.fence);
    let complete = SessionCommand::CompleteClientDispatch {
        id: record.resolution_id.to_string(),
        turn,
        call_id: record.call.id.clone(),
        owner: record.owner,
        fence: record.fence,
        result: ToolCompletedData::success(record.call.id, record.call.name, vec![], None),
    };
    let reply = hosting.execute_command(session, complete.clone()).unwrap();
    drop(hosting);
    let hosting = SqliteHosting::open(&path).unwrap();
    assert_eq!(hosting.execute_command(session, complete).unwrap(), reply);
    let retained: Option<ClientDispatchRecord> =
        serde_json::from_value(hosting.execute_command(session, inspect).unwrap()).unwrap();
    assert!(retained.unwrap().resolution_applied);
    let db = hosting.db.lock().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM framework_events WHERE session=? AND type='tool.completed'",
            [session.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}
