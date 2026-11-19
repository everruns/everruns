use super::*;

#[tokio::test]
async fn raw_evidence_pressure_compacts_below_both_budgets_and_preserves_raw_history() {
    let rig = ProactiveTestRig::new(
        DriverId::external("openai-codex-cost-pressure"),
        1_000_000,
        (10_000, 100),
        false,
        false,
    )
    .await;
    let mut trajectory = vec![RuntimeMessage::user(
        "Find the decisive evidence and complete the task without losing it.",
    )];
    for index in 0..12 {
        let call_id = format!("call_{index}");
        trajectory.push(RuntimeMessage::assistant_with_tools(
            "",
            vec![ToolCall {
                id: call_id.clone(),
                name: "read_file".to_string(),
                arguments: json!({ "path": format!("evidence/{index}.txt") }),
            }],
        ));
        let marker = (index == 3).then_some("DECISIVE-EVIDENCE=WREN-5081\n");
        trajectory.push(RuntimeMessage::tool_result(
            call_id,
            Some(json!({
                "output": format!("{}{}", marker.unwrap_or_default(), "x".repeat(24_000))
            })),
            None,
        ));
        if index == 3 {
            trajectory.push(RuntimeMessage::assistant(
                "Decision recorded from DECISIVE-EVIDENCE: use WREN-5081.",
            ));
        }
    }
    trajectory.push(RuntimeMessage::user("Use WREN-5081 and finish now."));
    let baseline_bytes = serde_json::to_vec(&trajectory).unwrap().len();
    rig.configure_cost_pressure(trajectory.clone()).await;

    let result = rig.execute(None).await.unwrap();

    assert!(result.success);
    assert_eq!(result.text, "ok");
    assert_eq!(rig.compact_attempts.load(Ordering::SeqCst), 1);
    let events = rig.event_emitter.events().await;
    assert!(events.iter().any(|event| matches!(&event.data,
        everruns_core::EventData::ContextCompacted(data) if data.trigger == everruns_core::events::CompactionTrigger::CostPressure)), "raw evidence must trigger below both window and working budgets");
    let calls = rig.calls.lock().await;
    let model_view_bytes =
        everruns_core::builtins::estimate_total_tokens(&calls.last().unwrap().0) * 4;
    let reduction_percent = 100usize.saturating_sub(model_view_bytes * 100 / baseline_bytes);
    println!(
        "context_cost_ab baseline_prompt_bytes={baseline_bytes} candidate_prompt_bytes={model_view_bytes} reduction_percent={reduction_percent} task_success={}",
        result.success
    );
    assert!(
        model_view_bytes * 4 < baseline_bytes,
        "durable replacement should reduce the next model view by at least 75%: {model_view_bytes} vs {}",
        baseline_bytes
    );
    drop(calls);

    let raw = rig
        .message_retriever
        .load(rig.session_id.into())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&raw).unwrap(),
        serde_json::to_value(&trajectory).unwrap()
    );
    let queryable = rig
        .message_retriever
        .load_filtered(
            everruns_core::MessageQuery::new(rig.session_id.into()).with_filter(
                everruns_core::message_filter::MessageFilter::Search(
                    "DECISIVE-EVIDENCE".to_string(),
                ),
            ),
        )
        .await
        .unwrap();
    assert_eq!(queryable.len(), 1);
    assert!(
        queryable[0]
            .text()
            .is_some_and(|text| text.contains("WREN-5081"))
    );

    rig.message_retriever
        .add(
            rig.session_id.into(),
            everruns_core::InputMessage::user(
                "Latest validation passed; keep this visible and finish.",
            ),
        )
        .await
        .unwrap();
    let resumed = rig.execute(None).await.unwrap();
    assert!(resumed.success);
    assert_eq!(rig.compact_attempts.load(Ordering::SeqCst), 1);
    let calls = rig.calls.lock().await;
    let (messages, config) = calls.last().unwrap();
    assert!(config.provider_opaque_context.is_some());
    assert!(messages.iter().any(|message| {
        matches!(
            &message.content,
            everruns_contracts::driver_registry::MessageContent::Text(text)
                if text.contains("Latest validation passed")
        )
    }));
}

#[tokio::test]
async fn lifetime_usage_does_not_compact_a_short_prompt() {
    let rig = ProactiveTestRig::new(
        DriverId::external("openai-codex-short-cost-pressure"),
        1_000_000,
        (10_000, 100),
        false,
        false,
    )
    .await;
    rig.configure_cost_pressure(vec![RuntimeMessage::user("short follow-up")])
        .await;

    rig.execute(None).await.unwrap();

    assert_eq!(rig.compact_attempts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn raw_evidence_compaction_failure_preserves_raw_model_view() {
    let rig = ProactiveTestRig::new(
        DriverId::external("openai-codex-cost-failure"),
        1_000_000,
        (10_000, 100),
        false,
        true,
    )
    .await;
    let trajectory = vec![
        RuntimeMessage::user("Read the evidence and retain it."),
        RuntimeMessage::assistant_with_tools(
            "",
            vec![ToolCall {
                id: "large".into(),
                name: "read_file".into(),
                arguments: json!({"path":"evidence.txt"}),
            }],
        ),
        RuntimeMessage::tool_result("large", Some(json!({"output":"x".repeat(400_000)})), None),
        RuntimeMessage::user("Finish using the evidence."),
    ];
    rig.configure_cost_pressure(trajectory.clone()).await;

    rig.execute(None).await.unwrap();
    rig.execute(None).await.unwrap();

    assert_eq!(rig.compact_attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        serde_json::to_value(
            rig.message_retriever
                .load(rig.session_id.into())
                .await
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(trajectory).unwrap()
    );
    assert!(
        rig.checkpoint_store
            .get_latest(
                rig.session_id.into(),
                rig.provider_type.as_str(),
                &rig.model,
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        rig.calls
            .lock()
            .await
            .iter()
            .all(|(_, config)| config.provider_opaque_context.is_none())
    );
}

#[tokio::test]
async fn lifetime_usage_does_not_compact_text_below_both_budgets() {
    let rig = ProactiveTestRig::new(
        DriverId::external("text-cost-pressure"),
        1_000_000,
        (10_000, 100),
        false,
        false,
    )
    .await;
    rig.configure_cost_pressure(vec![RuntimeMessage::user("x".repeat(400_000))])
        .await;
    rig.execute(None).await.unwrap();
    assert_eq!(rig.compact_attempts.load(Ordering::SeqCst), 0);
}
