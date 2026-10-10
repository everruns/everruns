use super::*;
use crate::runtime::{events::EventContext, typed_id::SessionId};
use std::sync::Mutex;

struct RecordingSender {
    sends: Mutex<Vec<(String, MessageId, String)>>,
    outcome: Outcome,
}

#[derive(Clone, Copy)]
enum Outcome {
    Delivered,
    NotExternal,
    Refused,
}

impl RecordingSender {
    fn new(outcome: Outcome) -> Arc<Self> {
        Arc::new(Self {
            sends: Mutex::new(vec![]),
            outcome,
        })
    }
}

#[async_trait]
impl ConversationSender for RecordingSender {
    async fn send(
        &self,
        text: &str,
        input: MessageId,
        call: &str,
    ) -> Result<Option<ConversationDelivery>, ToolExecutionResult> {
        match self.outcome {
            Outcome::Refused => Err(ToolExecutionResult::tool_error("channel refused")),
            Outcome::NotExternal => Ok(None),
            Outcome::Delivered => {
                self.sends
                    .lock()
                    .unwrap()
                    .push((text.into(), input, call.into()));
                Ok(Some(ConversationDelivery {
                    platform: "slack".into(),
                    channel: "C1".into(),
                    message_ref: "171.2".into(),
                }))
            }
        }
    }
}

fn context(sender: Option<Arc<RecordingSender>>) -> ToolContext {
    let mut context = ToolContext::new(SessionId::new());
    context.event_context = Some(EventContext {
        input_message_id: Some(MessageId::new()),
        ..Default::default()
    });
    context.tool_call_id = Some("call-1".into());
    if let Some(sender) = sender {
        context
            .extensions
            .insert(Arc::new(ConversationSenderExt(sender)));
    }
    context
}

async fn send(args: Value, ctx: &ToolContext) -> ToolExecutionResult {
    SendMessageTool.execute_with_context(args, ctx).await
}

#[tokio::test]
async fn sends_exact_text_to_the_trusted_conversation_and_reports_delivery() {
    let sender = RecordingSender::new(Outcome::Delivered);
    let ctx = context(Some(sender.clone()));
    let text = "**Result**\n\nCould you check this? α 🎉\n";
    let ToolExecutionResult::Success(value) = send(json!({"text": text}), &ctx).await else {
        panic!("send failed");
    };
    assert_eq!(value["sent"], true);
    assert_eq!(
        value["delivery"],
        json!({"platform":"slack","channel":"C1","message_ref":"171.2"})
    );
    assert_eq!(
        *sender.sends.lock().unwrap(),
        vec![(
            text.into(),
            ctx.event_context.unwrap().input_message_id.unwrap(),
            "call-1".into()
        )]
    );
}

#[tokio::test]
async fn the_session_is_the_conversation_when_nothing_external_is_bound() {
    // No sender, a sender with no conversation for this input, and a turn with
    // no input at all (a schedule) all send into the session itself.
    let mut no_input = context(Some(RecordingSender::new(Outcome::Delivered)));
    no_input.event_context = None;
    for ctx in [
        context(None),
        context(Some(RecordingSender::new(Outcome::NotExternal))),
        no_input,
    ] {
        let ToolExecutionResult::Success(value) = send(json!({"text":"hi"}), &ctx).await else {
            panic!("send failed");
        };
        assert_eq!(value["sent"], true);
        assert!(value.get("delivery").is_none());
        assert!(value["message_id"].as_str().is_some());
    }
}

#[tokio::test]
async fn rejects_bad_text_and_model_chosen_destinations_before_sending() {
    let sender = RecordingSender::new(Outcome::Delivered);
    let ctx = context(Some(sender.clone()));
    for args in [
        json!({}),
        json!({"text":null}),
        json!({"text":" \n\u{2003}"}),
        json!({"text":"hi","channel":"another-channel"}),
        json!({"text":"hi","thread_ts":"another-thread"}),
        json!({"text":"α".repeat(MAX_MESSAGE_CHARS + 1)}),
    ] {
        assert!(matches!(
            send(args, &ctx).await,
            ToolExecutionResult::ToolError(_)
        ));
    }
    assert!(sender.sends.lock().unwrap().is_empty());
    assert!(matches!(
        send(json!({"text":"α".repeat(MAX_MESSAGE_CHARS)}), &ctx).await,
        ToolExecutionResult::Success(_)
    ));
}

#[tokio::test]
async fn a_refused_delivery_is_never_reported_as_sent() {
    let ctx = context(Some(RecordingSender::new(Outcome::Refused)));
    assert!(matches!(
        send(json!({"text":"hello"}), &ctx).await,
        ToolExecutionResult::ToolError(message) if message == "channel refused"
    ));
}

#[tokio::test]
async fn no_reply_sends_nothing_and_accepts_only_a_reason() {
    assert!(matches!(
        NoReplyTool.execute(json!({})).await,
        ToolExecutionResult::Success(value) if value == json!({"replied": false})
    ));
    assert!(matches!(
        NoReplyTool.execute(json!({"reason":"thanks only"})).await,
        ToolExecutionResult::Success(_)
    ));
    assert!(matches!(
        NoReplyTool.execute(json!({"text":"hi"})).await,
        ToolExecutionResult::ToolError(_)
    ));
}

#[test]
fn sent_message_reads_only_delivered_send_message_results() {
    let args = json!({"text":"Done."});
    let id = MessageId::new();
    let ok = json!({"sent":true,"message_id":id,"delivery":{"platform":"slack","channel":"C1","message_ref":"9"}});
    let sent = sent_message(SEND_MESSAGE_TOOL_NAME, "call-1", &args, Some(&ok)).unwrap();
    assert_eq!(sent.message_id, id);
    assert_eq!(sent.text, "Done.");
    assert_eq!(sent.tool_call_id, "call-1");
    assert_eq!(sent.delivery.unwrap().message_ref, "9");
    assert!(sent_message("other_tool", "c", &args, Some(&ok)).is_none());
    assert!(sent_message(SEND_MESSAGE_TOOL_NAME, "c", &args, None).is_none());
    assert!(
        sent_message(
            SEND_MESSAGE_TOOL_NAME,
            "c",
            &args,
            Some(&json!({"sent":false}))
        )
        .is_none()
    );
}

#[test]
fn explicit_mode_is_idempotent_and_keeps_agent_settings() {
    let agent = RuntimeAgent::new("You investigate deployments", "test-model");
    let first = apply_explicit_communication(agent.clone());
    let mut names: Vec<_> = first.tools.iter().map(|t| t.name().to_owned()).collect();
    names.sort();
    assert_eq!(names, ["no_reply", "send_message"]);
    assert!(first.system_prompt.ends_with(&agent.system_prompt));
    assert!(first.system_prompt.contains("private working notes"));
    assert_eq!(first.communication, Communication::Explicit);
    assert_eq!(
        serde_json::to_value(apply_explicit_communication(first.clone())).unwrap(),
        serde_json::to_value(first).unwrap()
    );
    assert_eq!(
        SendMessageTool.to_definition().side_effect_class(),
        crate::tool_types::SideEffectClass::AtMostOnce
    );
}

#[test]
fn narration_previews_the_message_in_each_phase_and_locale() {
    let call = everruns_contracts::tool_types::ToolCall {
        id: "send".into(),
        name: SEND_MESSAGE_TOOL_NAME.into(),
        arguments: json!({"text":"Here is a new joke.\n\nEnjoy!"}),
    };
    for (phase, locale, expected) in [
        (
            ToolNarrationPhase::Started,
            None,
            "Sending message: Here is a new joke. Enjoy!",
        ),
        (
            ToolNarrationPhase::Completed,
            None,
            "Sent message: Here is a new joke. Enjoy!",
        ),
        (
            ToolNarrationPhase::Failed,
            None,
            "Could not send message: Here is a new joke. Enjoy!",
        ),
        (
            ToolNarrationPhase::Completed,
            Some("uk-UA"),
            "Надіслав повідомлення: Here is a new joke. Enjoy!",
        ),
    ] {
        assert_eq!(
            SendMessageTool
                .narrate(&call, phase, locale, ToolNarrationContext::default())
                .as_deref(),
            Some(expected)
        );
    }
}
