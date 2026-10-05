use everruns_capabilities::capabilities::SlackCapability;
use everruns_contracts::tool_types::ToolCall;
use everruns_core::capabilities::Capability;
use everruns_core::tool_narration::{ToolNarrationContext, ToolNarrationPhase};
use serde_json::json;

#[test]
fn slack_narration_shows_the_action_and_safe_detail_in_every_phase() {
    for tool in SlackCapability.tools() {
        let (arguments, expected) = match tool.name() {
            "slack_add_reaction" => (json!({"name":"thumbsup"}), "Added Slack reaction: thumbsup"),
            "slack_update_message" => (
                json!({"text":"The report\nis ready."}),
                "Updated Slack message: The report is ready.",
            ),
            "slack_lookup_user" => (json!({"user_id":"U123"}), "Looked up Slack user: U123"),
            "slack_upload_file" => (
                json!({"filename":"/workspace/report.md", "content":"PRIVATE_BODY", "initial_comment":"PRIVATE_BODY"}),
                "Uploaded file to Slack: report.md",
            ),
            name => panic!("add a narration case for {name}"),
        };
        let call = ToolCall {
            id: "call".into(),
            name: tool.name().into(),
            arguments,
        };
        for locale in [None, Some("uk-UA")] {
            let mut lines = Vec::new();
            for phase in [
                ToolNarrationPhase::Started,
                ToolNarrationPhase::Waiting,
                ToolNarrationPhase::Completed,
                ToolNarrationPhase::Failed,
            ] {
                let line = tool
                    .narrate(&call, phase, locale, ToolNarrationContext::default())
                    .unwrap_or_else(|| panic!("Slack tool must narrate"));
                assert!(!line.contains("PRIVATE_BODY"));
                lines.push(line);
            }
            assert_eq!(lines[0], lines[1]);
            assert_ne!(lines[0], lines[2]);
            assert_ne!(lines[2], lines[3]);
            if locale.is_none() {
                assert_eq!(lines[2], expected);
            } else {
                assert_ne!(lines[2], expected);
            }
            let empty = ToolCall {
                arguments: json!({"api_key":"PRIVATE_BODY"}),
                ..call.clone()
            };
            let line = tool
                .narrate(
                    &empty,
                    ToolNarrationPhase::Completed,
                    locale,
                    ToolNarrationContext::default(),
                )
                .unwrap_or_else(|| panic!("Slack tool must narrate"));
            assert!(!line.contains(':'));
            assert!(!line.contains("PRIVATE_BODY"));
        }
    }
}
