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
    reply.push_text("Let me check.", true);
    reply.extend_sent(["Shipped.".to_string()]);
    assert_eq!(reply.take(), "Shipped.");
}

#[test]
fn said_in_event_counts_replies_and_sent_messages_only() {
    let reply = json!({"message":{"content":[{"type":"text","text":"Done."}]}});
    let notes =
        json!({"message":{"phase":"commentary","content":[{"type":"text","text":"thinking"}]}});
    let sent = json!({"message_id":"message_01","text":"Shipped.","tool_call_id":"c1"});
    assert_eq!(
        said_in_event("output.message.completed", &reply).as_deref(),
        Some("Done.")
    );
    assert_eq!(said_in_event("output.message.completed", &notes), None);
    assert_eq!(
        said_in_event("conversation.message", &sent).as_deref(),
        Some("Shipped.")
    );
    assert_eq!(said_in_event("tool.completed", &sent), None);
    let preamble = json!({"message":{"content":[
        {"type":"text","text":"Let me check."},
        {"type":"tool_call","id":"c1","name":"search","arguments":{}}
    ]}});
    assert!(
        said_message_in_event("output.message.completed", &preamble)
            .unwrap()
            .with_tool_calls
    );
    assert!(
        !said_message_in_event("conversation.message", &sent)
            .unwrap()
            .with_tool_calls
    );
}

#[test]
fn said_in_transcript_reads_notes_out_and_delivered_messages_in() {
    let mut notes =
        RuntimeMessage::assistant("working notes").with_phase(ExecutionPhase::Commentary);
    notes.content.push(ContentPart::tool_call(
        "sent",
        SEND_MESSAGE_TOOL_NAME,
        json!({"text":"Here is the answer."}),
    ));
    notes.content.push(ContentPart::tool_call(
        "failed",
        SEND_MESSAGE_TOOL_NAME,
        json!({"text":"never delivered"}),
    ));
    let transcript = vec![
        RuntimeMessage::user("question"),
        notes,
        RuntimeMessage::tool_result("sent", Some(json!({"sent":true,"message_id":"m"})), None),
        RuntimeMessage::tool_result("failed", None, Some("refused".into())),
    ];
    let said = said_in_transcript(&transcript);
    assert_eq!(
        said,
        vec![SaidMessage {
            text: "Here is the answer.".into(),
            with_tool_calls: false
        }]
    );
    assert_eq!(final_reply(said).as_deref(), Some("Here is the answer."));
    // A direct agent's transcript reads as before.
    assert_eq!(
        final_reply(said_in_transcript(&[RuntimeMessage::assistant("Plain.")])).as_deref(),
        Some("Plain.")
    );
}

#[test]
fn transcript_lines_keep_one_line_per_message() {
    let mut notes =
        RuntimeMessage::assistant("working notes").with_phase(ExecutionPhase::Commentary);
    notes.content.push(ContentPart::tool_call(
        "sent",
        SEND_MESSAGE_TOOL_NAME,
        json!({"text":"Here is the answer."}),
    ));
    let transcript = vec![
        RuntimeMessage::user("question"),
        notes,
        RuntimeMessage::tool_result("sent", Some(json!({"sent":true})), None),
        RuntimeMessage::assistant("Plain."),
    ];
    assert_eq!(
        transcript_lines(&transcript),
        vec![
            Some("question".to_string()),
            Some("Here is the answer.".to_string()),
            None,
            Some("Plain.".to_string()),
        ]
    );
}
