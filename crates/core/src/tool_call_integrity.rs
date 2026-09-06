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
pub fn retain_complete_llm_tool_exchanges(messages: Vec<LlmMessage>) -> Vec<LlmMessage> {
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
        // Reasoning is never user-visible content on its own: a message
        // carrying only reasoning has nothing to show and nothing to act on.
        ContentPart::Reasoning(_) => false,
    })
}

fn llm_message_has_visible_content(message: &LlmMessage) -> bool {
    let has_content = match &message.content {
        LlmMessageContent::Text(text) => !text.is_empty(),
        LlmMessageContent::Parts(parts) => !parts.is_empty(),
    };
    has_content || message.tool_calls.is_some() || !message.reasoning.is_empty()
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
            reasoning: Vec::new(),
        }
    }

    fn tool_result(id: &str) -> LlmMessage {
        LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Text("result".to_string()),
            tool_calls: None,
            tool_call_id: Some(id.to_string()),
            phase: None,
            reasoning: Vec::new(),
        }
    }

    fn assert_text_messages(actual: &[LlmMessage], expected: &[LlmMessage]) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.role, expected.role);
            match (&actual.content, &expected.content) {
                (LlmMessageContent::Text(actual), LlmMessageContent::Text(expected)) => {
                    assert_eq!(actual, expected);
                }
                _ => panic!("expected text messages"),
            }
            assert_eq!(
                serde_json::to_value(&actual.tool_calls).unwrap(),
                serde_json::to_value(&expected.tool_calls).unwrap()
            );
            assert_eq!(actual.tool_call_id, expected.tool_call_id);
            assert_eq!(actual.phase, expected.phase);
            assert_eq!(actual.reasoning, expected.reasoning);
        }
    }

    #[test]
    fn llm_reduction_prunes_only_the_unmatched_parallel_call() {
        let mut batch = assistant_batch();
        batch.content = LlmMessageContent::Text("Keep this text".into());
        batch.phase = Some(everruns_provider::execution_phase::ExecutionPhase::Commentary);
        batch
            .reasoning
            .push(everruns_provider::reasoning::ReasoningContentPart::opaque(
                "test-provider",
            ));
        batch.tool_calls.as_mut().unwrap()[0].arguments = json!({"name":"ops"});
        let result = tool_result("call_skill");
        let reduced = retain_complete_llm_tool_exchanges(vec![
            batch.clone(),
            result.clone(),
            tool_result("orphan"),
        ]);
        batch.tool_calls.as_mut().unwrap().truncate(1);
        assert_text_messages(&reduced, &[batch, result]);
    }

    #[test]
    fn stateless_and_stateful_llm_reduction_handle_orphans_without_inventing_calls() {
        let result = tool_result("call_bash");
        assert!(retain_complete_llm_tool_exchanges(vec![result.clone()]).is_empty());
        assert_text_messages(
            &retain_complete_llm_tool_exchanges_for_request(vec![result.clone()], true),
            &[result],
        );
        for allow in [false, true] {
            let mut invalid = tool_result("ignored");
            invalid.tool_call_id = None;
            assert!(
                retain_complete_llm_tool_exchanges_for_request(vec![invalid], allow).is_empty()
            );
            assert!(
                retain_complete_llm_tool_exchanges_for_request(vec![assistant_batch()], allow)
                    .is_empty()
            );
        }
    }

    #[test]
    fn removing_all_llm_calls_preserves_independent_visible_text() {
        let mut batch = assistant_batch();
        batch.content = LlmMessageContent::Text("Keep α".into());
        let mut expected = batch.clone();
        expected.tool_calls = None;
        assert_text_messages(
            &retain_complete_llm_tool_exchanges(vec![batch]),
            &[expected],
        );
    }

    #[test]
    fn message_reduction_allows_stateful_result_deltas_only_when_requested() {
        let result = Message::tool_result(
            "call_bash",
            Some(json!({"output":"done","exit_code":0})),
            None,
        );
        let before = serde_json::to_value(&result).unwrap();
        assert!(
            retain_complete_message_tool_exchanges(std::slice::from_ref(&result), false).is_empty()
        );
        assert_eq!(
            serde_json::to_value(retain_complete_message_tool_exchanges(
                std::slice::from_ref(&result),
                true
            ))
            .unwrap(),
            json!([before])
        );
        assert_eq!(serde_json::to_value(&result).unwrap(), before);
    }

    #[test]
    fn message_reduction_preserves_matched_parts_and_does_not_mutate_source_history() {
        let calls = assistant_batch().tool_calls.unwrap();
        let batch = Message::assistant_with_tools("Visible α", calls);
        let matched = Message::tool_result(
            "call_skill",
            Some(json!({"skill":"ops","instructions":"Body"})),
            None,
        );
        let orphan = Message::tool_result("orphan", Some(json!("orphan result")), None);
        let source = vec![
            Message::user("Question"),
            batch.clone(),
            matched.clone(),
            orphan.clone(),
        ];
        let before = serde_json::to_value(&source).unwrap();
        let mut expected_batch = batch;
        expected_batch
            .content
            .retain(|part| !matches!(part,ContentPart::ToolCall(call) if call.id=="call_bash"));
        for allow in [false, true] {
            let mut expected = vec![source[0].clone(), expected_batch.clone(), matched.clone()];
            if allow {
                expected.push(orphan.clone());
            }
            assert_eq!(
                serde_json::to_value(retain_complete_message_tool_exchanges(&source, allow))
                    .unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
        assert_eq!(serde_json::to_value(&source).unwrap(), before);
        let orphan_call = Message::assistant_with_tools(
            "",
            vec![ToolCall {
                id: "missing".into(),
                name: "run".into(),
                arguments: json!({}),
            }],
        );
        assert!(retain_complete_message_tool_exchanges(&[orphan_call], true).is_empty());
    }
}
