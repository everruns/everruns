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
