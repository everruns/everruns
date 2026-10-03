//! The native OpenAI `computer` tool on the Open Responses driver (EVE-1133).
//!
//! Request side: when the call asks for native computer use
//! ([`NativeComputerUse`]) on an endpoint with OpenAI's hosted tools and a model
//! that has the GA `computer` tool, the `computer` function tool is left out and
//! `{"type": "computer"}` goes in its place (through
//! [`crate::openai_hosted_tools::OpenAiHostedTools::computer`]).
//!
//! Response side: a `computer_call` item becomes a call of the `computer` tool
//! whose arguments are the call's actions in the neutral vocabulary. It is a
//! client tool call like any function call, so the agent loop executes it,
//! gates it and budgets it. It never becomes a hosted-call event.
//!
//! Replay: calls of the `computer` tool go back as `computer_call` items and
//! their results as `computer_call_output` items carrying the screenshot. Wire
//! details live in [`crate::openai_computer`].

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::driver_registry::LlmCallConfig;
use crate::native_computer::{COMPUTER_TOOL_NAME, NativeComputerUse, openai_has_native_computer};
use crate::openai_computer::{
    BLANK_SCREENSHOT, PENDING_SAFETY_CHECKS_KEY, replay_call, replay_output,
};

use super::OpenResponsesProtocolChatDriver;
use super::wire::{ResponsesContent, ResponsesContentPart, ResponsesInputItem};

impl OpenResponsesProtocolChatDriver {
    /// The native computer tool for this call, when the endpoint, the model and
    /// the call all allow it.
    pub(crate) fn native_computer_for(&self, config: &LlmCallConfig) -> Option<NativeComputerUse> {
        if !self.hosted_tools || !openai_has_native_computer(&config.model) {
            return None;
        }
        NativeComputerUse::requested(config)
    }
}

/// Ids of every `computer` tool call in the transcript.
pub(crate) fn computer_call_ids(items: &[ResponsesInputItem]) -> HashSet<String> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponsesInputItem::FunctionCall { call_id, name, .. }
                if name == COMPUTER_TOOL_NAME =>
            {
                Some(call_id.clone())
            }
            _ => None,
        })
        .collect()
}

fn last_image(output: &ResponsesContent) -> Option<&str> {
    match output {
        ResponsesContent::Parts(parts) => parts.iter().rev().find_map(|part| match part {
            ResponsesContentPart::InputImage { image_url, .. } => Some(image_url.as_str()),
            _ => None,
        }),
        ResponsesContent::Text(_) => None,
    }
}

fn output_text(output: &ResponsesContent) -> String {
    match output {
        ResponsesContent::Text(text) => text.clone(),
        ResponsesContent::Parts(parts) => parts
            .iter()
            .filter_map(|part| match part {
                ResponsesContentPart::InputText { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Rewrite calls of the `computer` tool named in `ids`, and their outputs, into
/// OpenAI's computer items. Runs on the final request input, after delta
/// trimming and pair repair, which only know function items.
///
/// `computer_call_output` must carry a screenshot and has no error field. A call
/// that did not run (an approval wait, a budget stop, a bad action) is answered
/// with the newest screenshot seen so far, or a blank frame, and the error
/// follows as a user message so the model reads why.
pub(crate) fn replay_computer_calls(
    items: Vec<ResponsesInputItem>,
    ids: &HashSet<String>,
) -> Vec<ResponsesInputItem> {
    if ids.is_empty() {
        return items;
    }
    let mut safety_checks: HashMap<String, Value> = HashMap::new();
    let mut last_screenshot: Option<String> = None;
    let mut replayed = Vec::with_capacity(items.len());
    for item in items {
        match item {
            ResponsesInputItem::FunctionCall {
                call_id, arguments, ..
            } if ids.contains(&call_id) => {
                let arguments: Value = serde_json::from_str(&arguments).unwrap_or_default();
                if let Some(checks) = arguments.get(PENDING_SAFETY_CHECKS_KEY) {
                    safety_checks.insert(call_id.clone(), checks.clone());
                }
                replayed.push(ResponsesInputItem::ProviderItem(replay_call(
                    &call_id, &arguments,
                )));
            }
            ResponsesInputItem::FunctionCallOutput {
                call_id, output, ..
            } if ids.contains(&call_id) => match last_image(&output) {
                Some(image) => {
                    last_screenshot = Some(image.to_string());
                    // The call ran, so its safety checks were approved with it.
                    replayed.push(ResponsesInputItem::ProviderItem(replay_output(
                        &call_id,
                        image,
                        safety_checks.get(&call_id),
                    )));
                }
                None => {
                    let frame = last_screenshot.as_deref().unwrap_or(BLANK_SCREENSHOT);
                    replayed.push(ResponsesInputItem::ProviderItem(replay_output(
                        &call_id, frame, None,
                    )));
                    replayed.push(ResponsesInputItem::Message {
                        r#type: "message".to_string(),
                        role: "user".to_string(),
                        content: ResponsesContent::Text(format!(
                            "Computer call {call_id} returned no new screenshot (the frame above \
                             is older). Result: {}",
                            output_text(&output)
                        )),
                        phase: None,
                    });
                }
            },
            other => replayed.push(other),
        }
    }
    replayed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str, name: &str, arguments: Value) -> ResponsesInputItem {
        ResponsesInputItem::FunctionCall {
            r#type: "function_call".into(),
            call_id: id.into(),
            name: name.into(),
            arguments: arguments.to_string(),
        }
    }

    fn output(id: &str, output: ResponsesContent) -> ResponsesInputItem {
        ResponsesInputItem::FunctionCallOutput {
            r#type: "function_call_output".into(),
            call_id: id.into(),
            output,
        }
    }

    fn screenshot(url: &str) -> ResponsesContent {
        ResponsesContent::Parts(vec![
            ResponsesContentPart::InputText {
                r#type: "input_text".into(),
                text: "{\"status\":\"ok\"}".into(),
            },
            ResponsesContentPart::InputImage {
                r#type: "input_image".into(),
                image_url: url.into(),
            },
        ])
    }

    fn wire(items: &[ResponsesInputItem]) -> Vec<Value> {
        items
            .iter()
            .map(|i| serde_json::to_value(i).unwrap())
            .collect()
    }

    #[test]
    fn computer_calls_replay_as_computer_items_and_others_are_untouched() {
        let items = vec![
            call(
                "c1",
                "computer",
                json!({ "actions": [{ "action": "screenshot" }] }),
            ),
            output("c1", screenshot("data:image/png;base64,ONE")),
            call("f1", "web_fetch", json!({ "url": "https://example.com" })),
            output("f1", ResponsesContent::Text("ok".into())),
        ];
        let ids = computer_call_ids(&items);
        assert_eq!(ids, HashSet::from(["c1".to_string()]));
        let wire = wire(&replay_computer_calls(items, &ids));
        assert_eq!(wire[0]["type"], "computer_call");
        assert_eq!(wire[0]["actions"], json!([{ "type": "screenshot" }]));
        assert_eq!(wire[1]["type"], "computer_call_output");
        assert_eq!(wire[1]["output"]["image_url"], "data:image/png;base64,ONE");
        assert_eq!(wire[2]["type"], "function_call");
        assert_eq!(wire[3]["type"], "function_call_output");
    }

    #[test]
    fn a_call_without_a_screenshot_reuses_the_last_frame_and_explains() {
        let items = vec![
            call(
                "c1",
                "computer",
                json!({ "actions": [{ "action": "screenshot" }] }),
            ),
            output("c1", screenshot("data:image/png;base64,ONE")),
            call(
                "c2",
                "computer",
                json!({ "actions": [{ "action": "type", "text": "x" }] }),
            ),
            output(
                "c2",
                ResponsesContent::Text("needs a person's approval".into()),
            ),
        ];
        let ids = computer_call_ids(&items);
        let wire = wire(&replay_computer_calls(items, &ids));
        assert_eq!(wire.len(), 5);
        assert_eq!(wire[3]["output"]["image_url"], "data:image/png;base64,ONE");
        assert_eq!(wire[4]["role"], "user");
        assert!(
            wire[4]["content"]
                .as_str()
                .unwrap()
                .contains("needs a person's approval")
        );
    }

    #[test]
    fn a_first_failed_call_answers_with_a_blank_frame() {
        let items = vec![
            call(
                "c1",
                "computer",
                json!({ "actions": [{ "action": "type", "text": "x" }] }),
            ),
            output("c1", ResponsesContent::Text("budget exhausted".into())),
        ];
        let ids = computer_call_ids(&items);
        let wire = wire(&replay_computer_calls(items, &ids));
        assert_eq!(wire[1]["output"]["image_url"], BLANK_SCREENSHOT);
    }

    #[test]
    fn safety_checks_are_acknowledged_only_for_a_call_that_ran() {
        let checks = json!([{ "id": "sc_1", "code": "malicious_instructions" }]);
        let items = vec![
            call(
                "c1",
                "computer",
                json!({ "actions": [{ "action": "screenshot" }], "pending_safety_checks": checks }),
            ),
            output("c1", screenshot("data:image/png;base64,ONE")),
            call(
                "c2",
                "computer",
                json!({ "actions": [{ "action": "screenshot" }], "pending_safety_checks": checks }),
            ),
            output("c2", ResponsesContent::Text("rejected".into())),
        ];
        let ids = computer_call_ids(&items);
        let wire = wire(&replay_computer_calls(items, &ids));
        assert_eq!(wire[1]["acknowledged_safety_checks"], checks);
        assert!(wire[3].get("acknowledged_safety_checks").is_none());
    }
}
