use super::*;
use crate::{events::EventContext, typed_id::SessionId};
use std::sync::Mutex;

struct RecordingSender {
    platform: &'static str,
    posts: Mutex<Vec<(String, MessageId, String)>>,
    fail: bool,
}

#[async_trait]
impl ChannelMessageSender for RecordingSender {
    async fn post_message(
        &self,
        text: &str,
        input: MessageId,
        call: &str,
    ) -> Result<ChannelMessageReceipt, ToolExecutionResult> {
        if self.fail {
            return Err(ToolExecutionResult::tool_error(
                "channel refused the message",
            ));
        }
        self.posts
            .lock()
            .unwrap()
            .push((text.into(), input, call.into()));
        Ok(ChannelMessageReceipt {
            platform: self.platform.into(),
            channel: "conversation-1".into(),
            message_ref: "message-2".into(),
        })
    }
}

fn context(sender: Arc<RecordingSender>) -> ToolContext {
    let mut context = ToolContext::new(SessionId::new());
    context.event_context = Some(EventContext {
        input_message_id: Some(MessageId::new()),
        ..Default::default()
    });
    context.tool_call_id = Some("call-1".into());
    context
        .extensions
        .insert(Arc::new(ChannelMessageSenderExt(sender)));
    context
}

#[tokio::test]
async fn posts_exact_content_and_returns_receipts_across_transports() {
    for platform in ["slack", "teams"] {
        let sender = Arc::new(RecordingSender {
            platform,
            posts: Mutex::new(vec![]),
            fail: false,
        });
        let ctx = context(sender.clone());
        let text = "**Result**\n\nCould you check this? α 🎉\n";
        let result = ChannelPostMessageTool
            .execute_with_context(json!({"text":text}), &ctx)
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("post failed");
        };
        assert_eq!(
            value,
            json!({"delivered":true,"platform":platform,"channel":"conversation-1","message_ref":"message-2"})
        );
        assert_eq!(
            *sender.posts.lock().unwrap(),
            vec![(
                text.into(),
                ctx.event_context.unwrap().input_message_id.unwrap(),
                "call-1".into()
            )]
        );
    }
}

#[tokio::test]
async fn rejects_invalid_content_and_model_selected_destinations_before_posting() {
    let sender = Arc::new(RecordingSender {
        platform: "slack",
        posts: Mutex::new(vec![]),
        fail: false,
    });
    let ctx = context(sender.clone());
    for args in [
        json!({}),
        json!({"text":null}),
        json!({"text":" \n\u{2003}"}),
        json!({"text":"hi","channel":"another-channel"}),
        json!({"text":"hi","thread_ts":"another-thread"}),
        json!({"text":"α".repeat(MAX_CHANNEL_MESSAGE_CHARS+1)}),
    ] {
        assert!(matches!(
            ChannelPostMessageTool
                .execute_with_context(args, &ctx)
                .await,
            ToolExecutionResult::ToolError(_)
        ));
    }
    assert!(sender.posts.lock().unwrap().is_empty());
    assert!(matches!(
        ChannelPostMessageTool
            .execute_with_context(json!({"text":"α".repeat(MAX_CHANNEL_MESSAGE_CHARS)}), &ctx)
            .await,
        ToolExecutionResult::Success(_)
    ));
}

#[tokio::test]
async fn delivery_errors_and_missing_invocation_authority_never_return_success() {
    let sender = Arc::new(RecordingSender {
        platform: "slack",
        posts: Mutex::new(vec![]),
        fail: true,
    });
    let ctx = context(sender);
    assert!(
        matches!(ChannelPostMessageTool.execute_with_context(json!({"text":"hello"}), &ctx).await,
        ToolExecutionResult::ToolError(message) if message == "channel refused the message")
    );
    let sender = Arc::new(RecordingSender {
        platform: "slack",
        posts: Mutex::new(vec![]),
        fail: false,
    });
    let mut ctx = context(sender.clone());
    ctx.event_context = None;
    assert!(matches!(
        ChannelPostMessageTool
            .execute_with_context(json!({"text":"hello"}), &ctx)
            .await,
        ToolExecutionResult::ToolError(_)
    ));
    ctx = context(sender.clone());
    ctx.tool_call_id = None;
    assert!(matches!(
        ChannelPostMessageTool
            .execute_with_context(json!({"text":"hello"}), &ctx)
            .await,
        ToolExecutionResult::ToolError(_)
    ));
    assert!(sender.posts.lock().unwrap().is_empty());
    assert!(matches!(
        ChannelPostMessageTool
            .execute_with_context(json!({"text":"hello"}), &ToolContext::new(SessionId::new()))
            .await,
        ToolExecutionResult::ToolError(_)
    ));
}

#[test]
fn legacy_modes_normalize_and_unrelated_tags_survive() {
    let mut tags = vec![
        "custom:keep".into(),
        "slack:reply_mode:report_progress_only".into(),
        "channel:reply_mode:report_progress_only".into(),
    ];
    assert!(session_uses_channel_tools(&tags));
    sync_slack_reply_mode_tags(&mut tags, ChannelReplyMode::ToolOnly);
    assert_eq!(
        tags,
        [
            "custom:keep",
            "slack:reply_mode:tool_only",
            "channel:reply_mode:tool_only"
        ]
    );
    sync_slack_reply_mode_tags(&mut tags, ChannelReplyMode::ToolOnly);
    assert_eq!(tags.len(), 3);
    sync_slack_reply_mode_tags(&mut tags, ChannelReplyMode::AllMessages);
    assert_eq!(tags, ["custom:keep"]);
    for tag in [
        CHANNEL_TOOL_ONLY_TAG,
        SLACK_TOOL_ONLY_TAG,
        "channel:reply_mode:report_progress_only",
        "slack:reply_mode:report_progress_only",
    ] {
        assert!(session_uses_channel_tools(&[tag.into()]));
        assert!(!session_uses_channel_tools(&[format!("{tag}_extra")]));
    }
    let mode: ChannelReplyMode = serde_json::from_str("\"report_progress_only\"").unwrap();
    assert_eq!(mode, ChannelReplyMode::ToolOnly);
    assert_eq!(serde_json::to_string(&mode).unwrap(), "\"tool_only\"");
}

#[test]
fn communication_instructions_are_idempotent_and_preserve_agent_settings() {
    let mut agent = RuntimeAgent::new("You investigate deployments", "test-model");
    agent.system_prompt = "You investigate deployments".into();
    let original = agent.clone();
    let first = apply_channel_message_mode(agent);
    assert_eq!(
        first.tools.iter().map(|t| t.name()).collect::<Vec<_>>(),
        ["channel_post_message"]
    );
    assert!(first.system_prompt.ends_with(&original.system_prompt));
    assert!(first.system_prompt.contains("questions, and final answers"));
    assert_eq!(
        serde_json::to_value(apply_channel_message_mode(first.clone())).unwrap(),
        serde_json::to_value(first).unwrap()
    );
    assert_eq!(
        ChannelPostMessageTool.to_definition().side_effect_class(),
        everruns_contracts::tool_types::SideEffectClass::AtMostOnce
    );
}
