//! `Session::ag_ui`: one AG-UI 1.0 run per request, with `ask_user` and tool
//! approvals as interrupts through `InterruptGate` or a host's own
//! `InterruptSource`.
#![cfg(feature = "ag-ui")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use everruns::ag_ui::wire::RunFinishedOutcome;
use everruns::ag_ui::{
    ASK_USER_REASON, AgUiError, AgUiOptions, Event, Interrupt, InterruptGate, InterruptSource,
    Message, ResumeEntry, ResumeOutcome, ResumeStatus, RunAgentInput, TOOL_APPROVAL_REASON,
};
use everruns::{
    Agent, Engine, FunctionTool, LlmSimConfig, Model, SendDisposition, Session, SessionId, ToolCall,
};
use futures::StreamExt;
use serde_json::{Value, json};

fn input(text: &str) -> RunAgentInput {
    RunAgentInput {
        thread_id: "thread-1".into(),
        run_id: "run-1".into(),
        messages: vec![Message::user("m1", text)],
        ..RunAgentInput::default()
    }
}

fn resume(entries: Vec<ResumeEntry>) -> RunAgentInput {
    RunAgentInput {
        thread_id: "thread-1".into(),
        run_id: "run-2".into(),
        resume: entries,
        ..RunAgentInput::default()
    }
}

fn entry(id: &str, status: ResumeStatus, payload: Option<Value>) -> ResumeEntry {
    ResumeEntry {
        interrupt_id: id.into(),
        status,
        payload,
        metadata: None,
    }
}

async fn collect(session: &Session, input: RunAgentInput, gate: &InterruptGate) -> Vec<Event> {
    let stream = session
        .ag_ui_with(input, AgUiOptions::new().gate(gate.clone()))
        .await
        .expect("run starts");
    tokio::time::timeout(Duration::from_secs(10), stream.collect())
        .await
        .expect("run ends")
}

fn text(events: &[Event]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            Event::TextMessageContent(content) => Some(content.delta.as_str()),
            _ => None,
        })
        .collect()
}

fn outcome(events: &[Event]) -> Option<RunFinishedOutcome> {
    match events.last() {
        Some(Event::RunFinished(finished)) => finished.outcome.clone(),
        other => panic!("run must end with RUN_FINISHED, got {other:?}"),
    }
}

fn interrupts(events: &[Event]) -> Vec<everruns::ag_ui::Interrupt> {
    match outcome(events) {
        Some(RunFinishedOutcome::Interrupt { interrupts }) => interrupts,
        other => panic!("expected the interrupt outcome, got {other:?}"),
    }
}

/// Exactly one terminal event, and it is the last.
fn assert_well_formed(events: &[Event]) {
    assert!(matches!(events.first(), Some(Event::RunStarted(_))));
    let terminal = events.iter().filter(|event| event.is_terminal()).count();
    assert_eq!(terminal, 1, "{events:?}");
    assert!(events.last().is_some_and(Event::is_terminal));
}

#[tokio::test]
async fn a_text_turn_streams_as_one_run() {
    let agent = Agent::builder()
        .instructions("Be brief.")
        .model(Model::simulated("Hello from Everruns."))
        .build()
        .expect("valid agent");
    let session = Engine::new().create(agent);

    let events: Vec<Event> = session
        .ag_ui(RunAgentInput {
            protocol_version: Some("1.0".into()),
            ..input("Hi")
        })
        .await
        .expect("run starts")
        .collect()
        .await;

    assert_well_formed(&events);
    let Some(Event::RunStarted(started)) = events.first() else {
        unreachable!()
    };
    assert_eq!(started.thread_id, "thread-1");
    assert_eq!(started.protocol_version.as_deref(), Some("1.0"));
    assert_eq!(text(&events), "Hello from Everruns.");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::TextMessageStart(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::TextMessageEnd(_)))
    );
    assert_eq!(outcome(&events), None, "success has no outcome");
}

#[tokio::test]
async fn a_pre_1_0_client_gets_no_protocol_version() {
    let agent = Agent::builder()
        .instructions("Be brief.")
        .model(Model::simulated("ok"))
        .build()
        .expect("valid agent");
    let session = Engine::new().create(agent);
    let mut run = session.ag_ui(input("Hi")).await.expect("run starts");
    let Some(Event::RunStarted(started)) = run.recv().await else {
        panic!("RUN_STARTED first")
    };
    assert_eq!(started.protocol_version, None);
}

#[tokio::test]
async fn input_without_a_trailing_user_message_is_refused() {
    let agent = Agent::builder()
        .instructions("Be brief.")
        .model(Model::simulated("ok"))
        .build()
        .expect("valid agent");
    let session = Engine::new().create(agent);

    let empty = session.ag_ui(RunAgentInput::default()).await;
    assert!(matches!(empty, Err(AgUiError::InvalidInput(_))));

    let assistant_last = session
        .ag_ui(RunAgentInput {
            messages: vec![Message::user("m1", "Hi"), Message::assistant("m2", "Hello")],
            ..RunAgentInput::default()
        })
        .await;
    assert!(matches!(assistant_last, Err(AgUiError::InvalidInput(_))));
}

#[tokio::test]
async fn resume_entries_without_a_gate_finish_an_empty_run() {
    let agent = Agent::builder()
        .instructions("Be brief.")
        .model(Model::simulated("ok"))
        .build()
        .expect("valid agent");
    let session = Engine::new().create(agent);
    let events: Vec<Event> = session
        .ag_ui(resume(vec![entry("call_1", ResumeStatus::Cancelled, None)]))
        .await
        .expect("run starts")
        .collect()
        .await;
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(outcome(&events), None);
}

fn asking_agent(gate: &InterruptGate) -> Agent {
    let model = Model::simulated_with_config(
        LlmSimConfig::fixed("Deploying.").with_tool_call_sequence(vec![
            vec![ToolCall {
                id: "call_target".to_string(),
                name: "ask_user".to_string(),
                arguments: json!({
                    "questions": [{
                        "header": "Target",
                        "question": "Where should I deploy?",
                        "options": [
                            {"label": "Staging", "description": "Safe", "default": true},
                            {"label": "Production", "description": "Live"}
                        ]
                    }]
                }),
            }],
            vec![],
        ]),
    );
    Agent::builder()
        .instructions("Ask before choosing a deployment target.")
        .model(model)
        .ask_user(gate.clone())
        .build()
        .expect("valid agent")
}

#[tokio::test]
async fn ask_user_interrupts_and_a_resume_answers_it() {
    let gate = InterruptGate::new();
    let session = Engine::new().create(asking_agent(&gate));

    let first = collect(&session, input("Deploy the service."), &gate).await;
    assert_well_formed(&first);
    let open = interrupts(&first);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, "call_target");
    assert_eq!(open[0].reason, ASK_USER_REASON);
    assert_eq!(open[0].message.as_deref(), Some("Where should I deploy?"));
    let schema = open[0].response_schema.as_ref().expect("answer schema");
    assert_eq!(schema["properties"]["tool_call_id"]["const"], "call_target");
    assert_eq!(gate.interrupts(session.session_id()), open);

    let question_id =
        open[0].metadata.as_ref().expect("questions")["everruns"]["questions"][0]["id"]
            .as_str()
            .expect("question id")
            .to_string();
    let answer = json!({ "answers": [{ "id": question_id, "selected": ["Production"] }] });
    let second = collect(
        &session,
        resume(vec![entry(
            "call_target",
            ResumeStatus::Resolved,
            Some(answer),
        )]),
        &gate,
    )
    .await;
    assert_well_formed(&second);
    assert_eq!(outcome(&second), None, "{second:?}");
    assert_eq!(text(&second), "Deploying.");
    assert!(gate.interrupts(session.session_id()).is_empty());
}

#[tokio::test]
async fn an_unanswered_interrupt_is_asked_again() {
    let gate = InterruptGate::new();
    let session = Engine::new().create(asking_agent(&gate));
    let first = collect(&session, input("Deploy."), &gate).await;
    let open = interrupts(&first);

    // An entry for an interrupt nobody raised answers nothing.
    let unrelated = collect(
        &session,
        resume(vec![entry("call_other", ResumeStatus::Cancelled, None)]),
        &gate,
    )
    .await;
    assert_eq!(interrupts(&unrelated), open);

    // A new message cannot run past an open interrupt.
    let new_message = collect(&session, input("Never mind."), &gate).await;
    assert_eq!(interrupts(&new_message), open);
    assert_eq!(gate.interrupts(session.session_id()), open);
}

#[tokio::test]
async fn an_invalid_answer_is_refused_and_resolves_nothing() {
    let gate = InterruptGate::new();
    let session = Engine::new().create(asking_agent(&gate));
    let open = interrupts(&collect(&session, input("Deploy."), &gate).await);

    let refused = session
        .ag_ui_with(
            resume(vec![entry(
                "call_target",
                ResumeStatus::Resolved,
                Some(json!({ "answers": [{ "id": "question_1", "selected": ["Moon"] }] })),
            )]),
            AgUiOptions::new().gate(gate.clone()),
        )
        .await;
    assert!(matches!(refused, Err(AgUiError::InvalidInput(_))));
    assert_eq!(gate.interrupts(session.session_id()), open);
}

#[tokio::test]
async fn abandoning_a_question_declines_it_and_the_turn_continues() {
    let gate = InterruptGate::new();
    let session = Engine::new().create(asking_agent(&gate));
    interrupts(&collect(&session, input("Deploy."), &gate).await);

    let resumed = collect(
        &session,
        resume(vec![entry("call_target", ResumeStatus::Cancelled, None)]),
        &gate,
    )
    .await;
    assert_well_formed(&resumed);
    assert_eq!(outcome(&resumed), None);
    assert!(gate.interrupts(session.session_id()).is_empty());
}

fn gated_agent(gate: &InterruptGate, runs: Arc<AtomicUsize>) -> Agent {
    let deploy = FunctionTool::new(
        "deploy",
        "Deploy the service.",
        json!({ "type": "object", "properties": { "env": { "type": "string" } } }),
        move |_args: Value| {
            let runs = runs.clone();
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>(json!({ "deployed": true }))
            }
        },
    )
    .always_needs_approval();
    let model =
        Model::simulated_with_config(LlmSimConfig::fixed("Done.").with_tool_call_sequence(vec![
            vec![ToolCall {
                id: "call_deploy".to_string(),
                name: "deploy".to_string(),
                arguments: json!({ "env": "production" }),
            }],
            vec![],
        ]));
    Agent::builder()
        .instructions("Deploy when asked.")
        .model(model)
        .tool(deploy)
        .approver(gate.clone())
        .build()
        .expect("valid agent")
}

async fn approve(decision: Option<&str>) -> (Vec<Event>, usize) {
    let gate = InterruptGate::new();
    let runs = Arc::new(AtomicUsize::new(0));
    let session = Engine::new().create(gated_agent(&gate, runs.clone()));

    let first = collect(&session, input("Deploy."), &gate).await;
    let open = interrupts(&first);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].reason, TOOL_APPROVAL_REASON);
    assert_eq!(open[0].tool_call_id.as_deref(), Some("call_deploy"));
    assert_eq!(
        open[0].metadata.as_ref().expect("call")["everruns"]["arguments"],
        json!({ "env": "production" })
    );
    assert_eq!(
        runs.load(Ordering::SeqCst),
        0,
        "nothing runs before approval"
    );

    let answer = match decision {
        Some(decision) => entry(
            "call_deploy",
            ResumeStatus::Resolved,
            Some(json!({ "decision": decision })),
        ),
        None => entry("call_deploy", ResumeStatus::Cancelled, None),
    };
    let second = collect(&session, resume(vec![answer]), &gate).await;
    assert_well_formed(&second);
    (second, runs.load(Ordering::SeqCst))
}

#[tokio::test]
async fn an_allowed_tool_call_runs_after_resume() {
    let (events, runs) = approve(Some("allow")).await;
    assert_eq!(runs, 1);
    assert_eq!(outcome(&events), None);
    assert_eq!(text(&events), "Done.");
}

#[tokio::test]
async fn a_rejected_or_abandoned_tool_call_never_runs() {
    let (rejected, runs) = approve(Some("reject")).await;
    assert_eq!(runs, 0);
    assert_eq!(outcome(&rejected), None);

    let (abandoned, runs) = approve(None).await;
    assert_eq!(runs, 0);
    assert_eq!(outcome(&abandoned), None);
}

#[tokio::test]
async fn an_unknown_decision_is_refused() {
    let gate = InterruptGate::new();
    let session = Engine::new().create(gated_agent(&gate, Arc::default()));
    interrupts(&collect(&session, input("Deploy."), &gate).await);
    let refused = session
        .ag_ui_with(
            resume(vec![entry(
                "call_deploy",
                ResumeStatus::Resolved,
                Some(json!({ "decision": "maybe" })),
            )]),
            AgUiOptions::new().gate(gate.clone()),
        )
        .await;
    assert!(matches!(refused, Err(AgUiError::InvalidInput(_))));
    assert_eq!(gate.interrupts(session.session_id()).len(), 1);
}

/// A host's own source: delegates to a gate and counts resumes, standing in
/// for a host that parks requests for an API of its own.
struct CountingSource {
    gate: InterruptGate,
    resumes: Arc<AtomicUsize>,
}

impl InterruptSource for CountingSource {
    fn interrupts(&self, session_id: SessionId) -> Vec<Interrupt> {
        self.gate.interrupts(session_id)
    }

    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<SessionId> {
        InterruptSource::subscribe(&self.gate)
    }

    fn resume(
        &self,
        session_id: SessionId,
        entries: &[ResumeEntry],
    ) -> Result<ResumeOutcome, AgUiError> {
        self.resumes.fetch_add(1, Ordering::SeqCst);
        InterruptSource::resume(&self.gate, session_id, entries)
    }
}

#[tokio::test]
async fn a_host_interrupt_source_interrupts_and_resumes() {
    let gate = InterruptGate::new();
    let resumes = Arc::new(AtomicUsize::new(0));
    let session = Engine::new().create(asking_agent(&gate));
    let options = || {
        AgUiOptions::new().interrupts(CountingSource {
            gate: gate.clone(),
            resumes: resumes.clone(),
        })
    };
    let run = |input| async {
        let stream = session
            .ag_ui_with(input, options())
            .await
            .expect("run starts");
        tokio::time::timeout(Duration::from_secs(10), stream.collect::<Vec<_>>())
            .await
            .expect("run ends")
    };

    let first = run(input("Deploy.")).await;
    let open = interrupts(&first);
    let question_id =
        open[0].metadata.as_ref().expect("questions")["everruns"]["questions"][0]["id"]
            .as_str()
            .expect("question id")
            .to_string();
    let answer = json!({ "answers": [{ "id": question_id, "selected": ["Staging"] }] });
    let second = run(resume(vec![entry(
        "call_target",
        ResumeStatus::Resolved,
        Some(answer),
    )]))
    .await;
    assert_well_formed(&second);
    assert_eq!(text(&second), "Deploying.");
    assert_eq!(resumes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_run_reports_the_message_it_sent() {
    let agent = Agent::builder()
        .instructions("Be brief.")
        .model(Model::simulated("ok"))
        .build()
        .expect("valid agent");
    let session = Engine::new().create(agent);

    let run = session.ag_ui(input("Hi")).await.expect("run starts");
    let sent = run.sent().expect("a user message starts a turn").clone();
    assert_eq!(sent.disposition, SendDisposition::Started);
    let events: Vec<Event> = run.collect().await;
    assert_well_formed(&events);
    assert!(sent.wait().await.expect("turn ends").success);

    // A resume with nothing open sends nothing.
    let gate = InterruptGate::new();
    let empty = session
        .ag_ui_with(
            resume(vec![entry("none", ResumeStatus::Cancelled, None)]),
            AgUiOptions::new().gate(gate),
        )
        .await
        .expect("run starts");
    assert!(empty.sent().is_none());
}
