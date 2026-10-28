use super::*;

fn message(content: Value, stop: &str) -> Value {
    json!({"id":"msg_test","model":"claude-sonnet-5","stop_reason":stop,"content":content,"usage":{"input_tokens":2,"output_tokens":3}})
}

#[test]
fn decoding_preserves_reasoning_tools_usage_and_opaque_replay() {
    let content = json!([
        {"type":"thinking","thinking":"plan","signature":"signed"},
        {"type":"redacted_thinking","data":"opaque"},
        {"type":"text","text":"checking"},
        {"type":"tool_use","id":"tool_1","name":"weather","input":{"city":"Kyiv"}},
        {"type":"future_provider_block","value":42}
    ]);
    let output = decode_message(&message(content.clone(), "tool_use")).unwrap();
    assert_eq!(output.text, "checking");
    assert_eq!(
        output.tool_calls.unwrap()[0].arguments,
        json!({"city":"Kyiv"})
    );
    assert_eq!(output.reasoning[0].signature.as_deref(), Some("signed"));
    assert_eq!(output.reasoning[1].encrypted.as_deref(), Some("opaque"));
    assert_eq!(
        output.metadata.provider_opaque_content.unwrap().content,
        content
    );
    assert_eq!(output.metadata.total_tokens, Some(5));
    assert_eq!(output.metadata.finish_reason.as_deref(), Some("tool_calls"));
}

#[test]
fn truncated_or_refused_calls_are_never_returned_for_execution() {
    for (stop, normalized) in [
        ("max_tokens", "length"),
        ("refusal", "refusal"),
        ("end_turn", "stop"),
        ("pause_turn", "stop"),
    ] {
        let output = decode_message(&message(
            json!([{"type":"tool_use","id":"t","name":"write","input":{"path":"x"}}]),
            stop,
        ))
        .unwrap();
        assert!(output.tool_calls.is_none());
        assert_eq!(output.metadata.tool_calls_dropped, 1);
        assert_eq!(output.metadata.finish_reason.as_deref(), Some(normalized));
        assert_eq!(
            output.metadata.provider_finish_reason.as_deref(),
            Some(stop)
        );
    }
}

#[test]
fn malformed_known_blocks_and_partial_messages_are_rejected() {
    for content in [
        json!([{"type":"text"}]),
        json!([{"type":"tool_use","id":"t","name":"write","input":"invalid"}]),
        json!([{"type":"thinking","thinking":"no signature"}]),
    ] {
        assert!(decode_message(&message(content, "tool_use")).is_err());
    }
    for field in ["id", "model", "usage", "stop_reason", "content"] {
        let mut value = message(json!([{"type":"text","text":"hello"}]), "end_turn");
        value.as_object_mut().unwrap().remove(field);
        assert!(decode_message(&value).is_err(), "accepted missing {field}");
    }
}

#[test]
fn serialization_is_bounded_before_allocating_an_oversized_body() {
    let mut body = BoundedBody {
        bytes: Vec::new(),
        max_bytes: 12,
    };
    assert!(serde_json::to_writer(&mut body, &"long text that exceeds twelve bytes").is_err());
    assert!(body.bytes.len() <= 12);
    let mut body = BoundedBody {
        bytes: Vec::new(),
        max_bytes: 4,
    };
    body.write_all(b"1234").unwrap();
    assert!(body.write_all(b"5").is_err());
    assert_eq!(body.bytes, b"1234");
}

#[test]
fn provider_ids_and_states_are_validated() {
    for id in ["../other", "x?token=secret", "a/b", "", &"x".repeat(65)] {
        assert_eq!(path(id, "/results").unwrap_err(), BatchError::InvalidTicket);
    }
    for state in ["queued", "unknown", ""] {
        assert!(
            status(
                &json!({"id":"msgbatch_test","processing_status":state}),
                None
            )
            .is_err()
        );
    }
    assert!(
        status(
            &json!({"id":"other","processing_status":"ended"}),
            Some("expected")
        )
        .is_err()
    );
}
