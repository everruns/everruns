use super::*;
use everruns_contracts::reasoning::{ReasoningContentPart, ReasoningText};
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{SessionId, TurnId};
use everruns_core::events::{
    EventContext, InputMessageData, ModelMetadata, OutputMessageCompletedData, ReasonCompletedData,
    ToolCompletedData, TurnCompletedData, TurnFailedData,
};

fn event(session: SessionId, data: impl Into<EventData>) -> Event {
    Event::new(session, EventContext::empty(), data)
}

/// Synthetic session: user msg → iteration 1 (tool call + result) →
/// iteration 2 (final text) → turn.completed.
fn sample_events(session: SessionId) -> Vec<Event> {
    let turn_id = TurnId::new();
    let tool_call = ToolCall {
        id: "call_1".to_string(),
        name: "search".to_string(),
        arguments: json!({"query": "weather"}),
    };

    let mut first = RuntimeMessage::assistant_with_tools("checking", vec![tool_call]);
    first.content.insert(
        0,
        ContentPart::Reasoning(
            ReasoningContentPart::opaque("anthropic")
                .with_signature("sig")
                .with_text(ReasoningText::Plain {
                    text: "let me look this up".to_string(),
                }),
        ),
    );

    vec![
        event(
            session,
            InputMessageData::new(RuntimeMessage::user("hi there")),
        ),
        event(
            session,
            OutputMessageCompletedData::new(first)
                .with_metadata(ModelMetadata {
                    model: "test-model".to_string(),
                    model_id: None,
                    provider_id: None,
                })
                .with_usage(TokenUsage::with_cache(100, 20, Some(30), None)),
        ),
        event(
            session,
            ReasonCompletedData::success(
                "checking",
                true,
                1,
                Some(250),
                Some(TokenUsage::new(100, 20)),
            ),
        ),
        event(
            session,
            ToolCompletedData::success(
                "call_1".to_string(),
                "search".to_string(),
                vec![ContentPart::text("sunny, 21C")],
                Some(40),
            ),
        ),
        event(
            session,
            OutputMessageCompletedData::new(RuntimeMessage::assistant(
                "It is sunny. token sk-abcdef0123456789ABCDEF",
            ))
            .with_usage(TokenUsage::new(150, 10)),
        ),
        event(
            session,
            ReasonCompletedData::success("It is sunny.", false, 0, Some(120), None),
        ),
        event(
            session,
            TurnCompletedData {
                termination_reason: Some(everruns_contracts::turn::TurnStopReason::EndTurn),
                turn_id,
                iterations: 2,
                duration_ms: Some(1500),
                usage: None,
                input_content: None,
                final_message_id: None,
                final_answer_preview: None,
                time_to_first_token_ms: None,
                tool_call_count: Some(1),
                llm_call_count: Some(2),
                ..Default::default()
            },
        ),
    ]
}

#[test]
fn folds_events_into_atif_steps() {
    let session = SessionId::new();
    let built = build_trajectory(
        Some("session_x"),
        &sample_events(session),
        Map::new(),
        AtifOptions::default(),
    );
    let value = built.document;

    assert_eq!(value["schema_version"], json!(ATIF_SCHEMA_VERSION));
    assert_eq!(value["session_id"], json!("session_x"));
    assert_eq!(value["agent"]["name"], json!("everruns"));
    assert_eq!(value["agent"]["model_name"], json!("test-model"));

    let steps = value["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 3);

    // Step ids are 1-based and sequential.
    for (i, step) in steps.iter().enumerate() {
        assert_eq!(step["step_id"], json!(i + 1));
    }

    assert_eq!(steps[0]["source"], json!("user"));
    assert_eq!(steps[0]["message"], json!("hi there"));

    // Iteration 1: tool call, reasoning, observation, metrics.
    let s1 = &steps[1];
    assert_eq!(s1["source"], json!("agent"));
    assert_eq!(s1["message"], json!("checking"));
    assert_eq!(s1["reasoning_content"], json!("let me look this up"));
    assert_eq!(s1["model_name"], json!("test-model"));
    assert_eq!(s1["tool_calls"][0]["tool_call_id"], json!("call_1"));
    assert_eq!(s1["tool_calls"][0]["function_name"], json!("search"));
    assert_eq!(
        s1["tool_calls"][0]["arguments"],
        json!({"query": "weather"})
    );
    assert_eq!(
        s1["observation"]["results"][0]["source_call_id"],
        json!("call_1")
    );
    assert_eq!(
        s1["observation"]["results"][0]["content"],
        json!("sunny, 21C")
    );
    // prompt = input(100) + cache_read(30); cached = 30.
    assert_eq!(s1["metrics"]["prompt_tokens"], json!(130));
    assert_eq!(s1["metrics"]["completion_tokens"], json!(20));
    assert_eq!(s1["metrics"]["cached_tokens"], json!(30));
    assert_eq!(s1["extra"]["reason_duration_ms"], json!(250));

    // Iteration 2: final message, secret scrubbed.
    let s2 = &steps[2];
    assert_eq!(s2["source"], json!("agent"));
    let msg = s2["message"].as_str().unwrap();
    assert!(msg.contains("It is sunny."));
    assert!(!msg.contains("sk-abcdef0123456789ABCDEF"));
    assert!(msg.contains(REDACTED));
    assert!(s2.get("tool_calls").is_none());
    assert!(s2.get("observation").is_none());

    // Turn boundary recorded in extra, not as a step.
    assert_eq!(value["extra"]["turns"][0]["status"], json!("completed"));
    assert_eq!(value["extra"]["turns"][0]["iterations"], json!(2));
    assert_eq!(value["extra"]["turns"][0]["duration_ms"], json!(1500));

    // Aggregates: 130 + 150 prompt, 20 + 10 completion.
    assert_eq!(value["final_metrics"]["total_prompt_tokens"], json!(280));
    assert_eq!(value["final_metrics"]["total_completion_tokens"], json!(30));
    assert_eq!(value["final_metrics"]["total_steps"], json!(3));
}

#[test]
fn failed_reason_and_turn_produce_error_step_and_turn_record() {
    let session = SessionId::new();
    let turn_id = TurnId::new();
    let events = vec![
        event(session, InputMessageData::new(RuntimeMessage::user("hi"))),
        event(
            session,
            ReasonCompletedData::failure("provider exploded".to_string(), Some(50)),
        ),
        event(
            session,
            TurnFailedData {
                turn_id,
                error: "provider exploded".to_string(),
                error_code: None,
                error_fields: None,
                error_disclosure: None,
            },
        ),
    ];
    let value = build_trajectory(None, &events, Map::new(), AtifOptions::default()).document;
    let steps = value["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[1]["source"], json!("agent"));
    assert_eq!(steps[1]["extra"]["error"], json!("provider exploded"));
    assert_eq!(value["extra"]["turns"][0]["status"], json!("failed"));
    assert_eq!(
        value["extra"]["turns"][0]["error"],
        json!("provider exploded")
    );
}

#[test]
fn redact_content_blanks_content_but_keeps_structure() {
    let session = SessionId::new();
    let value = build_trajectory(
        Some("session_x"),
        &sample_events(session),
        Map::new(),
        AtifOptions {
            redact_content: true,
        },
    )
    .document;
    let serialized = serde_json::to_string(&value).unwrap();
    assert!(!serialized.contains("hi there"));
    assert!(!serialized.contains("sunny"));
    assert!(!serialized.contains("weather"));
    assert!(!serialized.contains("let me look this up"));
    // Structure preserved.
    let steps = value["steps"].as_array().unwrap();
    assert_eq!(steps[1]["tool_calls"][0]["function_name"], json!("search"));
    assert_eq!(
        steps[1]["observation"]["results"][0]["source_call_id"],
        json!("call_1")
    );
    assert_eq!(steps[0]["message"], json!(REDACTED));
}

#[test]
fn images_export_as_multimodal_content_parts() {
    use everruns_contracts::typed_id::ImageId;
    use everruns_core::message::{ImageContentPart, ImageFileContentPart};

    let session = SessionId::new();
    let image_id = ImageId::new();
    let mut user = RuntimeMessage::user("look at this");
    user.content
        .push(ContentPart::Image(ImageContentPart::from_url(
            "https://example.com/cat.png",
        )));
    user.content
        .push(ContentPart::ImageFile(ImageFileContentPart::with_filename(
            image_id, "cat.png",
        )));

    let tool_call = ToolCall {
        id: "call_img".to_string(),
        name: "screenshot".to_string(),
        arguments: json!({}),
    };
    let events = vec![
        event(session, InputMessageData::new(user)),
        event(
            session,
            OutputMessageCompletedData::new(RuntimeMessage::assistant_with_tools(
                "taking a screenshot",
                vec![tool_call],
            )),
        ),
        event(
            session,
            ToolCompletedData::success(
                "call_img".to_string(),
                "screenshot".to_string(),
                vec![
                    ContentPart::text("done"),
                    ContentPart::Image(ImageContentPart::from_base64("aGVsbG8=", "image/png")),
                ],
                Some(5),
            ),
        ),
    ];

    let built = build_trajectory(
        Some("session_x"),
        &events,
        Map::new(),
        AtifOptions::default(),
    );
    // Every image is materialized, so nothing is omitted.
    assert_eq!(built.images_omitted, 0);
    let value = built.document;
    assert!(value["extra"].get("images_omitted").is_none());

    // User step: message is a ContentPart array (text + URL image + file
    // image source referencing the file-serving route).
    let steps = value["steps"].as_array().unwrap();
    let user_msg = steps[0]["message"].as_array().unwrap();
    assert_eq!(user_msg[0], json!({"type": "text", "text": "look at this"}));
    assert_eq!(
        user_msg[1],
        json!({"type": "image", "source": {"path": "https://example.com/cat.png"}})
    );
    assert_eq!(
        user_msg[2],
        json!({"type": "image", "source": {"path": format!("/v1/images/{image_id}")}})
    );
    assert!(steps[0]["extra"].get("omitted_images").is_none());

    // Observation content is a ContentPart array; the base64 image inlines
    // as a data: URI source with its media type.
    let s1 = &steps[1];
    let obs = s1["observation"]["results"][0]["content"]
        .as_array()
        .unwrap();
    assert_eq!(obs[0], json!({"type": "text", "text": "done"}));
    assert_eq!(
        obs[1],
        json!({"type": "image", "source": {"media_type": "image/png", "path": "data:image/png;base64,aGVsbG8="}})
    );
    assert!(s1["extra"].get("omitted_images").is_none());
}

#[test]
fn unmaterializable_image_is_still_omitted_and_counted() {
    use everruns_core::message::ImageContentPart;

    let session = SessionId::new();
    let mut user = RuntimeMessage::user("no locator here");
    // An inline image with neither URL nor base64 cannot be materialized.
    user.content.push(ContentPart::Image(ImageContentPart {
        url: None,
        base64: None,
        media_type: Some("image/png".to_string()),
    }));
    let events = vec![event(session, InputMessageData::new(user))];

    let built = build_trajectory(
        Some("session_x"),
        &events,
        Map::new(),
        AtifOptions::default(),
    );
    assert_eq!(built.images_omitted, 1);
    let value = built.document;
    assert_eq!(value["extra"]["images_omitted"], json!(1));
    // Text-only (no materialized image) → the message stays a string with
    // the `[image]` marker, and the locator lands in step extra.
    let steps = value["steps"].as_array().unwrap();
    assert_eq!(steps[0]["message"], json!("no locator here\n[image]"));
    assert_eq!(
        steps[0]["extra"]["omitted_images"],
        json!([{"media_type": "image/png"}])
    );
}

#[test]
fn redaction_blanks_image_sources() {
    use everruns_contracts::typed_id::ImageId;
    use everruns_core::message::{ImageContentPart, ImageFileContentPart};

    let session = SessionId::new();
    let image_id = ImageId::new();
    let mut user = RuntimeMessage::user("secret url");
    user.content
        .push(ContentPart::Image(ImageContentPart::from_url(
            "https://example.com/private.png",
        )));
    user.content
        .push(ContentPart::ImageFile(ImageFileContentPart::new(image_id)));
    let events = vec![event(session, InputMessageData::new(user))];

    let value = build_trajectory(
        Some("session_x"),
        &events,
        Map::new(),
        AtifOptions {
            redact_content: true,
        },
    )
    .document;
    let msg = value["steps"][0]["message"].as_array().unwrap();
    // Text is redacted, image sources are blanked (path), structure kept.
    assert_eq!(msg[0], json!({"type": "text", "text": REDACTED}));
    assert_eq!(msg[1]["source"]["path"], json!(REDACTED));
    assert_eq!(msg[2]["source"]["path"], json!(REDACTED));
    let serialized = serde_json::to_string(&value).unwrap();
    assert!(!serialized.contains("example.com"));
}

#[test]
fn segments_byte_bound_with_inline_image_parts() {
    use everruns_core::message::ImageContentPart;

    let session = SessionId::new();
    let session_id = "session_x";
    // Two messages each carrying a sizeable inlined base64 image, so the
    // per-step serialized size is dominated by the image ContentPart.
    let big_b64 = "A".repeat(2000);
    let mut m0 = RuntimeMessage::user("first");
    m0.content
        .push(ContentPart::Image(ImageContentPart::from_base64(
            big_b64.clone(),
            "image/png",
        )));
    let mut m1 = RuntimeMessage::user("second");
    m1.content
        .push(ContentPart::Image(ImageContentPart::from_base64(
            big_b64,
            "image/png",
        )));
    let events = vec![
        event(session, InputMessageData::new(m0)),
        event(session, InputMessageData::new(m1)),
    ];

    // A cap just above one image step forces one step per segment, proving
    // image ContentParts count toward the byte budget.
    let (walked, docs) = walk_segments(session_id, &events, 4096 + 1500);
    assert_eq!(
        docs.len(),
        2,
        "each oversized image step gets its own segment"
    );
    assert_eq!(walked.len(), 2);
    for (i, step) in walked.iter().enumerate() {
        assert_eq!(step["step_id"], json!(i + 1));
        assert!(step["message"].is_array(), "image step message is an array");
    }
}

#[test]
fn subagent_spawn_attaches_trajectory_ref() {
    let session = SessionId::new();
    let child = SessionId::new();
    let spawn_call = ToolCall {
        id: "call_spawn".to_string(),
        name: "spawn_agent".to_string(),
        arguments: json!({"target": {"type": "subagent"}, "name": "Worker"}),
    };
    // The spawn tool result is a single text ContentPart holding the JSON
    // object the dispatcher returns (carries `subagent_id`).
    let spawn_result = json!({
        "subagent_id": child.to_string(),
        "name": "Worker",
        "status": "running",
        "task_id": "task_123",
    })
    .to_string();
    let events = vec![
        event(
            session,
            InputMessageData::new(RuntimeMessage::user("delegate this")),
        ),
        event(
            session,
            OutputMessageCompletedData::new(RuntimeMessage::assistant_with_tools(
                "spawning",
                vec![spawn_call],
            )),
        ),
        event(
            session,
            ToolCompletedData::success(
                "call_spawn".to_string(),
                "spawn_agent".to_string(),
                vec![ContentPart::text(spawn_result)],
                Some(5),
            ),
        ),
    ];

    let value = build_trajectory(
        Some("session_x"),
        &events,
        Map::new(),
        AtifOptions::default(),
    )
    .document;
    let refs = &value["steps"][1]["observation"]["results"][0]["subagent_trajectory_ref"];
    assert_eq!(
        refs[0]["trajectory_path"],
        json!(format!("/v1/sessions/{child}/export?format=atif"))
    );
    assert_eq!(refs[0]["session_id"], json!(child.to_string()));
}

#[test]
fn non_spawn_tool_has_no_subagent_ref() {
    let session = SessionId::new();
    let value = build_trajectory(
        Some("session_x"),
        &sample_events(session),
        Map::new(),
        AtifOptions::default(),
    )
    .document;
    // The sample session's tool observation is a plain search, not a spawn.
    assert!(
        value["steps"][1]["observation"]["results"][0]
            .get("subagent_trajectory_ref")
            .is_none()
    );
}

#[test]
fn zero_image_session_carries_no_omission_keys() {
    let session = SessionId::new();
    let built = build_trajectory(
        Some("session_x"),
        &sample_events(session),
        Map::new(),
        AtifOptions::default(),
    );
    assert_eq!(built.images_omitted, 0);
    let serialized = serde_json::to_string(&built.document).unwrap();
    assert!(!serialized.contains("omitted_images"));
    assert!(!serialized.contains("images_omitted"));
}

#[test]
fn case_record_carries_reward_and_identity_in_extra() {
    use crate::domains::evals::record::{
        CaseResultStatus, EvalCaseResult, EvalRun, EvalRunSource, EvalRunStatus,
    };
    use everruns_contracts::typed_id::{EvalCaseId, EvalResultId, EvalRunId};

    let session = SessionId::new();
    let run = EvalRun {
        public_id: EvalRunId::new(),
        internal_id: uuid::Uuid::nil(),
        org_id: 1,
        target: None,
        model_override: None,
        filter_tags: None,
        status: EvalRunStatus::Completed,
        source: EvalRunSource::Internal,
        attribution: None,
        triggered_by: "test".to_string(),
        trials: 1,
        started_at: None,
        completed_at: None,
        summary: None,
        results: vec![],
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let result = EvalCaseResult {
        public_id: EvalResultId::new(),
        internal_id: uuid::Uuid::nil(),
        eval_case_id: EvalCaseId::new(),
        case_name: Some("case-one".to_string()),
        session_id: Some(session),
        target: None,
        target_snapshot: None,
        status: CaseResultStatus::Passed,
        trial: 0,
        scores: Some(json!([{"pass": true, "value": 1.0, "reason": "ok"}])),
        metadata: None,
        turns: Some(1),
        latency_ms: Some(10),
        input_tokens: Some(1),
        output_tokens: Some(2),
        error_message: None,
        artifacts: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let record = build_case_record(
        &run,
        &result,
        &sample_events(session),
        AtifOptions::default(),
    );
    assert_eq!(record["schema_version"], json!(ATIF_SCHEMA_VERSION));
    assert_eq!(record["extra"]["reward"]["pass"], json!(true));
    assert_eq!(record["extra"]["reward"]["score"], json!(1.0));
    assert_eq!(record["extra"]["case_name"], json!("case-one"));
    assert!(
        record["extra"]["source_key"]
            .as_str()
            .unwrap()
            .contains('/')
    );
    assert_eq!(record["session_id"], json!(session.to_string()));
}

// ------------------------------------------------------------------
// Model-view (dataset) message fold
// ------------------------------------------------------------------

/// A minimal passing run/result pair for the dataset record builders.
fn sample_run_and_result(session: SessionId) -> (EvalRun, EvalCaseResult) {
    use crate::domains::evals::record::{
        CaseResultStatus, EvalCaseResult, EvalRun, EvalRunSource, EvalRunStatus,
    };
    use everruns_contracts::typed_id::{EvalCaseId, EvalResultId, EvalRunId};

    let run = EvalRun {
        public_id: EvalRunId::new(),
        internal_id: uuid::Uuid::nil(),
        org_id: 1,
        target: None,
        model_override: None,
        filter_tags: None,
        status: EvalRunStatus::Completed,
        source: EvalRunSource::Internal,
        attribution: None,
        triggered_by: "test".to_string(),
        trials: 1,
        started_at: None,
        completed_at: None,
        summary: None,
        results: vec![],
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let result = EvalCaseResult {
        public_id: EvalResultId::new(),
        internal_id: uuid::Uuid::nil(),
        eval_case_id: EvalCaseId::new(),
        case_name: Some("case-one".to_string()),
        session_id: Some(session),
        target: None,
        target_snapshot: None,
        status: CaseResultStatus::Passed,
        trial: 0,
        scores: Some(json!([{"pass": true, "value": 1.0, "reason": "ok"}])),
        metadata: None,
        turns: Some(1),
        latency_ms: Some(10),
        input_tokens: Some(11),
        output_tokens: Some(22),
        error_message: None,
        artifacts: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    (run, result)
}

#[test]
fn message_fold_maps_roles_to_steps() {
    let session = SessionId::new();
    let (run, result) = sample_run_and_result(session);

    let tool_call = ToolCall {
        id: "call_1".to_string(),
        name: "search".to_string(),
        arguments: json!({"q": "weather"}),
    };
    let mut agent = RuntimeMessage::assistant_with_tools("checking", vec![tool_call]);
    agent.content.insert(
        0,
        ContentPart::Reasoning(
            ReasoningContentPart::opaque("anthropic")
                .with_signature("sig")
                .with_text(ReasoningText::Plain {
                    text: "let me look".to_string(),
                }),
        ),
    );
    let messages = vec![
        RuntimeMessage::user("hi there"),
        agent,
        RuntimeMessage::tool_result("call_1", Some(json!("sunny, 21C")), None),
        RuntimeMessage::assistant("It is sunny."),
    ];

    let doc = build_case_record_from_messages(&run, &result, &messages, AtifOptions::default());
    // No per-step metrics or turns roll-up on the model-view fold.
    assert!(doc["extra"].get("turns").is_none());
    // Case-level token totals surface in final_metrics.
    assert_eq!(doc["final_metrics"]["total_prompt_tokens"], json!(11));
    assert_eq!(doc["final_metrics"]["total_completion_tokens"], json!(22));

    let steps = doc["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 3, "user, agent(tool+obs), agent(final)");
    assert_eq!(steps[0]["source"], json!("user"));
    assert_eq!(steps[0]["message"], json!("hi there"));
    assert_eq!(steps[0]["step_id"], json!(1));

    let s1 = &steps[1];
    assert_eq!(s1["source"], json!("agent"));
    assert_eq!(s1["message"], json!("checking"));
    assert_eq!(s1["reasoning_content"], json!("let me look"));
    assert_eq!(s1["tool_calls"][0]["tool_call_id"], json!("call_1"));
    assert_eq!(s1["tool_calls"][0]["function_name"], json!("search"));
    assert_eq!(
        s1["observation"]["results"][0]["source_call_id"],
        json!("call_1")
    );
    assert_eq!(
        s1["observation"]["results"][0]["content"],
        json!("sunny, 21C")
    );
    assert!(
        s1.get("metrics").is_none(),
        "messages carry no per-step usage"
    );

    assert_eq!(steps[2]["message"], json!("It is sunny."));
    assert_eq!(doc["extra"]["reward"]["pass"], json!(true));
}

#[test]
fn message_fold_reflects_model_view_masking() {
    use everruns_core::builtins::{RuntimeCompactionConfig, build_model_view_messages};

    let session = SessionId::new();
    let (run, result) = sample_run_and_result(session);

    // Ten padded (> 4 KiB) tool results. Default cost-control masking
    // keeps the two most recent and masks the eight older ones, one batch.
    let mut messages = vec![RuntimeMessage::user("start")];
    for i in 0..10 {
        let call = ToolCall {
            id: format!("call_{i}"),
            name: "fetch".to_string(),
            arguments: json!({"n": i}),
        };
        messages.push(RuntimeMessage::assistant_with_tools(
            format!("iter {i}"),
            vec![call],
        ));
        messages.push(RuntimeMessage::tool_result(
            format!("call_{i}"),
            Some(json!(format!("BULKY_RESULT_{i} {}", "x".repeat(5 * 1024)))),
            None,
        ));
    }

    // Folding the raw (unmasked) messages keeps every distinctive result.
    let raw = build_case_record_from_messages(&run, &result, &messages, AtifOptions::default());
    let raw_str = serde_json::to_string(&raw).unwrap();
    for i in 0..10 {
        assert!(
            raw_str.contains(&format!("BULKY_RESULT_{i}")),
            "unmasked fold keeps result {i}",
        );
    }

    // Folding the model view drops the masked (older) tool-result content —
    // exactly the training-faithfulness the dataset export now guarantees.
    let masked_messages =
        build_model_view_messages(&messages, &RuntimeCompactionConfig::default(), None).messages;
    let masked =
        build_case_record_from_messages(&run, &result, &masked_messages, AtifOptions::default());
    let masked_str = serde_json::to_string(&masked).unwrap();
    for i in 0..8 {
        assert!(
            !masked_str.contains(&format!("BULKY_RESULT_{i}")),
            "masked result {i} must be absent from the model-view fold",
        );
    }
    for i in 8..10 {
        assert!(
            masked_str.contains(&format!("BULKY_RESULT_{i}")),
            "recent result {i} must survive in the model-view fold",
        );
    }
    assert!(
        masked_str.contains("masked"),
        "masked results carry the compaction summary marker",
    );
}

#[test]
fn message_fold_redacts_and_links_subagents() {
    let session = SessionId::new();
    let (run, result) = sample_run_and_result(session);
    let child = SessionId::new();

    let spawn_call = ToolCall {
        id: "call_spawn".to_string(),
        name: "spawn_agent".to_string(),
        arguments: json!({"target": {"type": "subagent"}}),
    };
    let messages = vec![
        RuntimeMessage::user("delegate secret sk-abcdef0123456789ABCDEF"),
        RuntimeMessage::assistant_with_tools("spawning", vec![spawn_call]),
        RuntimeMessage::tool_result(
            "call_spawn",
            Some(json!({"subagent_id": child.to_string(), "status": "running"})),
            None,
        ),
    ];

    // Subagent linkage parity with the event fold.
    let doc = build_case_record_from_messages(&run, &result, &messages, AtifOptions::default());
    let refs = &doc["steps"][1]["observation"]["results"][0]["subagent_trajectory_ref"];
    assert_eq!(
        refs[0]["trajectory_path"],
        json!(format!("/v1/sessions/{child}/export?format=atif"))
    );
    // Secret scrubbing applies on the model-view path too.
    assert!(
        !serde_json::to_string(&doc)
            .unwrap()
            .contains("sk-abcdef0123456789ABCDEF")
    );

    // Redaction blanks content but preserves structure.
    let redacted = build_case_record_from_messages(
        &run,
        &result,
        &messages,
        AtifOptions {
            redact_content: true,
        },
    );
    assert_eq!(redacted["steps"][0]["message"], json!(REDACTED));
    assert_eq!(
        redacted["steps"][1]["tool_calls"][0]["function_name"],
        json!("spawn_agent")
    );
}

#[test]
fn message_fold_does_not_link_non_spawn_subagent_ids() {
    let session = SessionId::new();
    let (run, result) = sample_run_and_result(session);
    let fake_child = "patient-alice@example.internal";

    let lookup_call = ToolCall {
        id: "call_lookup".to_string(),
        name: "lookup_patient".to_string(),
        arguments: json!({"patient": "alice"}),
    };
    let messages = vec![
        RuntimeMessage::user("lookup alice"),
        RuntimeMessage::assistant_with_tools("checking", vec![lookup_call]),
        RuntimeMessage::tool_result(
            "call_lookup",
            Some(json!({"subagent_id": fake_child, "status": "matched"})),
            None,
        ),
    ];

    let redacted = build_case_record_from_messages(
        &run,
        &result,
        &messages,
        AtifOptions {
            redact_content: true,
        },
    );
    let observation = &redacted["steps"][1]["observation"]["results"][0];
    assert_eq!(observation["content"], json!(REDACTED));
    assert!(
        observation.get("subagent_trajectory_ref").is_none(),
        "non-spawn tool result subagent_id stays redacted content, not a structural link"
    );
    assert!(
        !serde_json::to_string(&redacted)
            .unwrap()
            .contains(fake_child),
        "redacted ATIF output must not leak non-spawn subagent_id content"
    );
}

// ------------------------------------------------------------------
// Import parsing
// ------------------------------------------------------------------

fn sample_trajectory() -> Value {
    json!({
        "schema_version": "ATIF-v1.7",
        "session_id": "session_abc",
        "agent": {"name": "yolop", "version": "0.3.0"},
        "steps": [
            {"step_id": 1, "source": "user", "message": "What is 2+2?"},
            {"step_id": 2, "source": "agent", "message": "4"},
        ],
        "extra": {"reward": {"pass": true, "score": 1.0}},
        "unknown_future_field": {"ignored": true},
    })
}

#[test]
fn parse_accepts_array_object_and_ndjson() {
    let t = sample_trajectory();

    let array_body = serde_json::to_string(&json!([t])).unwrap();
    assert_eq!(parse_import_body(&array_body).unwrap().len(), 1);

    let single_body = serde_json::to_string(&t).unwrap();
    assert_eq!(parse_import_body(&single_body).unwrap().len(), 1);

    let wrapped = serde_json::to_string(&json!({"trajectories": [t, t]})).unwrap();
    assert_eq!(parse_import_body(&wrapped).unwrap().len(), 2);

    let ndjson = format!(
        "{}\n{}\n",
        serde_json::to_string(&t).unwrap(),
        serde_json::to_string(&t).unwrap()
    );
    assert_eq!(parse_import_body(&ndjson).unwrap().len(), 2);
}

#[test]
fn parse_rejects_empty_and_oversized() {
    assert!(parse_import_body("[]").is_err());
    assert!(parse_import_body("not json at all").is_err());
    let big = "x".repeat(MAX_IMPORT_BYTES + 1);
    assert!(parse_import_body(&big).is_err());
}

#[test]
fn draft_maps_user_steps_and_reference() {
    let draft = trajectory_to_case_draft(&sample_trajectory(), 0).unwrap();
    assert_eq!(draft.name, "session_abc");
    assert_eq!(draft.conversation.len(), 1);
    assert_eq!(draft.conversation[0].content, "What is 2+2?");
    assert!(draft.description.contains("Reference final agent message"));
    assert!(draft.description.contains('4'));
    assert!(draft.description.contains("yolop"));
    assert!(draft.description.contains("reward"));
}

#[test]
fn draft_prefers_extra_case_name_and_tolerates_content_parts() {
    let t = json!({
        "schema_version": "ATIF-v1.9",
        "agent": {"name": "x", "version": "1"},
        "steps": [
            {"step_id": 1, "source": "user",
             "message": [{"type": "text", "text": "part one"}, {"type": "text", "text": "part two"}]},
        ],
        "extra": {"case_name": "named-case"},
    });
    let draft = trajectory_to_case_draft(&t, 3).unwrap();
    assert_eq!(draft.name, "named-case");
    assert_eq!(draft.conversation[0].content, "part one\npart two");
}

#[test]
fn draft_rejects_missing_version_and_missing_user_steps() {
    assert!(trajectory_to_case_draft(&json!({"steps": []}), 0).is_err());
    let no_user = json!({
        "schema_version": "ATIF-v1.7",
        "steps": [{"step_id": 1, "source": "agent", "message": "hello"}],
    });
    assert!(trajectory_to_case_draft(&no_user, 0).is_err());
    let bad_version = json!({
        "schema_version": "OTHER-v1",
        "steps": [{"step_id": 1, "source": "user", "message": "hi"}],
    });
    assert!(trajectory_to_case_draft(&bad_version, 0).is_err());
}

#[test]
fn draft_falls_back_to_indexed_name() {
    let t = json!({
        "schema_version": "ATIF-v1.7",
        "steps": [{"step_id": 1, "source": "user", "message": "hi"}],
    });
    let draft = trajectory_to_case_draft(&t, 4).unwrap();
    assert_eq!(draft.name, "atif-case-5");
}

// ------------------------------------------------------------------
// Segmented export
// ------------------------------------------------------------------

const LINK_BASE: &str = "/v1/sessions/session_x/export";

/// A session with `n` distinct user messages → `n` user steps.
fn many_input_events(session: SessionId, n: usize) -> Vec<Event> {
    (0..n)
        .map(|i| {
            event(
                session,
                InputMessageData::new(RuntimeMessage::user(format!("message number {i}"))),
            )
        })
        .collect()
}

/// Walk the whole segmented chain, concatenating every segment's steps.
/// Returns (all_steps_in_order, segment_docs).
fn walk_segments(session_id: &str, events: &[Event], max_bytes: usize) -> (Vec<Value>, Vec<Value>) {
    let mut steps = Vec::new();
    let mut docs = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let seg = build_segment(
            session_id,
            events,
            AtifOptions::default(),
            cursor.as_deref(),
            max_bytes,
            LINK_BASE,
        )
        .expect("segment builds");
        steps.extend(seg.document["steps"].as_array().unwrap().iter().cloned());
        docs.push(seg.document.clone());
        match seg.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
        // Guard against an accidental infinite loop in the test itself.
        assert!(docs.len() < 10_000, "segment walk did not terminate");
    }
    (steps, docs)
}

#[test]
fn segments_split_link_and_reconstruct_whole_document() {
    let session = SessionId::new();
    let session_id = "session_x";
    let events = many_input_events(session, 10);

    // Tiny cap (below the scaffold reserve) → one step per segment, so the
    // chain has as many segments as steps: exercises linking end to end.
    let (walked, docs) = walk_segments(session_id, &events, 256);

    assert!(docs.len() >= 2, "expected multiple segments");
    assert_eq!(docs.len(), 10, "tiny cap packs one step per segment");

    // Every non-final segment links forward; the final one does not.
    for (i, doc) in docs.iter().enumerate() {
        assert_eq!(doc["schema_version"], json!(ATIF_SCHEMA_VERSION));
        assert_eq!(doc["session_id"], json!(session_id));
        assert_eq!(
            doc["trajectory_id"],
            json!(format!("{session_id}#segment-{i}"))
        );
        assert_eq!(doc["extra"]["segment_index"], json!(i));
        let is_final = i == docs.len() - 1;
        assert_eq!(
            doc.get("continued_trajectory_ref").is_some(),
            !is_final,
            "continuation present iff not the final segment (segment {i})",
        );
    }
    // Final segment carries the (empty here) session turn roll-up slot only
    // and no continuation.
    assert!(
        docs.last()
            .unwrap()
            .get("continued_trajectory_ref")
            .is_none()
    );

    // The stitched steps equal the whole-document export's steps, with
    // absolute, sequential step_ids preserved.
    let whole = build_trajectory(
        Some(session_id),
        &events,
        Map::new(),
        AtifOptions::default(),
    )
    .document;
    assert_eq!(&walked, whole["steps"].as_array().unwrap());
    for (i, step) in walked.iter().enumerate() {
        assert_eq!(step["step_id"], json!(i + 1));
    }

    // The continuation ref embeds the segmented flag and a cursor param.
    let first_ref = docs[0]["continued_trajectory_ref"].as_str().unwrap();
    assert!(first_ref.starts_with(LINK_BASE));
    assert!(first_ref.contains("segmented=true"));
    assert!(first_ref.contains("cursor="));
}

#[test]
fn segments_pack_multiple_steps_when_budget_allows() {
    let session = SessionId::new();
    let session_id = "session_x";
    let events = many_input_events(session, 12);
    // Above the scaffold reserve → each segment packs several steps, and
    // there is still more than one segment.
    let (walked, docs) = walk_segments(session_id, &events, 4096 + 400);
    assert!(docs.len() >= 2, "expected multiple segments");
    assert!(
        docs[0]["steps"].as_array().unwrap().len() > 1,
        "first segment should pack more than one step under a larger cap",
    );
    assert_eq!(walked.len(), 12);
    for (i, step) in walked.iter().enumerate() {
        assert_eq!(step["step_id"], json!(i + 1));
    }
}

#[test]
fn cursor_round_trips_and_binds_session() {
    let cursor = SegmentCursor {
        session_id: "session_abc".to_string(),
        offset: 7,
    };
    let encoded = cursor.encode();
    assert_eq!(SegmentCursor::decode(&encoded, "session_abc").unwrap(), 7);
    // Bound to the session it was minted for.
    assert!(SegmentCursor::decode(&encoded, "session_other").is_err());
}

#[test]
fn bad_cursor_is_rejected_not_panicked() {
    let session = SessionId::new();
    let session_id = "session_x";
    let events = many_input_events(session, 3);
    let build = |cursor: &str| {
        build_segment(
            session_id,
            &events,
            AtifOptions::default(),
            Some(cursor),
            256,
            LINK_BASE,
        )
    };
    assert!(build("not-base64!!!").is_err());
    assert!(build("").is_err());
    // Valid base64 but not JSON.
    {
        use base64::Engine;
        let junk = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"not json");
        assert!(build(&junk).is_err());
    }
    // Foreign-session cursor is rejected (cannot widen scope).
    let foreign = SegmentCursor {
        session_id: "someone_elses_session".to_string(),
        offset: 1,
    }
    .encode();
    assert!(build(&foreign).is_err());
    // Offset past the end is rejected.
    let past_end = SegmentCursor {
        session_id: session_id.to_string(),
        offset: 999,
    }
    .encode();
    assert!(build(&past_end).is_err());
    // Oversized cursor rejected before parsing.
    assert!(build(&"A".repeat(MAX_CURSOR_BYTES + 1)).is_err());
}

#[test]
fn final_segment_has_no_continuation_when_it_fits() {
    let session = SessionId::new();
    let session_id = "session_x";
    let events = many_input_events(session, 2);
    // Large cap → the whole session fits in one segment.
    let seg = build_segment(
        session_id,
        &events,
        AtifOptions::default(),
        None,
        ATIF_EXPORT_MAX_BYTES,
        LINK_BASE,
    )
    .unwrap();
    assert!(seg.next_cursor.is_none());
    assert!(seg.document.get("continued_trajectory_ref").is_none());
    assert_eq!(seg.document["steps"].as_array().unwrap().len(), 2);
    assert_eq!(seg.segment_index, 0);
}

#[test]
fn single_oversized_step_returned_alone_over_cap() {
    let session = SessionId::new();
    let session_id = "session_x";
    // One user message far larger than the (tiny) cap.
    let big = "x".repeat(50_000);
    let events = vec![
        event(session, InputMessageData::new(RuntimeMessage::user(big))),
        event(
            session,
            InputMessageData::new(RuntimeMessage::user("small")),
        ),
    ];
    // Cap of 1 KiB: the giant step cannot fit but must still be emitted
    // alone rather than 413 or loop forever.
    let (walked, docs) = walk_segments(session_id, &events, 1024);
    assert_eq!(docs.len(), 2, "one segment per step under the tiny cap");
    assert_eq!(walked.len(), 2);
    // The oversized first segment is allowed to exceed the cap.
    let first_len = serde_json::to_vec(&docs[0]).unwrap().len();
    assert!(
        first_len > 1024,
        "the giant single-step segment may exceed the cap ({first_len} bytes)",
    );
}

#[test]
fn scrubbing_is_applied_per_segment() {
    let session = SessionId::new();
    let session_id = "session_x";
    // Put a secret in the second message so it lands in a later segment.
    let events = vec![
        event(
            session,
            InputMessageData::new(RuntimeMessage::user("hello")),
        ),
        event(
            session,
            InputMessageData::new(RuntimeMessage::user("token sk-abcdef0123456789ABCDEF here")),
        ),
    ];
    let (_, docs) = walk_segments(session_id, &events, 256);
    assert_eq!(docs.len(), 2);
    for doc in &docs {
        let serialized = serde_json::to_string(doc).unwrap();
        assert!(
            !serialized.contains("sk-abcdef0123456789ABCDEF"),
            "secret must be scrubbed in every segment",
        );
    }
    // And the secret-bearing segment shows the redaction marker.
    assert!(serde_json::to_string(&docs[1]).unwrap().contains(REDACTED));
}

#[test]
fn empty_session_yields_one_empty_final_segment() {
    let session_id = "session_x";
    let events: Vec<Event> = vec![];
    let seg = build_segment(
        session_id,
        &events,
        AtifOptions::default(),
        None,
        ATIF_EXPORT_MAX_BYTES,
        LINK_BASE,
    )
    .unwrap();
    assert!(seg.next_cursor.is_none());
    assert_eq!(seg.document["steps"].as_array().unwrap().len(), 0);
    assert_eq!(seg.segment_index, 0);
    // A cursor exactly at the end is accepted and yields the empty tail.
    let at_end = SegmentCursor {
        session_id: session_id.to_string(),
        offset: 0,
    }
    .encode();
    assert!(
        build_segment(
            session_id,
            &events,
            AtifOptions::default(),
            Some(&at_end),
            ATIF_EXPORT_MAX_BYTES,
            LINK_BASE,
        )
        .is_ok()
    );
}
