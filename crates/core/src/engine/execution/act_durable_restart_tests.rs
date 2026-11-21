use super::*;

#[derive(Clone, Default)]
struct StopsBeforeCompletion(Arc<tokio::sync::Notify>);

#[async_trait]
impl EventEmitter for StopsBeforeCompletion {
    async fn emit(&self, request: EventRequest) -> crate::error::Result<crate::events::Event> {
        if request.event_type == "tool.completed" {
            self.0.notify_one();
            return std::future::pending().await;
        }
        NoopEventEmitter.emit(request).await
    }
}

#[tokio::test]
async fn tool_result_is_durable_if_worker_stops_before_emitting_completion() {
    let store = Arc::new(InMemoryDurableStore::default());
    let emitter = StopsBeforeCompletion::default();
    let mut executor = ToolRegistry::with_defaults();
    executor.register(ArgumentEchoTool);
    let atom =
        ActAtom::new(executor, emitter.clone()).with_durable_tool_result_store(store.clone());
    let context = ExecutionContext::new(SessionId::new(), TurnId::new(), MessageId::new());
    let turn_id = context.turn_id.to_string();
    let call = ToolCall {
        id: "call_durable".into(),
        name: "argument_echo".into(),
        arguments: json!({"value": "worker output"}),
    };
    let input = make_act_input_with_store(
        call,
        vec![arg_echo_tool_def(SideEffectClass::AtMostOnce)],
        context,
    );

    let task = tokio::spawn(async move { atom.execute(input).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), emitter.0.notified())
        .await
        .expect("the worker reached the completion write");
    task.abort();
    let settled = store
        .get_tool_call_status(&turn_id, "call_durable")
        .await
        .unwrap();
    let Some(crate::durability::DurableToolCallStatus::Settled { result_json }) = settled else {
        panic!("the next worker must see the result, got {settled:?}");
    };
    assert!(result_json.to_string().contains("worker output"));
}
