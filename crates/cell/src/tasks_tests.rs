//! A session's tasks in cell state: create once, wake the owner once.

use super::*;
use crate::state::CellState;
use everruns_core::session_task::{TaskInputRequest, TaskWakePolicy};

const T0: i64 = 1_800_000_000_000;

fn session() -> (CellState, SessionId) {
    use everruns::hosting::spi::session_commands::SessionCommand;
    let id = SessionId::new();
    let mut s = SessionState {
        tenant: Some("org".into()),
        ..Default::default()
    };
    let binding = everruns::hosting::SessionBinding {
        framework: 1,
        behavior: everruns::hosting::AgentBuild::new("agent", "build-1"),
        harness: None,
        environment: None,
        metadata: json!(null),
    };
    let create = SessionCommand::Create {
        binding: serde_json::to_value(binding).unwrap(),
    };
    s.command(id, create, T0, EventId::new()).unwrap();
    let mut state = CellState::default();
    state.sessions.insert(id, s);
    (state, id)
}

fn apply_op(state: &mut CellState, session: SessionId, op: TaskOp, now_ms: i64) -> Response {
    let mut changes = Vec::new();
    state.apply(
        Command::Platform {
            session,
            action: crate::platform::Op::Task(op),
            now_ms,
            seed: EventId::new(),
        },
        &mut changes,
    )
}

fn value(response: Response) -> Value {
    match response {
        Response::Value { value } => value,
        other => panic!("{other:?}"),
    }
}

fn task(session: SessionId, policy: TaskWakePolicy) -> SessionTask {
    new_session_task(
        CreateSessionTask {
            session_id: session,
            id: Some("task-1".into()),
            kind: "subagent".into(),
            display_name: "helper".into(),
            spec: json!({
                "push_configs": [{ "url": "https://example.test/hook", "secret": "s3" }]
            }),
            state: SessionTaskState::Running,
            links: Default::default(),
            wake_policy: policy,
        },
        at(T0),
    )
}

fn head(state: &CellState, session: SessionId) -> i32 {
    state.sessions[&session].last_sequence().unwrap_or(0)
}

#[test]
fn a_task_is_created_once_and_keeps_its_secrets() {
    let (mut state, session) = session();
    let new = task(session, TaskWakePolicy::Silent);
    let first = value(apply_op(
        &mut state,
        session,
        TaskOp::Create { task: stored(&new) },
        T0,
    ));
    assert_eq!(first["id"], "task-1");
    // A retry answers the task it made.
    let mut retry = new.clone();
    retry.display_name = "other".into();
    let again = value(apply_op(
        &mut state,
        session,
        TaskOp::Create {
            task: stored(&retry),
        },
        T0 + 1,
    ));
    assert_eq!(again["display_name"], "helper");
    let kept = &state.sessions[&session].platform.tasks["task-1"];
    assert_eq!(kept["spec"]["push_configs"][0]["secret"], "s3");

    // Another session's task is not this session's to create.
    let foreign = task(SessionId::new(), TaskWakePolicy::Silent);
    assert!(matches!(
        apply_op(
            &mut state,
            session,
            TaskOp::Create {
                task: stored(&foreign)
            },
            T0
        ),
        Response::Rejected { .. }
    ));
}

#[test]
fn a_finished_task_wakes_its_owner_once() {
    let (mut state, session) = session();
    let new = task(session, TaskWakePolicy::OnTerminal);
    apply_op(
        &mut state,
        session,
        TaskOp::Create { task: stored(&new) },
        T0,
    );
    let before = head(&state, session);
    let finish = || TaskOp::Update {
        task_id: "task-1".into(),
        update: Box::new(SessionTaskUpdate {
            state: Some(SessionTaskState::Succeeded),
            summary: Some("found it".into()),
            ..Default::default()
        }),
    };
    let done = value(apply_op(&mut state, session, finish(), T0 + 10));
    assert_eq!(done["state"], "succeeded");
    let woke = head(&state, session);
    assert!(woke > before, "the wake is in the owner's log");
    // The same update again (a retry): no second wake.
    apply_op(&mut state, session, finish(), T0 + 20);
    assert_eq!(head(&state, session), woke);
    // An update to a task that does not exist answers nothing.
    let missing = TaskOp::Update {
        task_id: "nope".into(),
        update: Box::default(),
    };
    assert_eq!(
        value(apply_op(&mut state, session, missing, T0)),
        Value::Null
    );
}

#[test]
fn a_silent_task_never_wakes() {
    let (mut state, session) = session();
    let new = task(session, TaskWakePolicy::Silent);
    apply_op(
        &mut state,
        session,
        TaskOp::Create { task: stored(&new) },
        T0,
    );
    let before = head(&state, session);
    let finish = TaskOp::Update {
        task_id: "task-1".into(),
        update: Box::new(SessionTaskUpdate {
            state: Some(SessionTaskState::Failed),
            ..Default::default()
        }),
    };
    apply_op(&mut state, session, finish, T0 + 10);
    assert_eq!(head(&state, session), before);
}

#[test]
fn messages_wake_on_activity_and_answers_resume_the_task() {
    let (mut state, session) = session();
    let new = task(session, TaskWakePolicy::OnActivity);
    apply_op(
        &mut state,
        session,
        TaskOp::Create { task: stored(&new) },
        T0,
    );

    // The task asks for input: the owner wakes.
    let before = head(&state, session);
    let ask = TaskOp::Update {
        task_id: "task-1".into(),
        update: Box::new(SessionTaskUpdate {
            input_request: Some(TaskInputRequest {
                id: "q1".into(),
                prompt: "which branch?".into(),
                expected: None,
            }),
            ..Default::default()
        }),
    };
    let asked = value(apply_op(&mut state, session, ask, T0 + 1));
    assert_eq!(asked["state"], "awaiting_input");
    assert!(head(&state, session) > before);

    let message = |id: &str, direction, reply: Option<&str>| TaskMessage {
        id: id.into(),
        task_id: "task-1".into(),
        direction,
        content: vec![everruns_core::session_task::TaskMessagePart::Text {
            text: "main".into(),
        }],
        in_reply_to: reply.map(str::to_owned),
        created_at: at(T0),
    };
    // The answer returns it to running and does not wake the owner.
    let before = head(&state, session);
    let answer = TaskOp::Message {
        message: Box::new(message("m1", TaskMessageDirection::Inbound, Some("q1"))),
        expected_attempt: None,
    };
    apply_op(&mut state, session, answer.clone(), T0 + 2);
    assert_eq!(head(&state, session), before);
    let platform = &state.sessions[&session].platform;
    assert_eq!(platform.tasks["task-1"]["state"], "running");
    // A retried message lands once.
    apply_op(&mut state, session, answer, T0 + 3);
    assert_eq!(
        state.sessions[&session].platform.task_messages["task-1"].len(),
        1
    );

    // An outbound message wakes the owner.
    let out = TaskOp::Message {
        message: Box::new(message("m2", TaskMessageDirection::Outbound, None)),
        expected_attempt: None,
    };
    apply_op(&mut state, session, out, T0 + 4);
    assert!(head(&state, session) > before);

    // A message for an older attempt is refused.
    let stale = TaskOp::Message {
        message: Box::new(message("m3", TaskMessageDirection::Inbound, None)),
        expected_attempt: Some(99),
    };
    assert!(matches!(
        apply_op(&mut state, session, stale, T0 + 5),
        Response::Rejected { .. }
    ));
}

#[test]
fn a_cancel_request_is_recorded_once() {
    let (mut state, session) = session();
    let new = task(session, TaskWakePolicy::Silent);
    apply_op(
        &mut state,
        session,
        TaskOp::Create { task: stored(&new) },
        T0,
    );
    let cancel = || TaskOp::RequestCancel {
        task_id: "task-1".into(),
    };
    let first = value(apply_op(&mut state, session, cancel(), T0 + 5));
    let again = value(apply_op(&mut state, session, cancel(), T0 + 9));
    assert_eq!(first["cancel_requested_at"], again["cancel_requested_at"]);
}

#[test]
fn task_commands_round_trip() {
    let op = crate::platform::Op::Task(TaskOp::RequestCancel {
        task_id: "t".into(),
    });
    let json = serde_json::to_string(&op).unwrap();
    let back: crate::platform::Op = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        back,
        crate::platform::Op::Task(TaskOp::RequestCancel { .. })
    ));
}

#[test]
fn a_person_cancels_a_task_or_messages_one_the_api_may_message() {
    let write = |body: serde_json::Value| -> ApiWrite { serde_json::from_value(body).unwrap() };
    assert!(matches!(
        api_op(write(json!({ "task": "task_1", "cancel": true })), "subagent"),
        Ok(TaskOp::RequestCancel { task_id }) if task_id == "task_1"
    ));
    let Ok(TaskOp::Message {
        message,
        expected_attempt,
    }) = api_op(
        write(json!({ "task": "task_1", "text": "use staging", "in_reply_to": "" })),
        "background",
    )
    else {
        panic!("a message to a background task is taken");
    };
    assert_eq!(message.direction, TaskMessageDirection::Inbound);
    assert_eq!(message.in_reply_to, None);
    assert_eq!(expected_attempt, None);
    for kind in ["subagent", "agent_handoff", "assignment"] {
        assert!(api_op(write(json!({ "task": "t", "text": "hi" })), kind).is_err());
    }
    assert!(api_op(write(json!({ "task": "t", "text": "  " })), "background").is_err());
}

fn push_config(id: &str, filter: &[&str], secret: Option<&str>) -> PushConfig {
    PushConfig {
        id: id.into(),
        url: format!("https://example.test/{id}"),
        secret: secret.map(str::to_owned),
        event_filter: filter.iter().map(|f| (*f).to_owned()).collect(),
        created_at: at(T0),
        updated_at: at(T0),
    }
}

fn transitions(state: &CellState) -> Vec<(String, Vec<String>, Value)> {
    state
        .feed
        .records
        .iter()
        .filter_map(|r| match &r.record {
            Record::TaskTransition {
                transition,
                targets,
                task,
                ..
            } => {
                assert_eq!(r.org_id, "org");
                Some((
                    transition.clone(),
                    targets.iter().map(|t| t.label.clone()).collect(),
                    task.clone(),
                ))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn push_configs_are_kept_with_their_secrets_and_listed_without_them() {
    let (mut state, session) = session();
    let new = task(session, TaskWakePolicy::Silent);
    apply_op(
        &mut state,
        session,
        TaskOp::Create { task: stored(&new) },
        T0,
    );
    let add = |config: PushConfig| TaskOp::AddPushConfig {
        task_id: "task-1".into(),
        config,
    };
    let shown = value(apply_op(
        &mut state,
        session,
        add(push_config("tpc_a", &["terminal"], Some("k1"))),
        T0,
    ));
    assert_eq!(shown["has_secret"], true);
    assert!(shown.get("secret").is_none(), "a secret is never shown");
    // A retried add keeps one.
    value(apply_op(
        &mut state,
        session,
        add(push_config("tpc_a", &["terminal"], Some("k1"))),
        T0,
    ));
    let kept = &state.sessions[&session].platform.task_push_configs["task-1"];
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].secret.as_deref(), Some("k1"));
    // A task the session does not have answers nothing.
    let missing = TaskOp::AddPushConfig {
        task_id: "nope".into(),
        config: push_config("tpc_b", &["terminal"], None),
    };
    assert_eq!(
        value(apply_op(&mut state, session, missing, T0)),
        Value::Null
    );
    // Deleting: once, then not found.
    let delete = || TaskOp::DeletePushConfig {
        task_id: "task-1".into(),
        config_id: "tpc_a".into(),
    };
    assert_eq!(
        value(apply_op(&mut state, session, delete(), T0))["deleted"],
        true
    );
    assert_eq!(
        value(apply_op(&mut state, session, delete(), T0))["deleted"],
        false
    );
    assert!(
        !state.sessions[&session]
            .platform
            .task_push_configs
            .contains_key("task-1")
    );
    // A secret stays out of the debug form too.
    assert!(!format!("{:?}", push_config("tpc_c", &[], Some("hidden"))).contains("hidden"));
}

#[test]
fn transitions_go_on_the_feed_with_the_targets_that_ask_for_them() {
    let (mut state, session) = session();
    let new = task(session, TaskWakePolicy::Silent);
    apply_op(
        &mut state,
        session,
        TaskOp::Create { task: stored(&new) },
        T0,
    );
    for config in [
        push_config("tpc_input", &["awaiting_input"], None),
        push_config("tpc_done", &["terminal"], Some("k")),
    ] {
        apply_op(
            &mut state,
            session,
            TaskOp::AddPushConfig {
                task_id: "task-1".into(),
                config,
            },
            T0,
        );
    }
    // Waiting for input: only the config that names it.
    let ask = TaskOp::Update {
        task_id: "task-1".into(),
        update: Box::new(SessionTaskUpdate {
            input_request: Some(TaskInputRequest {
                id: "q1".into(),
                prompt: "which branch?".into(),
                expected: None,
            }),
            ..Default::default()
        }),
    };
    apply_op(&mut state, session, ask, T0 + 1);
    // An outbound message no config names: nothing goes on the feed.
    let message = TaskMessage {
        id: "m1".into(),
        task_id: "task-1".into(),
        direction: TaskMessageDirection::Outbound,
        content: Vec::new(),
        in_reply_to: None,
        created_at: at(T0),
    };
    apply_op(
        &mut state,
        session,
        TaskOp::Message {
            message: Box::new(message),
            expected_attempt: None,
        },
        T0 + 2,
    );
    // Terminal: the stored config and the one given at spawn.
    let finish = || TaskOp::Update {
        task_id: "task-1".into(),
        update: Box::new(SessionTaskUpdate {
            state: Some(SessionTaskState::Succeeded),
            summary: Some("done".into()),
            ..Default::default()
        }),
    };
    apply_op(&mut state, session, finish(), T0 + 3);
    // A repeated update is no transition.
    apply_op(&mut state, session, finish(), T0 + 4);
    let seen = transitions(&state);
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[0].0, "awaiting_input");
    assert_eq!(seen[0].1, vec!["tpc_input".to_owned()]);
    assert_eq!(seen[1].0, "terminal");
    assert_eq!(seen[1].1, vec!["tpc_done".to_owned(), "spec".to_owned()]);
    assert_eq!(seen[1].2["state"], "succeeded");
    assert_eq!(seen[1].2["summary"], "done");
    let Record::TaskTransition { targets, .. } = &state.feed.records.back().unwrap().record else {
        panic!("a task transition");
    };
    assert_eq!(targets[0].secret.as_deref(), Some("k"));
    assert_eq!(targets[1].secret.as_deref(), Some("s3"));
}

#[test]
fn a_terminal_state_goes_on_the_feed_for_the_orgs_webhooks_even_without_push_configs() {
    let (mut state, session) = session();
    let mut new = task(session, TaskWakePolicy::Silent);
    new.spec = json!({});
    apply_op(
        &mut state,
        session,
        TaskOp::Create { task: stored(&new) },
        T0,
    );
    let fail = TaskOp::Update {
        task_id: "task-1".into(),
        update: Box::new(SessionTaskUpdate {
            state: Some(SessionTaskState::Failed),
            ..Default::default()
        }),
    };
    apply_op(&mut state, session, fail, T0 + 1);
    let seen = transitions(&state);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "terminal");
    assert!(seen[0].1.is_empty());
}

#[test]
fn event_filters_follow_the_control_planes_rules() {
    assert_eq!(event_filter(None).unwrap(), vec!["terminal"]);
    assert_eq!(
        event_filter(Some(vec!["message".into(), "message".into()])).unwrap(),
        vec!["message"]
    );
    assert!(event_filter(Some(vec!["everything".into()])).is_err());
}
