use super::*;
use crate::runtime::{events::EventContext, typed_id::SessionId};
use std::sync::Mutex;

struct RecordingSender {
    sends: Mutex<Vec<(String, MessageId, String)>>,
    files: Mutex<Vec<OutgoingFile>>,
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
            files: Mutex::new(vec![]),
            outcome,
        })
    }
}

#[async_trait]
impl ConversationSender for RecordingSender {
    async fn send(
        &self,
        text: &str,
        files: &[OutgoingFile],
        input: MessageId,
        call: &str,
    ) -> Result<Option<ConversationDelivery>, ToolExecutionResult> {
        self.files.lock().unwrap().extend_from_slice(files);
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
    let agent = TurnAgent::new("You investigate deployments", "test-model");
    let first = apply_explicit_communication(agent.clone());
    let mut names: Vec<_> = first.tools.iter().map(|t| t.name().to_owned()).collect();
    names.sort();
    assert_eq!(names, ["no_reply", "send_message"]);
    assert!(first.system_prompt.ends_with(&agent.system_prompt));
    assert!(first.system_prompt.contains("private working notes"));
    assert!(
        first
            .system_prompt
            .contains("A step that only sends a message ends the turn.")
    );
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

/// Read-only session files for the `files` argument: path to (content, encoding).
struct Files(Vec<(&'static str, &'static str, &'static str)>);

#[async_trait]
impl crate::runtime::session_files::SessionFileSystem for Files {
    fn is_mount_resolver(&self) -> bool {
        false
    }
    async fn read_file(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> crate::runtime::error::Result<Option<crate::runtime::session_file::SessionFile>> {
        Ok(self
            .0
            .iter()
            .find(|(p, _, _)| *p == path || p.starts_with(&format!("{path}/")))
            .map(|(p, content, encoding)| {
                let is_directory = *p != path;
                crate::runtime::session_file::SessionFile {
                    id: uuid::Uuid::new_v4(),
                    session_id: session_id.uuid(),
                    path: path.into(),
                    name: path.rsplit('/').next().unwrap_or(path).into(),
                    content: (!is_directory).then(|| content.to_string()),
                    encoding: encoding.to_string(),
                    is_directory,
                    is_readonly: false,
                    size_bytes: content.len() as i64,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }
            }))
    }
    async fn write_file(
        &self,
        _: SessionId,
        _: &str,
        _: &str,
        _: &str,
    ) -> crate::runtime::error::Result<crate::runtime::session_file::SessionFile> {
        unreachable!("send_message never writes")
    }
    async fn delete_file(
        &self,
        _: SessionId,
        _: &str,
        _: bool,
    ) -> crate::runtime::error::Result<bool> {
        unreachable!("send_message never deletes")
    }
    async fn list_directory(
        &self,
        _: SessionId,
        _: &str,
    ) -> crate::runtime::error::Result<Vec<crate::runtime::session_file::FileInfo>> {
        unreachable!("send_message never lists")
    }
    async fn stat_file(
        &self,
        _: SessionId,
        _: &str,
    ) -> crate::runtime::error::Result<Option<crate::runtime::session_file::FileStat>> {
        unreachable!("send_message reads, it does not stat")
    }
    async fn grep_files(
        &self,
        _: SessionId,
        _: &str,
        _: Option<&str>,
    ) -> crate::runtime::error::Result<Vec<crate::runtime::session_file::GrepMatch>> {
        unreachable!("send_message never searches")
    }
    async fn create_directory(
        &self,
        _: SessionId,
        _: &str,
    ) -> crate::runtime::error::Result<crate::runtime::session_file::FileInfo> {
        unreachable!("send_message never creates directories")
    }
}

fn with_files(mut ctx: ToolContext) -> ToolContext {
    ctx.file_store = Some(Arc::new(Files(vec![
        ("/out/chart.png", "iVBORw==", "base64"),
        ("/out/notes.md", "# Notes", "utf-8"),
        ("/out/empty.txt", "", "utf-8"),
        ("/out/dir/inner.txt", "x", "utf-8"),
    ])));
    ctx
}

#[tokio::test]
async fn files_are_read_from_the_session_and_delivered_with_the_text() {
    let sender = RecordingSender::new(Outcome::Delivered);
    let ctx = with_files(context(Some(sender.clone())));
    let args = json!({"text":"Here you go", "files":["/out/chart.png", "/out/notes.md"]});
    let ToolExecutionResult::Success(value) = send(args.clone(), &ctx).await else {
        panic!("send failed");
    };
    assert_eq!(value["files"], json!(["/out/chart.png", "/out/notes.md"]));
    assert_eq!(
        *sender.files.lock().unwrap(),
        vec![
            OutgoingFile {
                path: "/out/chart.png".into(),
                filename: "chart.png".into(),
                content: b"\x89PNG".to_vec(),
            },
            OutgoingFile {
                path: "/out/notes.md".into(),
                filename: "notes.md".into(),
                content: b"# Notes".to_vec(),
            },
        ]
    );
    let sent = sent_message(SEND_MESSAGE_TOOL_NAME, "call-1", &args, Some(&value)).unwrap();
    assert_eq!(sent.files, vec!["/out/chart.png", "/out/notes.md"]);
}

#[tokio::test]
async fn files_are_recorded_when_the_session_is_the_conversation() {
    let ctx = with_files(context(None));
    let ToolExecutionResult::Success(value) =
        send(json!({"text":"Chart", "files":["/out/chart.png"]}), &ctx).await
    else {
        panic!("send failed");
    };
    assert_eq!(value["files"], json!(["/out/chart.png"]));
    assert!(value.get("delivery").is_none());
}

#[tokio::test]
async fn a_bad_file_sends_nothing() {
    let sender = RecordingSender::new(Outcome::Delivered);
    let too_many: Vec<String> = (0..=MAX_MESSAGE_FILES).map(|i| format!("/f{i}")).collect();
    for files in [
        json!(["/out/missing.png"]),
        json!(["/out/empty.txt"]),
        json!(["/out/dir"]),
        json!(["/out/chart.png", "/out/chart.png"]),
        json!([""]),
        json!(too_many),
    ] {
        let ctx = with_files(context(Some(sender.clone())));
        let result = send(json!({"text":"hi", "files": files}), &ctx).await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(_)),
            "{files}: {result:?}"
        );
    }
    // Without session storage, files cannot be read at all.
    let ctx = context(Some(sender.clone()));
    let result = send(json!({"text":"hi", "files":["/out/chart.png"]}), &ctx).await;
    assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    assert!(sender.sends.lock().unwrap().is_empty());
    assert!(sender.files.lock().unwrap().is_empty());
}

#[test]
fn files_are_omitted_from_a_text_only_message() {
    let ok = json!({"sent": true, "message_id": MessageId::new()});
    let sent = sent_message(
        SEND_MESSAGE_TOOL_NAME,
        "c",
        &json!({"text":"hi"}),
        Some(&ok),
    )
    .unwrap();
    assert!(sent.files.is_empty());
    let serialized = serde_json::to_value(&sent).unwrap();
    assert!(serialized.get("files").is_none(), "{serialized}");
}

#[tokio::test]
async fn message_is_the_field_and_text_still_reads_as_it() {
    let schema = SendMessageTool.parameters_schema();
    assert_eq!(schema["required"], json!(["message"]));
    assert!(schema["properties"]["message"].is_object());

    for args in [json!({"message": "hi"}), json!({"text": "hi"})] {
        let sender = RecordingSender::new(Outcome::Delivered);
        let ctx = context(Some(sender.clone()));
        let ToolExecutionResult::Success(result) = send(args.clone(), &ctx).await else {
            panic!("send failed for {args}");
        };
        assert_eq!(sender.sends.lock().unwrap()[0].0, "hi");
        let sent = sent_message(SEND_MESSAGE_TOOL_NAME, "call-1", &args, Some(&result)).unwrap();
        assert_eq!(sent.text, "hi");
    }
    // Both at once is ambiguous, so it is refused rather than guessed.
    let ctx = context(Some(RecordingSender::new(Outcome::Delivered)));
    assert!(matches!(
        send(json!({"message": "a", "text": "b"}), &ctx).await,
        ToolExecutionResult::ToolError(_)
    ));
    assert_eq!(send_message_text(&json!({"message": "m"})), Some("m"));
    assert_eq!(send_message_text(&json!({"text": "t"})), Some("t"));
}
