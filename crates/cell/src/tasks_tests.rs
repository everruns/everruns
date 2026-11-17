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
