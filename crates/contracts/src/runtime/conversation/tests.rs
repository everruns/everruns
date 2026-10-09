use super::*;
use serde_json::json;

fn with_tool_call(text: &str) -> RuntimeMessage {
    let mut message = RuntimeMessage::assistant(text);
    message
        .content
        .push(ContentPart::tool_call("call_1", "search", json!({})));
    message
}

#[test]
fn said_text_is_the_agent_text_and_nothing_else() {
    let cases: Vec<(&str, RuntimeMessage, Option<&str>)> = vec![
        (
            "plain answer",
            RuntimeMessage::assistant("Done."),
            Some("Done."),
        ),
        (
            "commentary",
            RuntimeMessage::assistant("thinking").with_phase(ExecutionPhase::Commentary),
            None,
        ),
        (
            "final answer phase",
            RuntimeMessage::assistant("Here.").with_phase(ExecutionPhase::FinalAnswer),
            Some("Here."),
        ),
        ("user message", RuntimeMessage::user("hi"), None),
        ("blank text", RuntimeMessage::assistant("  \n"), None),
        (
            "tool call preamble",
            with_tool_call("Let me check"),
            Some("Let me check"),
        ),
        ("tool call only", with_tool_call(""), None),
    ];
    for (name, message, expected) in cases {
        assert_eq!(said_text(&message).as_deref(), expected, "{name}");
    }
}

#[test]
fn spoken_text_joins_text_parts_and_drops_others() {
    let parts = vec![
        ContentPart::text("one"),
        ContentPart::tool_call("c", "t", json!({})),
        ContentPart::text(""),
        ContentPart::text("two"),
    ];
    assert_eq!(spoken_text(&parts), "one\ntwo");
}

#[test]
fn final_reply_prefers_the_last_message_that_was_not_a_preamble() {
    let said = |text: &str, with_tool_calls| SaidMessage {
        text: text.into(),
        with_tool_calls,
    };
    let cases: Vec<(&str, Vec<SaidMessage>, Option<&str>)> = vec![
        ("nothing said", vec![], None),
        (
            "preamble then answer",
            vec![said("Let me check", true), said("It is 4.", false)],
            Some("It is 4."),
        ),
        (
            "answer then trailing preamble",
            vec![said("Partial.", false), said("Checking more", true)],
            Some("Partial."),
        ),
        (
            "only preamble",
            vec![said("Checking", true)],
            Some("Checking"),
        ),
        (
            "several answers",
            vec![said("First.", false), said("Second.", false)],
            Some("Second."),
        ),
    ];
    for (name, said, expected) in cases {
        assert_eq!(final_reply(said).as_deref(), expected, "{name}");
    }
}

#[test]
fn said_text_in_event_agrees_with_the_typed_reader() {
    let messages = [
        RuntimeMessage::assistant("Done."),
        RuntimeMessage::assistant("thinking").with_phase(ExecutionPhase::Commentary),
        RuntimeMessage::assistant("Here.").with_phase(ExecutionPhase::FinalAnswer),
        RuntimeMessage::assistant(" "),
        with_tool_call("Let me check"),
    ];
    for message in messages {
        let data = json!({ "message": serde_json::to_value(&message).unwrap() });
        assert_eq!(
            said_text_in_event(&data),
            said_text(&message),
            "{message:?}"
        );
    }
    assert_eq!(
        said_text_in_event(&json!({"message": {"content": [{"type": "text", "text": "hi"}]}})),
        Some("hi".to_string())
    );
    assert_eq!(said_text_in_event(&json!({})), None);
}

#[test]
fn turn_reply_skips_blank_text_and_resets_on_take() {
    let mut reply = TurnReply::default();
    reply.push_text("  ", false);
    reply.push_text("Let me check.", true);
    reply.push_text("Here it is.", false);
    assert_eq!(reply.take(), "Here it is.");
    assert_eq!(reply.take(), "");
}
