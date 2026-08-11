//! Structural integrity helpers for reduced conversation histories.
//!
//! Session storage stays lossless. Reducers use these helpers only on prompt-facing
//! copies so an assistant tool call is never exposed without its result, and a
//! stateless request never exposes a result without its call.

use crate::driver_registry::{LlmMessage, LlmMessageContent, LlmMessageRole};
use crate::message::{ContentPart, Message};
use std::collections::HashSet;

/// Keep only complete tool-call/result exchanges in a prompt-facing `Message` view.
///
/// Stateful Responses requests may legitimately carry a result whose call lives in
/// `previous_response_id`; `allow_unmatched_results` preserves that delta shape.
/// Calls without visible results are always removed rather than synthesized.
pub fn retain_complete_message_tool_exchanges(
    messages: &[Message],
    allow_unmatched_results: bool,
) -> Vec<Message> {
    let result_ids: HashSet<String> = messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|part| match part {
            ContentPart::ToolResult(result) => Some(result.tool_call_id.clone()),
            _ => None,
        })
        .collect();

    let calls_filtered: Vec<Message> = messages
        .iter()
        .filter_map(|message| {
            let mut message = message.clone();
            let had_calls = message
                .content
                .iter()
                .any(|part| matches!(part, ContentPart::ToolCall(_)));
            message.content.retain(|part| match part {
                ContentPart::ToolCall(call) => result_ids.contains(call.id.as_str()),
                _ => true,
            });
            (!had_calls || message_has_visible_content(&message)).then_some(message)
        })
        .collect();

    let call_ids: HashSet<String> = calls_filtered
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|part| match part {
            ContentPart::ToolCall(call) => Some(call.id.clone()),
            _ => None,
        })
        .collect();

    calls_filtered
        .iter()
        .filter_map(|message| {
            let mut message = message.clone();
            let had_results = message
                .content
                .iter()
                .any(|part| matches!(part, ContentPart::ToolResult(_)));
            message.content.retain(|part| match part {
                ContentPart::ToolResult(result) => {
                    call_ids.contains(result.tool_call_id.as_str()) || allow_unmatched_results
                }
                _ => true,
            });
            (!had_results || message_has_visible_content(&message)).then_some(message)
        })
        .collect()
}

/// Keep only complete tool-call/result exchanges in a reduced `LlmMessage` view.
///
/// This is strict because compaction operates on a self-contained transcript; it
/// never synthesizes execution results and can only reduce the selected output.
pub(crate) fn retain_complete_llm_tool_exchanges(messages: Vec<LlmMessage>) -> Vec<LlmMessage> {
    retain_complete_llm_tool_exchanges_for_request(messages, false)
}

/// Enforce tool exchange integrity at the final runtime boundary.
///
/// `allow_unmatched_results` is reserved for stateful Responses continuations,
/// where the matching call is stored behind `previous_response_id`.
pub fn retain_complete_llm_tool_exchanges_for_request(
    messages: Vec<LlmMessage>,
    allow_unmatched_results: bool,
) -> Vec<LlmMessage> {
    let result_ids: HashSet<String> = messages
        .iter()
        .filter(|message| message.role == LlmMessageRole::Tool)
        .filter_map(|message| message.tool_call_id.clone())
        .collect();

    let calls_filtered: Vec<LlmMessage> = messages
        .into_iter()
        .filter_map(|mut message| {
            let had_calls = message.tool_calls.is_some();
            if let Some(calls) = &mut message.tool_calls {
                calls.retain(|call| result_ids.contains(call.id.as_str()));
                if calls.is_empty() {
                    message.tool_calls = None;
                }
            }
            (!had_calls || llm_message_has_visible_content(&message)).then_some(message)
        })
        .collect();

    let call_ids: HashSet<String> = calls_filtered
        .iter()
        .flat_map(|message| message.tool_calls.iter().flatten())
        .map(|call| call.id.clone())
        .collect();

    calls_filtered
        .into_iter()
        .filter(|message| {
            message.role != LlmMessageRole::Tool
                || message
                    .tool_call_id
                    .as_deref()
                    .is_some_and(|id| call_ids.contains(id) || allow_unmatched_results)
        })
        .collect()
}

fn message_has_visible_content(message: &Message) -> bool {
    message.content.iter().any(|part| match part {
        ContentPart::Text(text) => !text.text.is_empty(),
        ContentPart::Image(_) | ContentPart::ImageFile(_) => true,
        ContentPart::ToolCall(_) | ContentPart::ToolResult(_) => true,
    })
}

fn llm_message_has_visible_content(message: &LlmMessage) -> bool {
    let has_content = match &message.content {
        LlmMessageContent::Text(text) => !text.is_empty(),
        LlmMessageContent::Parts(parts) => !parts.is_empty(),
    };
    has_content
        || message.tool_calls.is_some()
        || message.thinking.is_some()
        || message.thinking_signature.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver_registry::{LlmMessageContent, LlmMessageRole};
    use crate::tool_types::ToolCall;
    use serde_json::json;

    fn assistant_batch() -> LlmMessage {
        LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text(String::new()),
            tool_calls: Some(vec![
                ToolCall {
                    id: "call_skill".to_string(),
                    name: "activate_skill".to_string(),
                    arguments: json!({}),
                },
                ToolCall {
                    id: "call_bash".to_string(),
                    name: "bash".to_string(),
                    arguments: json!({}),
                },
            ]),
            tool_call_id: None,
            phase: None,
            thinking: None,
            thinking_signature: None,
        }
    }

    fn tool_result(id: &str) -> LlmMessage {
        LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Text("result".to_string()),
            tool_calls: None,
            tool_call_id: Some(id.to_string()),
            phase: None,
            thinking: None,
            thinking_signature: None,
        }
    }

    #[test]
    fn llm_reduction_prunes_only_the_unmatched_parallel_call() {
        let reduced =
            retain_complete_llm_tool_exchanges(vec![assistant_batch(), tool_result("call_skill")]);

        let calls = reduced[0].tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_skill");
        assert_eq!(reduced[1].tool_call_id.as_deref(), Some("call_skill"));
    }

    #[test]
    fn llm_reduction_drops_a_result_without_a_visible_call() {
        let reduced = retain_complete_llm_tool_exchanges(vec![tool_result("call_bash")]);
        assert!(reduced.is_empty());
    }

    #[test]
    fn stateful_llm_request_keeps_a_result_delta_without_reintroducing_calls() {
        let reduced =
            retain_complete_llm_tool_exchanges_for_request(vec![tool_result("call_bash")], true);
        assert_eq!(reduced.len(), 1);
        assert_eq!(reduced[0].tool_call_id.as_deref(), Some("call_bash"));
    }

    #[test]
    fn message_reduction_allows_stateful_result_deltas_only_when_requested() {
        let result = Message::tool_result("call_bash", Some(json!("done")), None);

        assert!(
            retain_complete_message_tool_exchanges(std::slice::from_ref(&result), false).is_empty()
        );
        assert_eq!(
            retain_complete_message_tool_exchanges(&[result], true).len(),
            1
        );
    }
}
