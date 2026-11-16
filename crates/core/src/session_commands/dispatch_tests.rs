use super::*;
fn setup() -> (CommandView, SessionCommand) {
    let mut view = CommandView::new(SessionId::new(), 1000);
    view.binding = Some(json!({"agent":"test","build_id":"1"}));
    view.dispatch_supported = true;
    let turn = TurnId::new();
    view.open = Some(EventContext::turn(turn, MessageId::new()));
    let call = ToolCall {
        id: "browser-call".into(),
        name: "browser_handoff".into(),
        arguments: json!({"tab":42,"message":"review"}),
    };
    view.pending = vec![call.clone()];
    let begin = SessionCommand::BeginClientDispatch {
        id: uuid::Uuid::new_v4().to_string(),
        turn,
        call,
        owner: uuid::Uuid::new_v4(),
        resolution_id: uuid::Uuid::new_v4(),
        deadline_ms: 2000,
    };
    (view, begin)
}
fn claim(view: &mut CommandView, command: SessionCommand) -> ClientDispatchRecord {
    let plan = prepare(command, view.clone()).unwrap();
    let reply: ClientDispatchClaimReply = serde_json::from_value(plan.reply).unwrap();
    assert!(reply.fresh);
    assert_eq!(plan.objects.len(), 2);
    assert!(plan.events.is_empty());
    view.dispatches.push(reply.record.clone());
    reply.record
}
fn complete(record: &ClientDispatchRecord) -> SessionCommand {
    SessionCommand::CompleteClientDispatch {
        id: record.resolution_id.to_string(),
        turn: record.turn,
        call_id: record.call.id.clone(),
        owner: record.owner,
        fence: record.fence,
        result: ToolCompletedData::success(
            record.call.id.clone(),
            record.call.name.clone(),
            vec![],
            None,
        )
        .into(),
    }
}
fn interrupt(record: &ClientDispatchRecord, owner: uuid::Uuid) -> SessionCommand {
    SessionCommand::InterruptClientDispatch {
        id: record.resolution_id.to_string(),
        turn: record.turn,
        call_id: record.call.id.clone(),
        owner,
        fence: record.fence,
    }
}
#[test]
fn dispatch_lost_claim_reply_never_reauthorizes_execution() {
    let (mut view, begin) = setup();
    let first = prepare(begin.clone(), view.clone()).unwrap();
    view.previous = Some(CommandReceipt {
        fingerprint: first.fingerprint,
        value: first.reply.clone(),
    });
    view.now_ms = 3000;
    view.pending.clear();
    view.open = None;
    let replay = prepare(begin, view).unwrap();
    assert!(replay.replay);
    assert_eq!(replay.reply["fresh"], false);
    assert_eq!(replay.reply["record"], first.reply["record"]);
    assert!(replay.objects.is_empty());
    assert!(replay.events.is_empty());
}
#[test]
fn dispatch_one_claim_exact_identity_and_no_resolution_bypass() {
    let (mut view, begin) = setup();
    let record = claim(&mut view, begin.clone());
    let mut competitor = begin;
    if let SessionCommand::BeginClientDispatch { id, owner, .. } = &mut competitor {
        *id = uuid::Uuid::new_v4().to_string();
        *owner = uuid::Uuid::new_v4();
    }
    assert!(prepare(competitor, view.clone()).is_err());
    let result = ToolCompletedData::success(
        record.call.id.clone(),
        record.call.name.clone(),
        vec![],
        None,
    );
    assert!(
        prepare(
            SessionCommand::Resolve {
                id: "bypass".into(),
                turn: record.turn,
                results: vec![result]
            },
            view.clone()
        )
        .is_err()
    );
    for alteration in 0..5 {
        let mut cmd = complete(&record);
        if let SessionCommand::CompleteClientDispatch {
            turn,
            call_id,
            owner,
            fence,
            result,
            ..
        } = &mut cmd
        {
            match alteration {
                0 => *turn = TurnId::new(),
                1 => *call_id = "wrong".into(),
                2 => *owner = uuid::Uuid::new_v4(),
                3 => *fence = uuid::Uuid::new_v4(),
                _ => result.tool_name = "wrong".into(),
            }
        }
        assert!(prepare(cmd, view.clone()).is_err());
    }
    let mut other = view.clone();
    other.session = SessionId::new();
    assert!(prepare(complete(&record), other).is_err());
    view.pending[0].arguments = json!({"tab":99});
    assert!(prepare(complete(&record), view).is_err());
}
#[test]
fn dispatch_completion_is_atomic_and_lost_response_idempotent() {
    let (mut view, begin) = setup();
    let record = claim(&mut view, begin);
    let cmd = complete(&record);
    let plan = prepare(cmd.clone(), view.clone()).unwrap();
    assert_eq!(plan.events.len(), 1);
    assert_eq!(plan.objects.len(), 1);
    assert!(matches!(plan.control, ControlUpdate::Resume));
    assert_eq!(plan.reply["state"], "completed");
    view.previous = Some(CommandReceipt {
        fingerprint: plan.fingerprint,
        value: plan.reply.clone(),
    });
    view.pending.clear();
    view.open = None;
    view.now_ms = 9999;
    let replay = prepare(cmd, view.clone()).unwrap();
    assert!(replay.replay);
    assert_eq!(replay.reply, plan.reply);
    assert!(replay.events.is_empty());
    assert!(prepare(interrupt(&record, record.owner), view).is_err());
}
#[test]
fn dispatch_crash_expiry_fences_success_and_never_reclaims() {
    let (mut view, begin) = setup();
    let record = claim(&mut view, begin.clone());
    let peer = uuid::Uuid::new_v4();
    assert!(prepare(interrupt(&record, peer), view.clone()).is_err());
    view.now_ms = record.deadline_ms;
    assert!(prepare(complete(&record), view.clone()).is_err());
    let plan = prepare(interrupt(&record, peer), view.clone()).unwrap();
    assert_eq!(plan.reply["state"], "interrupted");
    assert_eq!(plan.reply["result"]["success"], false);
    view.dispatches = vec![serde_json::from_value(plan.reply).unwrap()];
    assert!(prepare(begin, view).is_err());
}
#[test]
fn dispatch_requires_authority_support_exact_call_and_bounded_deadline() {
    let (view, begin) = setup();
    for variation in 0..5 {
        let mut view = view.clone();
        let mut cmd = begin.clone();
        if let SessionCommand::BeginClientDispatch {
            deadline_ms,
            call,
            owner,
            ..
        } = &mut cmd
        {
            match variation {
                0 => view.dispatch_supported = false,
                1 => *deadline_ms = i64::MAX,
                2 => *deadline_ms = view.now_ms,
                3 => call.arguments = json!({"tab":0}),
                _ => *owner = uuid::Uuid::nil(),
            }
        }
        assert!(prepare(cmd, view).is_err());
    }
}

#[test]
fn dispatch_interruption_retires_cancelled_call_without_reviving_turn() {
    let (mut view, begin) = setup();
    let record = claim(&mut view, begin);
    view.open = Some(EventContext::turn(TurnId::new(), MessageId::new()));
    view.pending = vec![ToolCall {
        id: "different".into(),
        name: "other".into(),
        arguments: json!({}),
    }];
    assert!(prepare(complete(&record), view.clone()).is_err());
    let plan = prepare(interrupt(&record, record.owner), view).unwrap();
    assert_eq!(plan.reply["state"], "interrupted");
    assert_eq!(plan.reply["resolution_applied"], false);
    assert!(plan.events.is_empty());
    assert!(matches!(plan.control, ControlUpdate::None));
    assert_eq!(plan.objects.len(), 1);
}

#[test]
fn dispatch_resolution_identity_cannot_be_shared_by_different_calls() {
    let (mut view, begin) = setup();
    let record = claim(&mut view, begin.clone());
    let mut second = begin;
    if let SessionCommand::BeginClientDispatch { id, call, .. } = &mut second {
        *id = uuid::Uuid::new_v4().to_string();
        call.id = "other-call".into();
        view.pending.push(call.clone());
    }
    assert!(prepare(second, view).is_err());
    assert!(!record.resolution_id.is_nil());
}
