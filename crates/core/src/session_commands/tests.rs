use super::*;

fn view() -> CommandView {
    let mut view = CommandView::new(SessionId::new(), 1_000_000);
    view.binding = Some(json!({"agent":"assistant","build_id":"v1"}));
    view.open = Some(EventContext::turn(TurnId::new(), MessageId::new()));
    view
}

fn approval(id: &str) -> ToolCall {
    ToolCall {
        id: format!("tool_approval_{id}"),
        name: "approve_tool_call".into(),
        arguments: json!({"code":"tool_approval_required","error":"waiting", "tool_call_id":id,
            "tool":"mutate","arguments":{"n":1},"fingerprint":format!("sha256:{id}"),
            "risk":"policy","mode":"normal","asked_at":"1970-01-01T00:16:40Z",
            "expires_at":"1970-01-01T00:18:20Z"}),
    }
}
fn result(id: &str) -> ToolCompletedData {
    ToolCompletedData::success(id.into(), String::new(), vec![], None)
}
fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "render_client".into(),
        arguments: json!({}),
    }
}

#[test]
fn empty_command_identity_cannot_be_hidden_by_a_namespace() {
    let result = prepare(
        SessionCommand::Cancel {
            id: String::new(),
            input: MessageId::new(),
        },
        view(),
    );
    assert!(result.err().unwrap().to_string().contains("identity"));
}

#[test]
fn idempotent_binding_still_opens_at_command_capacity() {
    let mut view = view();
    view.command_count = 100_000;
    let command = SessionCommand::Create {
        binding: view.binding.clone().unwrap(),
    };
    assert!(prepare(command, view).is_ok());
}

#[test]
fn accepted_approval_replays_after_its_deadline_and_without_a_pending_call() {
    let mut view = view();
    view.pending = vec![approval("a")];
    let command = SessionCommand::Approve {
        id: "answer".into(),
        approvals: vec![ApprovalCommit {
            turn_id: view.open.as_ref().unwrap().turn_id.unwrap(),
            request_id: view.pending[0].id.clone(),
            decision: ToolApprovalDecision::Allow,
        }],
    };
    let plan = prepare(command.clone(), view.clone()).unwrap();
    assert_eq!(plan.objects.len(), 1);
    view.now_ms = 2_000_000;
    view.pending.clear();
    view.open = None;
    view.previous = Some(CommandReceipt {
        fingerprint: plan.fingerprint,
        value: plan.reply.clone(),
    });
    let retry = prepare(command, view).unwrap();
    assert!(retry.replay);
    assert_eq!(retry.reply, plan.reply);
    assert!(retry.events.is_empty());
    assert!(retry.objects.is_empty());
    assert!(matches!(retry.control, ControlUpdate::None));
}

#[test]
fn omitted_approval_has_no_persisted_permission_and_speaks_the_shared_outcome() {
    use everruns_contracts::execution_api::ToolApprovalAnswer;
    let mut view = view();
    view.pending = vec![approval("a"), approval("b")];
    let turn = view.open.as_ref().unwrap().turn_id.unwrap();
    let plan = prepare(
        SessionCommand::SubmitApprovals {
            id: "answer".into(),
            turn,
            decisions: vec![ToolApprovalAnswer {
                tool_call_id: view.pending[0].id.clone(),
                decision: ToolApprovalDecision::Allow,
            }],
        },
        view,
    )
    .unwrap();
    assert_eq!(
        plan.objects.len(),
        1,
        "silence must not manufacture a permission record"
    );
    assert_eq!(plan.reply["resolved"][1]["outcome"], "not_approved");
    let event = serde_json::to_value(&plan.events[2]).unwrap();
    assert!(
        event["data"]["result"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not_approved")
    );
    let spoken = serde_json::to_value(&plan.events[3]).unwrap();
    assert!(
        spoken["data"]["message"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("didn't approve")
    );
    assert!(matches!(plan.control, ControlUpdate::Resume));
}

#[test]
fn client_results_use_canonical_names_and_partial_batches_do_not_resume() {
    let mut view = view();
    view.pending = vec![call("a"), call("b")];
    let turn = view.open.as_ref().unwrap().turn_id.unwrap();
    let command = SessionCommand::Resolve {
        id: "first".into(),
        turn,
        results: vec![result("a")],
    };
    let plan = prepare(command, view.clone()).unwrap();
    assert!(matches!(plan.control, ControlUpdate::None));
    assert_eq!(
        serde_json::to_value(&plan.events[0]).unwrap()["data"]["tool_name"],
        "render_client"
    );
    view.pending.remove(0);
    let plan = prepare(
        SessionCommand::Resolve {
            id: "second".into(),
            turn,
            results: vec![result("b")],
        },
        view,
    )
    .unwrap();
    assert!(matches!(plan.control, ControlUpdate::Resume));
}

#[test]
fn a_malformed_member_rejects_the_entire_mixed_batch() {
    let mut view = view();
    view.pending = vec![approval("a"), call("b")];
    let turn = view.open.as_ref().unwrap().turn_id.unwrap();
    let result = prepare(
        SessionCommand::ResolveWaits {
            id: "batch".into(),
            turn,
            approvals: vec![ApprovalCommit {
                turn_id: turn,
                request_id: view.pending[0].id.clone(),
                decision: ToolApprovalDecision::AllowAlways,
            }],
            results: vec![result("not-pending")],
        },
        view,
    );
    assert!(result.err().unwrap().to_string().contains("pending call"));
}

#[test]
fn sequence_exhaustion_rejects_a_seed_before_the_first_append() {
    let mut view = view();
    view.head = i32::MAX - 1;
    let result = prepare(
        SessionCommand::Seed {
            id: "seed".into(),
            messages: vec![RuntimeMessage::user("one"), RuntimeMessage::user("two")],
        },
        view,
    );
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("sequence capacity")
    );
}

#[test]
fn a_command_cannot_reuse_another_payloads_receipt() {
    let mut view = view();
    view.previous = Some(CommandReceipt {
        fingerprint: json!({"input":MessageId::new()}),
        value: json!({}),
    });
    assert!(
        prepare(
            SessionCommand::Cancel {
                id: "cancel".into(),
                input: MessageId::new()
            },
            view
        )
        .err()
        .unwrap()
        .to_string()
        .contains("different input")
    );
}

#[test]
fn restored_build_retry_is_fenced_binding_preserving_and_idempotent() {
    let mut facts = view();
    facts.execution_active = false;
    facts.suspended = true;
    facts.suspension_reason = Some("retained_build_unavailable".into());
    let command = SessionCommand::RetryExecution {
        id: "restore-v1".into(),
        binding: facts.binding.clone().unwrap(),
    };
    let plan = prepare(command.clone(), facts.clone()).unwrap();
    assert!(matches!(plan.control, ControlUpdate::RetryExecution));
    assert!(plan.events.is_empty() && plan.objects.is_empty());
    facts.execution_active = true;
    assert!(prepare(command.clone(), facts.clone()).is_err());
    facts.execution_active = false;
    facts.binding.as_mut().unwrap()["build_id"] = json!("changed");
    assert!(prepare(command.clone(), facts.clone()).is_err());
    facts.binding = Some(command.fingerprint());
    facts.suspension_reason = Some("uncertain_agent_start".into());
    assert!(prepare(command.clone(), facts.clone()).is_err());
    // A lost reply must replay even after execution restarts or a different
    // suspension appears; it must not clear that later suspension.
    facts.execution_active = true;
    facts.previous = Some(CommandReceipt {
        fingerprint: plan.fingerprint,
        value: plan.reply.clone(),
    });
    let replay = prepare(command, facts).unwrap();
    assert!(replay.replay && matches!(replay.control, ControlUpdate::None));
    assert_eq!(replay.reply, plan.reply);
}
