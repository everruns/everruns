use super::*;

#[test]
fn test_reason_completed_data_roundtrip() {
    use everruns_core::events::ReasonCompletedData;

    // Create test data
    let data = ReasonCompletedData::success("Test response", true, 3, Some(2000), None);

    // Serialize to JSON
    let json = serde_json::to_value(&data).unwrap();

    // Convert to proto struct and back
    let proto_struct = json_to_proto_struct(&json);
    let result_json = proto_struct_to_json(&proto_struct);

    // Deserialize back to ReasonCompletedData
    let result: ReasonCompletedData = serde_json::from_value(result_json).unwrap();

    assert!(result.success);
    assert_eq!(result.tool_call_count, 3);
    assert!(result.has_tool_calls);
}

#[test]
fn test_message_reasoning_roundtrip() {
    use chrono::Utc;
    use everruns_contracts::reasoning::{ReasoningContentPart, ReasoningText};
    use everruns_core::{ContentPart, RuntimeMessage, RuntimeMessageRole};
    use uuid::Uuid;

    // Two separately-signed reasoning artifacts, as interleaved thinking
    // produces. Both must survive the worker boundary with their own
    // signature: a merged pair carrying one signature is exactly what the
    // provider rejects.
    let message = RuntimeMessage {
        id: Uuid::now_v7().into(),
        role: RuntimeMessageRole::Agent,
        content: vec![
            ContentPart::Reasoning(
                ReasoningContentPart::opaque("anthropic")
                    .with_signature("sig-first")
                    .with_text(ReasoningText::Plain {
                        text: "First I check the logs".to_string(),
                    }),
            ),
            ContentPart::Reasoning(
                ReasoningContentPart::opaque("anthropic")
                    .with_signature("sig-second")
                    .with_text(ReasoningText::Plain {
                        text: "Now I read the diff".to_string(),
                    }),
            ),
            ContentPart::text("Here is my response based on my analysis."),
        ],
        phase: Some(everruns_contracts::ExecutionPhase::Commentary),
        phase_source: Some(everruns_contracts::PhaseSource::Derived),
        controls: None,
        metadata: None,
        external_actor: None,
        created_at: Utc::now(),
    };

    let roundtripped = proto_message_to_schema(schema_message_to_proto(&message)).unwrap();

    let parts: Vec<&ReasoningContentPart> = roundtripped.reasoning_parts().collect();
    assert_eq!(parts.len(), 2, "both artifacts must survive");
    assert_eq!(parts[0].signature.as_deref(), Some("sig-first"));
    assert_eq!(parts[1].signature.as_deref(), Some("sig-second"));
    assert_eq!(
        parts[0].text,
        Some(ReasoningText::Plain {
            text: "First I check the logs".to_string(),
        })
    );
    assert_eq!(roundtripped.role, RuntimeMessageRole::Agent);
}

/// Phase and its source cross the boundary. Dropping either leaves the API
/// unable to say whether a message is the answer, or whether its
/// classification came from the provider or was inferred.
#[test]
fn test_message_phase_and_source_roundtrip() {
    use chrono::Utc;
    use everruns_contracts::{ExecutionPhase, PhaseSource};
    use everruns_core::{ContentPart, RuntimeMessage, RuntimeMessageRole};
    use uuid::Uuid;

    for (phase, source) in [
        (ExecutionPhase::Commentary, PhaseSource::Derived),
        (ExecutionPhase::FinalAnswer, PhaseSource::Provider),
    ] {
        let message = RuntimeMessage {
            id: Uuid::now_v7().into(),
            role: RuntimeMessageRole::Agent,
            content: vec![ContentPart::text("answer")],
            phase: Some(phase),
            phase_source: Some(source),
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: Utc::now(),
        };

        let roundtripped = proto_message_to_schema(schema_message_to_proto(&message)).unwrap();
        assert_eq!(roundtripped.phase, Some(phase));
        assert_eq!(roundtripped.phase_source, Some(source));
    }
}

#[test]
fn test_message_without_reasoning_roundtrip() {
    use chrono::Utc;
    use everruns_core::{ContentPart, RuntimeMessage, RuntimeMessageRole};
    use uuid::Uuid;

    let message = RuntimeMessage {
        id: Uuid::now_v7().into(),
        role: RuntimeMessageRole::Agent,
        content: vec![ContentPart::text("A simple response without reasoning.")],
        phase: None,
        phase_source: None,
        controls: None,
        metadata: None,
        external_actor: None,
        created_at: Utc::now(),
    };

    let roundtripped = proto_message_to_schema(schema_message_to_proto(&message)).unwrap();
    assert!(!roundtripped.has_reasoning());
    assert_eq!(roundtripped.phase, None);
    assert_eq!(roundtripped.phase_source, None);
}

#[test]
fn test_external_actor_proto_roundtrip() {
    use chrono::Utc;
    use everruns_core::{ContentPart, ExternalActor, RuntimeMessage, RuntimeMessageRole};
    use uuid::Uuid;

    let actor = ExternalActor {
        actor_id: "U0123456789".to_string(),
        actor_name: Some("Alice".to_string()),
        source: "slack".to_string(),
        metadata: Some(
            [("channel".to_string(), "C999".to_string())]
                .into_iter()
                .collect(),
        ),
    };

    let message = RuntimeMessage {
        id: Uuid::now_v7().into(),
        role: RuntimeMessageRole::User,
        content: vec![ContentPart::text("Hello")],
        phase: None,
        phase_source: None,
        controls: None,
        metadata: None,
        external_actor: Some(actor.clone()),
        created_at: Utc::now(),
    };

    let proto_message = schema_message_to_proto(&message);
    assert!(proto_message.external_actor.is_some());

    let schema_message = proto_message_to_schema(proto_message).unwrap();
    let roundtripped = schema_message.external_actor.unwrap();
    assert_eq!(roundtripped.actor_id, "U0123456789");
    assert_eq!(roundtripped.actor_name, Some("Alice".to_string()));
    assert_eq!(roundtripped.source, "slack");
    assert_eq!(
        roundtripped
            .metadata
            .as_ref()
            .unwrap()
            .get("channel")
            .unwrap(),
        "C999"
    );
}

#[test]
fn test_external_actor_none_proto_roundtrip() {
    use chrono::Utc;
    use everruns_core::{ContentPart, RuntimeMessage, RuntimeMessageRole};
    use uuid::Uuid;

    let message = RuntimeMessage {
        id: Uuid::now_v7().into(),
        role: RuntimeMessageRole::User,
        content: vec![ContentPart::text("Hello")],
        phase: None,
        phase_source: None,
        controls: None,
        metadata: None,
        external_actor: None,
        created_at: Utc::now(),
    };

    let proto_message = schema_message_to_proto(&message);
    assert!(proto_message.external_actor.is_none());

    let schema_message = proto_message_to_schema(proto_message).unwrap();
    assert!(schema_message.external_actor.is_none());
}

#[test]
fn test_conversion_error_to_tonic_status() {
    let err = ConversionError::MissingField("session_id");
    let status: tonic::Status = err.into();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(
        status
            .message()
            .contains("Missing required field: session_id")
    );

    let err = ConversionError::JsonError(
        serde_json::from_str::<serde_json::Value>("invalid").unwrap_err(),
    );
    let status: tonic::Status = err.into();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert!(status.message().contains("JSON error"));
}

// EVE-652: a proto entity missing its required id used to build an empty-string
// typed id and fail later as an opaque JsonError. It now fails with the precise
// MissingField, instead of silently corrupting the id.
// EVE-652: a proto entity missing its required id fails with the precise
// MissingField("id") across every entity's proto->schema conversion.
#[test]
fn test_prefixed_id_strips_dashes_and_prefixes() {
    let u = proto::Uuid {
        value: "0191e1a2-3b4c-7d8e-9f00-112233445566".to_string(),
    };
    assert_eq!(
        prefixed_id("agent", &u),
        "agent_0191e1a23b4c7d8e9f00112233445566"
    );
}

// EVE-652: known roles map exactly; an unknown role still defaults to User
// (now logged) rather than erroring or being dropped.
#[test]
fn test_parse_message_role_known_and_unknown() {
    use everruns_core::RuntimeMessageRole;
    assert!(matches!(
        parse_message_role("system"),
        RuntimeMessageRole::System
    ));
    assert!(matches!(
        parse_message_role("USER"),
        RuntimeMessageRole::User
    ));
    assert!(matches!(
        parse_message_role("assistant"),
        RuntimeMessageRole::Agent
    ));
    assert!(matches!(
        parse_message_role("agent"),
        RuntimeMessageRole::Agent
    ));
    assert!(matches!(
        parse_message_role("tool_result"),
        RuntimeMessageRole::ToolResult
    ));
    assert!(matches!(
        parse_message_role("something_unknown"),
        RuntimeMessageRole::User
    ));
}

// ------------------------------------------------------------------------
// Session task registry native-proto round-trip fidelity (EVE-642)
// ------------------------------------------------------------------------

fn sample_session_task() -> st::SessionTask {
    st::SessionTask {
        id: "task_abc123".to_string(),
        session_id: SessionId::new(),
        root_session_id: None,
        kind: st::TASK_KIND_SUBAGENT.to_string(),
        display_name: "Investigate flake".to_string(),
        spec: serde_json::json!({
            "instructions": "run tests",
            "retries": 3,
            "nested": { "a": [1, 2, 3], "b": null, "flag": true },
        }),
        state: st::SessionTaskState::AwaitingInput,
        state_detail: Some("iteration 4/10".to_string()),
        progress: Some(st::TaskProgress {
            current: Some(4),
            total: Some(10),
            unit: Some("steps".to_string()),
            label: Some("running".to_string()),
            steps: vec![
                everruns_core::background::ProgressStep {
                    title: "Read".to_string(),
                    status: everruns_core::background::ProgressStepStatus::Done,
                },
                everruns_core::background::ProgressStep {
                    title: "Write".to_string(),
                    status: everruns_core::background::ProgressStepStatus::InProgress,
                },
            ],
        }),
        input_request: Some(st::TaskInputRequest {
            id: "req_1".to_string(),
            prompt: "Approve?".to_string(),
            expected: Some(serde_json::json!({ "type": "boolean" })),
        }),
        cancel_requested_at: Some(Utc.timestamp_opt(1_700_000_100, 0).unwrap()),
        summary: Some("did the thing".to_string()),
        result_path: Some("/.tasks/task_abc123/result.json".to_string()),
        artifacts: vec![
            st::TaskArtifact {
                name: "report".to_string(),
                artifact_type: "file".to_string(),
                path: Some("/report.md".to_string()),
                url: None,
            },
            st::TaskArtifact {
                name: "pr".to_string(),
                artifact_type: "url".to_string(),
                path: None,
                url: Some("https://example.com/pr/1".to_string()),
            },
        ],
        error: Some(st::TaskError {
            kind: "timeout".to_string(),
            message: "exceeded deadline".to_string(),
        }),
        attempt: 2,
        worker_id: Some("worker-7".to_string()),
        heartbeat_at: Some(Utc.timestamp_opt(1_700_000_200, 500_000_000).unwrap()),
        links: st::TaskLinks {
            child_session_id: Some(SessionId::new()),
            remote_task_id: Some("rt_9".to_string()),
            resource_ids: vec!["res_1".to_string(), "res_2".to_string()],
        },
        wake_policy: st::TaskWakePolicy::OnActivity,
        created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        started_at: Some(Utc.timestamp_opt(1_700_000_050, 0).unwrap()),
        finished_at: None,
        updated_at: Utc.timestamp_opt(1_700_000_300, 0).unwrap(),
    }
}

/// Full round-trip must preserve the task through the native proto and back
/// so switching the wire format from JSON-in-bytes to native protobuf is
/// lossless. Comparison is done via canonical JSON so the numeric widening
/// in `google.protobuf.Value` (spec/expected) is accounted for.
#[test]
fn session_task_native_proto_round_trip() {
    let original = sample_session_task();
    let proto = session_task_to_proto(&original);
    let restored = proto_to_session_task(proto).expect("round-trip");

    let a = serde_json::to_value(&original).unwrap();
    let b = serde_json::to_value(&restored).unwrap();
    assert_eq!(a, b, "session task must survive native-proto round-trip");
}

#[test]
fn create_session_task_native_proto_round_trip() {
    let original = st::CreateSessionTask {
        session_id: SessionId::new(),
        id: Some("task_seed".to_string()),
        kind: st::TASK_KIND_BACKGROUND_TOOL.to_string(),
        display_name: "seed".to_string(),
        spec: serde_json::json!({ "k": "v", "n": 12 }),
        state: st::SessionTaskState::Running,
        links: st::TaskLinks {
            child_session_id: None,
            remote_task_id: Some("rt".to_string()),
            resource_ids: vec!["res".to_string()],
        },
        wake_policy: st::TaskWakePolicy::OnTerminal,
    };
    let proto = create_session_task_to_proto(&original);
    let restored = proto_to_create_session_task(proto).expect("round-trip");

    assert_eq!(
        serde_json::to_value(&original).unwrap(),
        serde_json::to_value(&restored).unwrap()
    );
}

#[test]
fn append_artifact_has_an_independent_native_wire_field() {
    use prost::Message;
    let artifact = st::TaskArtifact {
        name: "a".into(),
        artifact_type: "file".into(),
        path: Some("/a".into()),
        url: None,
    };
    let update = st::SessionTaskUpdate {
        append_artifact: Some(artifact.clone()),
        ..Default::default()
    };
    let expected = [
        0x72, 0x0d, 0x0a, 0x01, b'a', 0x12, 0x04, b'f', b'i', b'l', b'e', 0x1a, 0x02, b'/', b'a',
    ];
    assert_eq!(
        session_task_update_to_proto(&update).encode_to_vec(),
        expected
    );
    let restored = proto_to_session_task_update(
        proto::SessionTaskUpdateProto::decode(expected.as_slice()).unwrap(),
    );
    assert_eq!(restored.append_artifact, Some(artifact));
    assert!(restored.artifacts.is_none());
    assert!(
        proto_to_session_task_update(proto::SessionTaskUpdateProto::default())
            .append_artifact
            .is_none()
    );
}

#[test]
fn session_task_update_native_proto_round_trip() {
    // A fully-populated update (all Option/Vec fields set).
    let full = st::SessionTaskUpdate {
        state: Some(st::SessionTaskState::Failed),
        state_detail: Some("boom".to_string()),
        progress: Some(st::TaskProgress {
            current: Some(1),
            total: None,
            unit: None,
            label: Some("x".to_string()),
            ..Default::default()
        }),
        input_request: Some(st::TaskInputRequest {
            id: "r".to_string(),
            prompt: "p".to_string(),
            expected: None,
        }),
        summary: Some("s".to_string()),
        result_path: Some("/r.json".to_string()),
        artifacts: Some(vec![st::TaskArtifact {
            name: "a".to_string(),
            artifact_type: "file".to_string(),
            path: Some("/a".to_string()),
            url: None,
        }]),
        error: Some(st::TaskError {
            kind: "orphaned".to_string(),
            message: "m".to_string(),
        }),
        links: Some(st::TaskLinks::default()),
        worker_id: Some("w".to_string()),
        heartbeat_at: Some(Utc.timestamp_opt(1_700_000_000, 0).unwrap()),
        expected_attempt: Some(3),
        increment_attempt: true,
        append_artifact: Some(st::TaskArtifact {
            name: "appended".into(),
            artifact_type: "file".into(),
            path: Some("/appended".into()),
            url: None,
        }),
    };
    let restored = proto_to_session_task_update(session_task_update_to_proto(&full));
    assert_eq!(
        serde_json::to_value(&full).unwrap(),
        serde_json::to_value(&restored).unwrap()
    );

    // An empty update must stay empty: crucially, an absent `artifacts`
    // must not become `Some(vec![])`, which would wrongly clear artifacts.
    let empty = st::SessionTaskUpdate::default();
    let restored_empty = proto_to_session_task_update(session_task_update_to_proto(&empty));
    assert!(restored_empty.state.is_none());
    assert!(
        restored_empty.artifacts.is_none(),
        "absent artifacts update must round-trip as None, not Some(empty)"
    );
    assert!(!restored_empty.increment_attempt);
    assert!(restored_empty.expected_attempt.is_none());

    // An explicit empty artifact list (clear all) must survive as Some(empty).
    let clear = st::SessionTaskUpdate {
        artifacts: Some(vec![]),
        ..Default::default()
    };
    let restored_clear = proto_to_session_task_update(session_task_update_to_proto(&clear));
    assert_eq!(restored_clear.artifacts, Some(vec![]));
}

#[test]
fn task_message_native_proto_round_trip() {
    let msg = st::TaskMessage {
        id: "tmsg_1".to_string(),
        task_id: "task_1".to_string(),
        direction: st::TaskMessageDirection::Outbound,
        content: vec![
            st::TaskMessagePart::text("hello"),
            st::TaskMessagePart::Data {
                data: serde_json::json!({ "k": [1, 2], "b": true }),
            },
        ],
        in_reply_to: Some("tmsg_0".to_string()),
        created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
    };
    let restored = proto_to_task_message(task_message_to_proto(&msg)).expect("round-trip");
    assert_eq!(
        serde_json::to_value(&msg).unwrap(),
        serde_json::to_value(&restored).unwrap()
    );
}

#[test]
fn new_task_message_native_proto_round_trip() {
    let msg = st::NewTaskMessage {
        direction: st::TaskMessageDirection::Inbound,
        content: vec![st::TaskMessagePart::text("answer")],
        in_reply_to: Some("req_1".to_string()),
        expected_attempt: Some(5),
    };
    let restored = proto_to_new_task_message(new_task_message_to_proto(&msg));
    assert_eq!(
        serde_json::to_value(&msg).unwrap(),
        serde_json::to_value(&restored).unwrap()
    );
}
