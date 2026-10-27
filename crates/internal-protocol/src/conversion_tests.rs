use super::*;

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

fn json_wire_sample_message() -> everruns_core::RuntimeMessage {
    use everruns_core::{ContentPart, Controls, ExternalActor, RuntimeMessage, RuntimeMessageRole};

    RuntimeMessage {
        id: uuid::Uuid::now_v7().into(),
        role: RuntimeMessageRole::User,
        content: vec![ContentPart::text("Hello"), ContentPart::text("again")],
        phase: Some(everruns_contracts::ExecutionPhase::FinalAnswer),
        phase_source: Some(everruns_contracts::PhaseSource::Provider),
        controls: Some(Controls {
            locale: Some("uk-UA".to_string()),
            ..Default::default()
        }),
        // An integer that must stay an integer: the Struct path carries every
        // number as f64.
        metadata: Some([("count".to_string(), serde_json::json!(5314))].into()),
        external_actor: Some(ExternalActor {
            actor_id: "U0123456789".to_string(),
            actor_name: Some("Alice".to_string()),
            source: "slack".to_string(),
            metadata: None,
        }),
        created_at: chrono::Utc::now(),
    }
}

/// Both wire forms of a message decode to the same message, and the bytes
/// form leaves the Struct fields empty so they cost nothing to encode.
#[test]
fn test_message_json_bytes_and_struct_forms_agree() {
    let message = json_wire_sample_message();
    let expected = serde_json::to_value(&message).unwrap();

    let as_struct = message_to_proto_with(&message, false);
    assert!(as_struct.content.is_some() && as_struct.content_json.is_empty());
    let as_bytes = message_to_proto_with(&message, true);
    assert!(as_bytes.content.is_none() && as_bytes.controls.is_none());
    assert!(as_bytes.metadata.is_none() && as_bytes.external_actor.is_none());
    assert!(!as_bytes.content_json.is_empty());

    for proto_message in [as_struct, as_bytes] {
        let decoded = proto_message_to_schema(proto_message).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), expected);
    }
}

/// A receiver reads the bytes when a sender fills both forms.
#[test]
fn test_message_json_bytes_take_precedence_over_struct() {
    let message = json_wire_sample_message();
    let mut proto_message = message_to_proto_with(&message, false);
    proto_message.content_json = br#"[{"type":"text","text":"from bytes"}]"#.to_vec();

    let decoded = proto_message_to_schema(proto_message).unwrap();
    assert_eq!(decoded.content.len(), 1);
    assert_eq!(decoded.content[0].as_text(), Some("from bytes"));
}

/// Malformed bytes are an error, not a silently empty message.
#[test]
fn test_message_json_bytes_malformed_is_error() {
    let mut proto_message = message_to_proto_with(&json_wire_sample_message(), true);
    proto_message.content_json = b"not json".to_vec();
    assert!(matches!(
        proto_message_to_schema(proto_message),
        Err(ConversionError::JsonError(_))
    ));
}

fn json_wire_sample_event_request() -> everruns_core::EventRequest {
    use everruns_core::{EventContext, EventRequest, InputMessageData};

    let mut request = EventRequest::new(
        everruns_contracts::typed_id::SessionId::from_uuid(uuid::Uuid::now_v7()),
        EventContext::empty(),
        InputMessageData::new(json_wire_sample_message()),
    );
    request.metadata = Some(serde_json::json!({"attempt": 2, "ratio": 0.5}));
    request
}

#[test]
fn test_event_request_json_bytes_and_struct_forms_agree() {
    let request = json_wire_sample_event_request();
    let expected = serde_json::to_value(&request).unwrap();

    for json_bytes in [false, true] {
        let payload = encode_event_payload(&request.data, request.metadata.as_ref(), json_bytes);
        assert_eq!(payload.data.is_some(), !json_bytes);
        assert_eq!(payload.data_json.is_empty(), !json_bytes);
        let mut proto_request = schema_event_request_to_proto(&request);
        proto_request.data = payload.data;
        proto_request.metadata = payload.metadata;
        proto_request.data_json = payload.data_json;
        proto_request.metadata_json = payload.metadata_json;

        let decoded = proto_event_request_to_schema(proto_request).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), expected);
    }
}

#[test]
fn test_event_json_bytes_and_struct_forms_agree() {
    let request = json_wire_sample_event_request();
    let event = everruns_core::Event {
        id: EventId::from_uuid(uuid::Uuid::now_v7()),
        event_type: request.event_type.clone(),
        ts: request.ts,
        session_id: request.session_id,
        context: request.context.clone(),
        data: request.data.clone(),
        metadata: request.metadata.clone(),
        tags: Some(vec!["t".to_string()]),
        sequence: Some(7),
    };
    let expected = serde_json::to_value(&event).unwrap();

    for json_bytes in [false, true] {
        let payload = encode_event_payload(&event.data, event.metadata.as_ref(), json_bytes);
        let mut proto_event = schema_event_to_proto(&event);
        proto_event.data = payload.data;
        proto_event.metadata = payload.metadata;
        proto_event.data_json = payload.data_json;
        proto_event.metadata_json = payload.metadata_json;

        let decoded = proto_event_to_schema(proto_event).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), expected);
    }
}

/// An event with neither wire form of `data` is still rejected.
#[test]
fn test_event_without_data_is_missing_field() {
    let mut proto_request = schema_event_request_to_proto(&json_wire_sample_event_request());
    proto_request.data = None;
    proto_request.data_json.clear();
    assert!(matches!(
        proto_event_request_to_schema(proto_request),
        Err(ConversionError::MissingField("data"))
    ));
}
