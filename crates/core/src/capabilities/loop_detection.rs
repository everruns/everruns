// Loop detection capability (EVE-227)
//
// Detects repeated identical tool calls and injects a warning to break the loop.
// Uses MessageFilterProvider::post_load to scan loaded messages for repeated
// tool-call batches. When N consecutive assistant messages carry the same
// tool-call signature, a system warning is appended telling the model to
// change its approach.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::capabilities::Capability;
use crate::message::{Message, MessageRole, ToolCallContentPart};
use crate::message_filter::{MessageFilterProvider, MessageQuery};

/// Default threshold: 3 consecutive identical tool call batches triggers warning.
const DEFAULT_THRESHOLD: usize = 3;

pub struct LoopDetectionCapability;

impl Capability for LoopDetectionCapability {
    fn id(&self) -> &str {
        "loop_detection"
    }

    fn name(&self) -> &str {
        "Tool Loop Detection"
    }

    fn description(&self) -> &str {
        "Detects repeated identical tool calls and injects a warning to break the loop."
    }

    fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
        Some(Arc::new(LoopDetectionFilter))
    }
}

struct LoopDetectionFilter;

impl MessageFilterProvider for LoopDetectionFilter {
    fn priority(&self) -> i32 {
        35
    }

    fn apply_filters(&self, _query: &mut MessageQuery, _config: &serde_json::Value) {
        // No query-time filters needed
    }

    fn post_load(&self, messages: &mut Vec<Message>, config: &serde_json::Value) {
        let threshold = config
            .get("threshold")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_THRESHOLD)
            .max(1); // Clamp to at least 1 to avoid indexing empty vec

        // Collect tool call signature hashes from recent agent messages (reverse order).
        let mut recent_hashes: Vec<u64> = Vec::new();
        for msg in messages.iter().rev() {
            if msg.role != MessageRole::Agent {
                continue;
            }
            let tool_calls = msg.tool_calls();
            if tool_calls.is_empty() {
                // Agent message without tool calls breaks the pattern
                break;
            }
            recent_hashes.push(hash_tool_calls(&tool_calls));
        }

        // recent_hashes is in reverse chronological order.
        // Check for `threshold` consecutive identical hashes.
        if recent_hashes.len() >= threshold {
            let target = recent_hashes[0];
            let consecutive = recent_hashes.iter().take_while(|&&h| h == target).count();
            if consecutive >= threshold {
                tracing::warn!(
                    consecutive,
                    threshold,
                    "Loop detected: identical tool calls repeated"
                );
                messages.push(Message::system(
                    "\u{26a0} Loop detected: you called the same tool(s) with identical arguments \
                     multiple times in a row. The approach is not working. \
                     Try a different command, different arguments, or report the blocker.",
                ));
            }
        }
    }
}

/// Hash a set of tool calls into a single u64 for comparison.
/// Tool calls are sorted by (name, arguments) so ordering doesn't matter.
fn hash_tool_calls(calls: &[&ToolCallContentPart]) -> u64 {
    let mut h = DefaultHasher::new();
    let mut sorted: Vec<_> = calls
        .iter()
        .map(|tc| (&tc.name, tc.arguments.to_string()))
        .collect();
    sorted.sort();
    for (name, args) in &sorted {
        name.hash(&mut h);
        args.hash(&mut h);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ContentPart, ToolCallContentPart};

    /// Helper: build an agent message with the given tool calls.
    fn agent_msg_with_calls(calls: Vec<(&str, serde_json::Value)>) -> Message {
        let content = calls
            .into_iter()
            .map(|(name, args)| {
                ContentPart::ToolCall(ToolCallContentPart::new(
                    uuid::Uuid::new_v4().to_string(),
                    name,
                    args,
                ))
            })
            .collect();
        Message {
            id: crate::typed_id::MessageId::new(),
            role: MessageRole::Agent,
            content,
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn default_config() -> serde_json::Value {
        serde_json::json!({})
    }

    #[test]
    fn test_no_loop_different_tool_calls() {
        let filter = LoopDetectionFilter;
        let mut messages = vec![
            Message::user("hello"),
            agent_msg_with_calls(vec![("tool_a", serde_json::json!({"x": 1}))]),
            Message::user("ok"),
            agent_msg_with_calls(vec![("tool_b", serde_json::json!({"x": 2}))]),
            Message::user("ok"),
            agent_msg_with_calls(vec![("tool_c", serde_json::json!({"x": 3}))]),
        ];
        let original_len = messages.len();
        filter.post_load(&mut messages, &default_config());
        // No warning should be injected
        assert_eq!(messages.len(), original_len);
    }

    #[test]
    fn test_loop_detected_three_identical_calls() {
        let filter = LoopDetectionFilter;
        let mut messages = vec![
            Message::user("do something"),
            agent_msg_with_calls(vec![("read_file", serde_json::json!({"path": "/foo"}))]),
            agent_msg_with_calls(vec![("read_file", serde_json::json!({"path": "/foo"}))]),
            agent_msg_with_calls(vec![("read_file", serde_json::json!({"path": "/foo"}))]),
        ];
        let original_len = messages.len();
        filter.post_load(&mut messages, &default_config());
        // Warning should be injected
        assert_eq!(messages.len(), original_len + 1);
        let last = messages.last().unwrap();
        assert_eq!(last.role, MessageRole::System);
        assert!(last.text().unwrap().contains("Loop detected"));
    }

    #[test]
    fn test_loop_broken_by_different_call() {
        let filter = LoopDetectionFilter;
        let mut messages = vec![
            Message::user("do something"),
            agent_msg_with_calls(vec![("read_file", serde_json::json!({"path": "/foo"}))]),
            agent_msg_with_calls(vec![("read_file", serde_json::json!({"path": "/foo"}))]),
            // Different call breaks the streak
            agent_msg_with_calls(vec![("write_file", serde_json::json!({"path": "/bar"}))]),
        ];
        let original_len = messages.len();
        filter.post_load(&mut messages, &default_config());
        // No warning
        assert_eq!(messages.len(), original_len);
    }

    #[test]
    fn test_configurable_threshold() {
        let filter = LoopDetectionFilter;
        let mut messages = vec![
            agent_msg_with_calls(vec![("tool_a", serde_json::json!({}))]),
            agent_msg_with_calls(vec![("tool_a", serde_json::json!({}))]),
        ];

        // Default threshold is 3, so 2 identical calls should NOT trigger
        let original_len = messages.len();
        filter.post_load(&mut messages, &default_config());
        assert_eq!(messages.len(), original_len);

        // With threshold = 2, it should trigger
        let config = serde_json::json!({"threshold": 2});
        filter.post_load(&mut messages, &config);
        assert_eq!(messages.len(), original_len + 1);
        assert!(
            messages
                .last()
                .unwrap()
                .text()
                .unwrap()
                .contains("Loop detected")
        );
    }

    #[test]
    fn test_hash_tool_calls_deterministic_sorted_args() {
        let tc1 = ToolCallContentPart::new("id1", "tool_a", serde_json::json!({"x": 1}));
        let tc2 = ToolCallContentPart::new("id2", "tool_b", serde_json::json!({"y": 2}));

        // Order should not matter due to sorting
        let h1 = hash_tool_calls(&[&tc1, &tc2]);
        let h2 = hash_tool_calls(&[&tc2, &tc1]);
        assert_eq!(h1, h2);

        // Different calls should produce different hashes
        let tc3 = ToolCallContentPart::new("id3", "tool_c", serde_json::json!({"z": 3}));
        let h3 = hash_tool_calls(&[&tc1, &tc3]);
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_loop_not_triggered_by_non_agent_messages() {
        let filter = LoopDetectionFilter;
        // Only user messages, no agent messages with tool calls
        let mut messages = vec![
            Message::user("hello"),
            Message::user("hello"),
            Message::user("hello"),
        ];
        let original_len = messages.len();
        filter.post_load(&mut messages, &default_config());
        assert_eq!(messages.len(), original_len);
    }

    #[test]
    fn test_capability_provides_filter() {
        let cap = LoopDetectionCapability;
        assert_eq!(cap.id(), "loop_detection");
        assert!(cap.message_filter_provider().is_some());
    }
}
