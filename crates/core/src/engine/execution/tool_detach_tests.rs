//! A long call leaves its act for the host, and the act parks the turn.
use super::*;
use crate::engine::execution::{ActAtom, ActInput, ExecutionContext};
use crate::engine::test_fixtures::TestEventEmitter;
use crate::engine::tool_execution::ToolExecutor;
use crate::engine::tool_types::{ToolCall, ToolHints, ToolResult};
use everruns_contracts::typed_id::{AgentId, HarnessId};
use serde_json::json;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A tool that takes `0` to answer.
struct Slow(Duration);

#[async_trait]
impl ToolExecutor for Slow {
    async fn execute(&self, call: &ToolCall, _: &ToolDefinition) -> Result<ToolResult> {
        tokio::time::sleep(self.0).await;
        Ok(ToolResult {
            tool_call_id: call.id.clone(),
            result: Some(json!({"done": true})),
            error: None,
            details: None,
            images: None,
            connection_required: None,
            raw_output: None,
        })
    }
}

#[derive(Default)]
struct Host {
    accept: bool,
    asked: AtomicUsize,
    emitter: TestEventEmitter,
    work: Arc<Mutex<Vec<DetachedWork>>>,
}

#[async_trait]
impl ToolDetacher for Host {
    fn detach_after(&self) -> Duration {
        Duration::from_millis(10)
    }

    async fn detach(&self, _call: &DetachedToolCall) -> Option<Detached> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        if !self.accept {
            return None;
        }
        let work = self.work.clone();
        Some(Detached {
            emitter: Arc::new(self.emitter.clone()),
            run: Box::new(move |run| work.lock().unwrap().push(run)),
        })
    }
}

fn definition(cpu_bound: bool) -> ToolDefinition {
    ToolDefinition::Builtin(everruns_contracts::BuiltinTool {
        name: "build".into(),
        display_name: None,
        description: "slow fixture".into(),
        parameters: json!({"type":"object"}),
        policy: Default::default(),
        category: None,
        deferrable: Default::default(),
        hints: ToolHints::default().with_cpu_bound(cpu_bound),
        full_parameters: None,
    })
}

fn input(cpu_bound: bool) -> ActInput {
    ActInput {
        org_id: Some(1),
        context: ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new()),
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        tool_calls: vec![ToolCall {
            id: "call_build".into(),
            name: "build".into(),
            arguments: json!({}),
        }],
        tool_definitions: vec![definition(cpu_bound)],
        locale: None,
        blueprint_id: None,
        network_access: None,
        parallel_tool_calls: None,
    }
}

async fn types(emitter: &TestEventEmitter) -> Vec<String> {
    emitter
        .events()
        .await
        .into_iter()
        .map(|event| event.event_type)
        .collect()
}

/// Run one act of `input` under `host`, as a leased turn's loop runs it.
async fn act(
    host: Arc<Host>,
    tool: Duration,
    input: ActInput,
) -> (
    Result<crate::engine::execution::ActResult>,
    TestEventEmitter,
) {
    let turn = TestEventEmitter::new();
    let atom = ActAtom::new(Slow(tool), turn.clone());
    let result = provide(host, for_act(atom.execute(input))).await;
    (result, turn)
}

#[tokio::test]
async fn a_call_past_the_threshold_detaches_and_the_act_parks() {
    let host = Arc::new(Host {
        accept: true,
        ..Host::default()
    });
    let (result, turn) = act(host.clone(), Duration::from_millis(200), input(false)).await;

    let error = result.expect_err("the act parks");
    assert!(everruns_contracts::durable_retry::is_parked(&error));
    // The act did not finish: no completion of the call or the act from it.
    assert_eq!(types(&turn).await, ["act.started", "tool.started"]);

    let work = host
        .work
        .lock()
        .unwrap()
        .pop()
        .expect("the host got the call");
    work.await;
    assert_eq!(types(&host.emitter).await, ["tool.completed"]);
    assert!(
        types(&turn).await.len() == 2,
        "nothing more through the turn"
    );
}

#[tokio::test]
async fn a_call_the_host_declines_stays_with_its_turn() {
    let host = Arc::new(Host::default());
    let (result, turn) = act(host.clone(), Duration::from_millis(60), input(false)).await;

    let result = result.expect("the act finishes");
    assert!(result.results[0].success);
    assert_eq!(host.asked.load(Ordering::SeqCst), 1);
    assert_eq!(
        types(&turn).await,
        [
            "act.started",
            "tool.started",
            "tool.completed",
            "act.completed"
        ]
    );
}

#[tokio::test]
async fn a_call_under_the_threshold_never_asks_the_host() {
    let host = Arc::new(Host {
        accept: true,
        ..Host::default()
    });
    let (result, _) = act(host.clone(), Duration::ZERO, input(false)).await;

    assert!(result.expect("the act finishes").results[0].success);
    assert_eq!(host.asked.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cpu_bound_calls_never_detach() {
    let host = Arc::new(Host {
        accept: true,
        ..Host::default()
    });
    let (result, _) = act(host.clone(), Duration::from_millis(60), input(true)).await;

    assert!(result.expect("the act finishes").results[0].success);
    assert_eq!(host.asked.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn only_the_turn_loops_act_sees_the_detacher() {
    let host: Arc<dyn ToolDetacher> = Arc::new(Host::default());
    assert!(active().is_none());
    provide(host.clone(), async {
        // Provided, but no act of the turn's own loop is running.
        assert!(active().is_none());
        for_act(async {
            assert!(active().is_some());
            // A tool the act runs inline, and any turn it runs, see none.
            without(async {
                assert!(active().is_none());
                for_act(async { assert!(active().is_none()) }).await;
            })
            .await;
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn the_switch_follows_the_call_but_keeps_the_turns_trace() {
    let turn = TestEventEmitter::new();
    let host = TestEventEmitter::new();
    let switch = SwitchEmitter::new(Arc::new(turn.clone()));
    let session = SessionId::new();
    let request = || {
        EventRequest::new(
            session,
            EventContext::turn(TurnId::new(), MessageId::new()),
            crate::engine::events::ToolProgressData {
                tool_call_id: "call".into(),
                tool_name: "build".into(),
                message: "half way".into(),
                display_name: None,
                current: None,
                total: None,
            },
        )
    };
    switch.emit(request()).await.unwrap();
    switch.redirect(Arc::new(host.clone()));
    switch.emit(request()).await.unwrap();
    assert_eq!(turn.events().await.len(), 1);
    assert_eq!(host.events().await.len(), 1);
}

#[test]
fn only_server_calls_that_do_not_shape_the_turn_may_detach() {
    assert!(may_detach(&definition(false)));
    assert!(!may_detach(&definition(true)));
    let mut direct = definition(false);
    if let ToolDefinition::Builtin(tool) = &mut direct {
        tool.hints.stays_direct = Some(true);
    }
    assert!(!may_detach(&direct));
    let mut native = definition(false);
    if let ToolDefinition::Builtin(tool) = &mut native {
        tool.name = NATIVE_ONLY_TOOL_NAMES[0].to_string();
    }
    assert!(!may_detach(&native));
}
