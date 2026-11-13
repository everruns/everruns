//! Metadata-only traces retain errors and timing without disclosing free text.
use super::*;

const PRIVATE: &str = "private-input-token-sentinel";

fn turn_failure() -> (Event, TurnFailedData) {
    let turn_id = TurnId::new();
    let data = TurnFailedData {
        turn_id,
        error: PRIVATE.into(),
        error_code: Some("provider_error".into()),
        error_fields: None,
        error_disclosure: None,
    };
    let event = Event::new(
        SessionId::new(),
        EventContext::turn(turn_id, MessageId::new()),
        EventData::TurnFailed(data.clone()),
    );
    (event, data)
}

#[test]
fn failed_and_cancelled_turns_close_without_usage_and_redact_free_text() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let (event, data) = turn_failure();
    let failed = listener.convert_turn_failed(&event, &data);
    assert_eq!(failed.error.as_deref(), Some("[REDACTED]"));
    assert_eq!(failed.metadata["error_code"], "provider_error");
    assert!(failed.metrics.as_ref().unwrap().end.is_some());
    assert!(failed.metrics.as_ref().unwrap().tokens.is_none());
    assert!(!serde_json::to_string(&failed).unwrap().contains(PRIVATE));

    let cancelled = TurnCancelledData {
        turn_id: data.turn_id,
        reason: Some(PRIVATE.into()),
        usage: None,
    };
    let cancelled = listener.convert_turn_cancelled(&event, &cancelled);
    assert_eq!(cancelled.metadata["cancellation_reason"], "[REDACTED]");
    assert!(cancelled.metrics.unwrap().end.is_some());
}

#[test]
fn failed_generation_without_usage_retains_duration_and_error_status() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let mut metadata = bare_generation_metadata("model");
    metadata.success = false;
    metadata.error = Some(PRIVATE.into());
    metadata.duration_ms = Some(1500);
    metadata.time_to_first_token_ms = Some(200);
    let data = LlmGenerationData {
        messages: vec![],
        tools: vec![],
        request_body: None,
        output: LlmGenerationOutput {
            text: None,
            tool_calls: vec![],
        },
        metadata,
    };
    let event = Event::new(
        SessionId::new(),
        EventContext::empty(),
        EventData::LlmGeneration(data.clone()),
    );
    let exported = listener.convert_llm_generation(&event, &data);
    assert_eq!(exported.error.as_deref(), Some("[REDACTED]"));
    assert_eq!(exported.metadata["generation_success"], false);
    let metrics = exported.metrics.unwrap();
    assert_eq!(metrics.end.unwrap() - metrics.start.unwrap(), 1.5);
    assert_eq!(metrics.time_to_first_token, Some(0.2));
    assert!(metrics.tokens.is_none());
}

#[test]
fn reason_and_tool_errors_obey_content_policy_even_in_top_level_error_field() {
    let mut config = test_config();
    config.content.record_content = true;
    // Messages opted in must not override tool-results redaction.
    config.content.tool_results_mode = BraintrustPayloadMode::None;
    let listener = BraintrustListener::new(config).unwrap();
    let data: ToolCompletedData = serde_json::from_value(json!({
        "tool_call_id": "tool-1", "tool_name": "read_file", "success": false,
        "status": "error", "error": PRIVATE
    }))
    .unwrap();
    let event = Event::new(
        SessionId::new(),
        EventContext::empty(),
        EventData::ToolCompleted(data.clone()),
    );
    let exported = listener.convert_tool_call_completed(&event, &data);
    assert_eq!(exported.error.as_deref(), Some("[REDACTED]"));
    assert!(!serde_json::to_string(&exported).unwrap().contains(PRIVATE));
    assert!(exported.metrics.unwrap().end.is_some());

    let reason = ReasonCompletedData {
        success: false,
        text_preview: None,
        has_tool_calls: false,
        tool_call_count: 0,
        error: Some(PRIVATE.into()),
        duration_ms: None,
        usage: None,
    };
    let listener = BraintrustListener::new(test_config()).unwrap();
    let exported = listener.convert_reason_completed(&event, &reason);
    assert_eq!(exported.error.as_deref(), Some("[REDACTED]"));
    assert!(exported.metrics.unwrap().end.is_some());
}

#[test]
fn explicit_content_capture_keeps_diagnostic_errors() {
    let mut config = test_config();
    config.content.record_content = true;
    let listener = BraintrustListener::new(config).unwrap();
    let (event, data) = turn_failure();
    assert_eq!(
        listener.convert_turn_failed(&event, &data).error.as_deref(),
        Some(PRIVATE)
    );
}

#[tokio::test]
async fn flush_reports_rejected_batch_not_successful_drain() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/project_logs/test-project-id/insert"))
        .respond_with(ResponseTemplate::new(403).set_body_string(PRIVATE))
        .expect(1)
        .mount(&server)
        .await;
    let mut config = test_config();
    config.api_url = server.uri();
    let listener = BraintrustListener::new(config).unwrap();
    listener.on_event(&turn_failure().0).await;
    let error = listener.flush().await.unwrap_err();
    assert_eq!(error, "Braintrust delivery lost events or failed batches");
    assert!(!error.contains(PRIVATE));
}

#[tokio::test]
async fn flush_reports_queue_loss_even_when_receiver_accepts_batches() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    listener
        .state
        .dropped_events
        .fetch_add(1, Ordering::Relaxed);
    assert!(listener.flush().await.is_err());
}

#[test]
fn disjoint_usage_maps_to_braintrust_inclusive_prompt_and_native_cache_names() {
    let (event, _) = turn_failure();
    let usage = TokenUsage::with_cache(10, 5, Some(20), Some(30));
    let metrics = super::super::quality::terminal_metrics(&event, None, Some(&usage), None);
    let payload = serde_json::to_value(metrics).unwrap();
    assert_eq!(payload["prompt_tokens"], 60);
    assert_eq!(payload["completion_tokens"], 5);
    assert_eq!(payload["tokens"], 65);
    assert_eq!(payload["prompt_cached_tokens"], 20);
    assert_eq!(payload["prompt_cache_creation_tokens"], 30);
    assert!(payload.get("cache_read_tokens").is_none());
    assert!(payload.get("cache_creation_tokens").is_none());
}

#[test]
fn known_costs_preserve_actual_estimated_and_effective_provenance() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let mut metadata = bare_generation_metadata("model");
    metadata.usage = Some(TokenUsage {
        actual_cost_usd: Some(0.0),
        estimated_cost_usd: Some(0.25),
        effective_cost_usd: Some(0.1),
        ..TokenUsage::new(10, 5)
    });
    let data = LlmGenerationData {
        messages: vec![],
        tools: vec![],
        request_body: None,
        output: LlmGenerationOutput {
            text: None,
            tool_calls: vec![],
        },
        metadata,
    };
    let event = Event::new(
        SessionId::new(),
        EventContext::empty(),
        EventData::LlmGeneration(data.clone()),
    );
    let exported = listener.convert_llm_generation(&event, &data);
    assert_eq!(exported.metadata["actual_cost_usd"], 0.0);
    assert_eq!(exported.metadata["estimated_cost_usd"], 0.25);
    assert_eq!(exported.metadata["effective_cost_usd"], 0.1);
}

#[tokio::test]
async fn exec_only_thinking_events_merge_and_link_to_the_actual_generation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/project_logs/test-project-id/insert"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let mut config = test_config();
    config.api_url = server.uri();
    let listener = BraintrustListener::new(config).unwrap();
    let turn_id = TurnId::new();
    let session = SessionId::new();
    let mut context = EventContext::turn(turn_id, MessageId::new());
    context.exec_id = Some(everruns_contracts::typed_id::ExecId::new());
    let started = Event::new(
        session,
        context.clone(),
        EventData::ReasonThinkingStarted(ReasonThinkingStartedData {
            turn_id,
            model: Some("model".into()),
        }),
    );
    let completed = Event::new(
        session,
        context.clone(),
        EventData::ReasonThinkingCompleted(ReasonThinkingCompletedData {
            turn_id,
            thinking: PRIVATE.into(),
        }),
    );
    listener.on_event(&started).await;
    listener.on_event(&completed).await;
    context.span_id = Some("generation-span".into());
    context.parent_span_id = Some("reason-span".into());
    let generation = Event::new(
        session,
        context,
        EventData::LlmGeneration(LlmGenerationData {
            messages: vec![],
            tools: vec![],
            request_body: None,
            output: LlmGenerationOutput {
                text: None,
                tool_calls: vec![],
            },
            metadata: bare_generation_metadata("model"),
        }),
    );
    listener.on_event(&generation).await;
    listener.flush().await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let rows: Vec<serde_json::Value> = requests
        .iter()
        .flat_map(|r| {
            serde_json::from_slice::<serde_json::Value>(&r.body).unwrap()["events"]
                .as_array()
                .unwrap()
                .clone()
        })
        .collect();
    let thinking: Vec<_> = rows
        .iter()
        .filter(|r| r["span_attributes"]["name"] == "thinking")
        .collect();
    assert_eq!(thinking.len(), 3); // start, finish, later generation-parent merge
    assert_eq!(thinking[0]["id"], thinking[1]["id"]);
    assert_eq!(thinking[1]["id"], thinking[2]["id"]);
    assert_eq!(thinking[2]["span_parents"], json!(["generation-span"]));
    assert_eq!(thinking[2]["_is_merge"], true);
    assert!(!serde_json::to_string(&rows).unwrap().contains(PRIVATE));
}

#[tokio::test]
async fn dropping_listener_drains_queued_events_and_releases_secret_state() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/project_logs/test-project-id/insert"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let mut config = test_config();
    config.api_url = server.uri();
    config.delivery.flush_interval = Duration::from_secs(60);
    let listener = BraintrustListener::new(config).unwrap();
    let state = Arc::downgrade(&listener.state);
    listener.on_event(&turn_failure().0).await;
    drop(listener);
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("closed listener must release its delivery worker and credentials");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[test]
fn config_debug_never_contains_api_key() {
    let mut config = test_config();
    config.api_key = PRIVATE.into();
    assert!(!format!("{config:?}").contains(PRIVATE));
}

#[test]
fn act_summaries_do_not_bypass_tool_content_redaction() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let start: ActStartedData = serde_json::from_value(json!({
        "tool_calls": [{"id":"call", "name":"read_file", "display_name":PRIVATE, "narration":PRIVATE}],
        "headline":PRIVATE
    })).unwrap();
    let event = Event::new(
        SessionId::new(),
        EventContext::empty(),
        EventData::ActStarted(start.clone()),
    );
    let exported = listener.convert_act_started(&event, &start);
    assert!(!serde_json::to_string(&exported).unwrap().contains(PRIVATE));
    assert_eq!(
        exported.input.unwrap()["tool_calls"][0]["name"],
        "read_file"
    );
    let complete: ActCompletedData = serde_json::from_value(json!({
        "completed":true,"success_count":1,"error_count":0,"headline":PRIVATE
    }))
    .unwrap();
    let exported = listener.convert_act_completed(&event, &complete);
    assert!(!serde_json::to_string(&exported).unwrap().contains(PRIVATE));
    assert!(exported.metrics.unwrap().end.is_some());
}
