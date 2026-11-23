use super::*;

struct SteeringRaceTool {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Tool for SteeringRaceTool {
    fn name(&self) -> &str {
        "argument_echo"
    }
    fn description(&self) -> &str {
        "deterministic steering race"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type":"object"})
    }
    async fn execute(&self, args: serde_json::Value) -> ToolExecutionResult {
        let label = args["value"].as_str().unwrap().to_string();
        self.calls.lock().unwrap().push(label.clone());
        if label == "A" {
            self.entered.notify_one();
            self.release.notified().await;
        }
        ToolExecutionResult::success(json!({"completed": label}))
    }
}

/// A has entered the actual executor; B is blocked by the same class. Neither
/// a sleep nor a paid provider is used. A must settle normally before B gets
/// its paired skipped result. A plain follow-up is the negative control.
async fn steering_race(explicit: bool, serialize_all: bool) {
    use crate::engine::tool_steering::{REDIRECT_TURN, ToolBatchSteering};
    use crate::host::{AcceptedTurnInput, TurnSteering};
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ToolRegistry::new();
    registry.register(SteeringRaceTool {
        entered: entered.clone(),
        release: release.clone(),
        calls: calls.clone(),
    });
    let steering = TurnSteering::new();
    let store = Arc::new(InMemoryDurableStore::default());
    let atom = ActAtom::new(registry, NoopEventEmitter)
        .with_tool_batch_steering(Some(Arc::new(steering.clone())))
        .with_durable_tool_result_store(store.clone());
    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let turn = context.turn_id;
    let mut definition = arg_echo_tool_def(SideEffectClass::AtMostOnce);
    if let ToolDefinition::Builtin(ref mut tool) = definition {
        tool.hints.concurrency_class = Some("steering-race".into());
    }
    let call = |name: &str| ToolCall {
        id: name.into(),
        name: "argument_echo".into(),
        arguments: json!({"value":name}),
    };
    let mut input = make_act_input_with_store(call("A"), vec![definition], context);
    input.tool_calls.push(call("B"));
    input.parallel_tool_calls = Some(!serialize_all);
    let redirect = async {
        entered.notified().await;
        let mut message = crate::InputMessage::user("Use the new direction instead");
        // Even ordinary caller metadata cannot opt into redirection.
        message = message.with_metadata(REDIRECT_TURN, json!(turn.to_string()));
        if explicit {
            steering
                .try_redirect(AcceptedTurnInput::new(message), turn)
                .unwrap();
        } else {
            steering.try_push(AcceptedTurnInput::new(message)).unwrap();
        }
        assert_eq!(*calls.lock().unwrap(), vec!["A"]);
        release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(atom.execute(input.clone()), redirect)
    })
    .await
    .expect("steering race must settle");
    let result = result.unwrap();
    assert_eq!(
        result
            .results
            .iter()
            .map(|r| r.result.tool_call_id.as_str())
            .collect::<Vec<_>>(),
        vec!["A", "B"]
    );
    assert!(result.results[0].success);
    assert_eq!(
        result.results[1].status,
        if explicit { "skipped" } else { "success" }
    );
    assert_eq!(
        *calls.lock().unwrap(),
        if explicit { vec!["A"] } else { vec!["A", "B"] }
    );
    // The gate does not consume the redirect. The next reason boundary does.
    assert_eq!(steering.redirected(turn).await.unwrap(), explicit);
    assert_eq!(steering.drain().len(), 1);
    assert!(!steering.redirected(turn).await.unwrap());
    // Retained results win after a restart even after the input was consumed.
    // Use a new atom/steering handle to avoid testing only warm state.
    let restarted =
        ActAtom::new(ToolRegistry::new(), NoopEventEmitter).with_durable_tool_result_store(store);
    let replay = restarted.execute(input).await.unwrap();
    assert_eq!(
        replay.results[1].result.error,
        result.results[1].result.error
    );
    assert_eq!(replay.results[1].status, result.results[1].status);
    assert_eq!(
        *calls.lock().unwrap(),
        if explicit { vec!["A"] } else { vec!["A", "B"] }
    );
}

#[tokio::test]
async fn explicit_redirect_skips_class_waiter_and_replays() {
    steering_race(true, false).await;
}

#[tokio::test]
async fn explicit_redirect_skips_sequential_waiter() {
    steering_race(true, true).await;
}

#[tokio::test]
async fn ordinary_follow_up_does_not_skip_class_waiter() {
    steering_race(false, false).await;
}

#[tokio::test]
async fn redirect_after_crash_between_claim_and_skip_never_redispatches() {
    use crate::engine::tool_fingerprint::tool_call_fingerprint;
    use crate::host::{AcceptedTurnInput, TurnSteering};
    let store = Arc::new(InMemoryDurableStore::default());
    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let turn = context.turn_id;
    let call = ToolCall {
        id: "B".into(),
        name: "argument_echo".into(),
        arguments: json!({"value":"B"}),
    };
    store
        .try_claim_tool_call(
            &turn.to_string(),
            &call.id,
            &call.name,
            &tool_call_fingerprint(&call),
        )
        .await
        .unwrap();
    // A new owner reconstructs a retained redirect while the prior owner left
    // only the claim, not the skip receipt. Even Pure must not redispatch.
    let steering = TurnSteering::new();
    steering
        .try_redirect(
            AcceptedTurnInput::new(crate::InputMessage::user("redirect")),
            turn,
        )
        .unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(ArgumentEchoTool);
    let atom = ActAtom::new(registry, NoopEventEmitter)
        .with_durable_tool_result_store(store)
        .with_tool_batch_steering(Some(Arc::new(steering)));
    let input = make_act_input_with_store(
        call,
        vec![arg_echo_tool_def(SideEffectClass::Pure)],
        context,
    );
    let result = atom.execute(input).await.unwrap();
    assert_eq!(result.results[0].status, "interrupted");
    assert!(
        result.results[0]
            .result
            .error
            .as_ref()
            .unwrap()
            .contains("uncertain")
    );
    assert!(!result.results[0].success);
}
