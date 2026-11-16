use super::*;
use everruns::hosting::{
    AgentBuild,
    spi::session_commands::{ClientDispatchClaimReply, ClientDispatchRecord},
};
use everruns_contracts::typed_id::SessionId;
#[test]
fn client_dispatch_cell_snapshot_retains_owner_and_atomic_resolution() {
    let session = SessionId::new();
    let turn = TurnId::new();
    let mut state = crate::state::SessionState::default();
    state.authority.binding = Some(SessionBinding {
        framework: 1,
        behavior: AgentBuild::new("dispatch", "v1"),
        harness: None,
        environment: None,
        metadata: Value::Null,
    });
    state.authority.open = Some(turn);
    state.authority.open_input = Some(MessageId::new());
    let call = ToolCall {
        id: "external".into(),
        name: "browser_handoff".into(),
        arguments: json!({"tab":42}),
    };
    state.authority.pending = vec![call.clone()];
    let begin = SessionCommand::BeginClientDispatch {
        id: uuid::Uuid::new_v4().to_string(),
        turn,
        call,
        owner: uuid::Uuid::new_v4(),
        resolution_id: uuid::Uuid::new_v4(),
        deadline_ms: 2000,
    };
    let (reply, events) = state
        .command(session, begin.clone(), 1000, EventId::new())
        .unwrap();
    assert!(events.is_empty());
    let claim: ClientDispatchClaimReply = serde_json::from_value(reply).unwrap();
    let record = claim.record;
    let mut state: crate::state::SessionState =
        serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
    let (reply, events) = state.command(session, begin, 1001, EventId::new()).unwrap();
    assert_eq!(reply["fresh"], false);
    assert!(events.is_empty());
    let command = SessionCommand::CompleteClientDispatch {
        id: record.resolution_id.to_string(),
        turn,
        call_id: record.call.id.clone(),
        owner: record.owner,
        fence: record.fence,
        result: everruns_core::events::ToolCompletedData::success(
            record.call.id.clone(),
            record.call.name.clone(),
            vec![],
            None,
        ),
    };
    let (reply, events) = state
        .command(session, command.clone(), 1002, EventId::new())
        .unwrap();
    assert_eq!(events.len(), 1);
    let (replayed, events) = state
        .command(session, command, 3000, EventId::new())
        .unwrap();
    assert_eq!(replayed, reply);
    assert!(events.is_empty());
    let (retained, _) = state
        .command(
            session,
            SessionCommand::InspectClientDispatch {
                turn,
                call_id: record.call.id,
            },
            3000,
            EventId::new(),
        )
        .unwrap();
    let retained: Option<ClientDispatchRecord> = serde_json::from_value(retained).unwrap();
    assert!(retained.unwrap().resolution_applied);
    assert!(state.authority.pending.is_empty());
}
