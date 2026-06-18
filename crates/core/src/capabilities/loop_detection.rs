// Loop detection capability (EVE-227)
//
// Detects repeated identical tool calls and injects a warning to break the loop.
// Uses MessageFilterProvider::post_load to scan loaded messages for repeated
// tool-call batches. When N consecutive assistant messages carry the same
// tool-call signature, a system warning is appended telling the model to
// change its approach.

use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::capabilities::{Capability, CapabilityLocalization};
use crate::message::{Message, MessageRole, ToolCallContentPart};
use crate::message_filter::{MessageFilterProvider, MessageQuery};
use crate::tool_fingerprint::tool_call_parts_fingerprint;

/// Default threshold: 3 repeated attempts triggers warning.
const DEFAULT_THRESHOLD: usize = 3;

pub const LOOP_DETECTION_CAPABILITY_ID: &str = "loop_detection";

pub struct LoopDetectionCapability;

impl Capability for LoopDetectionCapability {
    fn id(&self) -> &str {
        LOOP_DETECTION_CAPABILITY_ID
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

    /// `threshold` is the only knob this capability reads from config (see
    /// `post_load`), so it is the only exposed field.
    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "threshold": {
                    "type": "integer",
                    "title": "Repetition threshold",
                    "description": "Number of repeated identical tool-call batches, tool results, or read ranges that triggers the loop warning.",
                    "minimum": 1,
                    "default": DEFAULT_THRESHOLD
                }
            }
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        if !config.is_object() {
            return Err("loop_detection config must be an object".to_string());
        }
        match config.get("threshold") {
            None => Ok(()),
            // `post_load` clamps the threshold to >= 1; reject values that
            // would be silently clamped or are not unsigned integers.
            Some(value) => match value.as_u64() {
                Some(threshold) if threshold >= 1 => Ok(()),
                _ => Err(format!(
                    "threshold must be a positive integer (>= 1), got {value}"
                )),
            },
        }
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Controls how many repeated identical tool-call batches, tool results, or read ranges count as a loop.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Виявлення циклів інструментів"),
                description: Some(
                    "Виявляє повторювані однакові виклики інструментів і додає попередження, \
                     щоб розірвати цикл.",
                ),
                config_description: Some(
                    "Визначає, скільки однакових послідовних викликів інструментів вважається циклом.",
                ),
                config_overlay: Some(serde_json::json!({
                    "properties": {
                        "threshold": {
                            "title": "Поріг повторень",
                            "description": "Кількість повторюваних однакових викликів інструментів, результатів або діапазонів читання, після якої додається попередження про цикл."
                        }
                    }
                })),
            },
        ]
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

        if let Some(consecutive) = repeated_tool_result_count(messages, threshold) {
            tracing::warn!(
                consecutive,
                threshold,
                "Loop detected: identical tool call/result pairs repeated"
            );
            messages.push(Message::system(
                "Loop detected: the same tool call produced the same result repeatedly. \
                 The approach is not making progress. Try different arguments, inspect a \
                 new source of context, change state before retrying, or report the blocker.",
            ));
            return;
        }

        if let Some(repetition) = repeated_read_range_count(messages, threshold) {
            tracing::warn!(
                tool_name = repetition.tool_name,
                path = repetition.path,
                repeated_range_count = repetition.repeated_range_count,
                total_recent_reads = repetition.total_recent_reads,
                threshold,
                "Loop detected: read tool repeatedly requested the same range"
            );
            messages.push(Message::system(
                "Loop detected: you are repeatedly reading the same file or output range. \
                 Use the content already returned, read a different range once, change approach, \
                 or report the blocker.",
            ));
            return;
        }

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
    let mut sorted: Vec<_> = calls
        .iter()
        .map(|tc| tool_call_parts_fingerprint(&tc.name, &tc.arguments))
        .collect();
    sorted.sort();
    let mut h = DefaultHasher::new();
    sorted.hash(&mut h);
    h.finish()
}

fn repeated_tool_result_count(messages: &[Message], threshold: usize) -> Option<usize> {
    let mut target: Option<String> = None;
    let mut consecutive = 0;

    for msg in messages.iter().rev() {
        if msg.role == MessageRole::User || msg.role == MessageRole::System {
            break;
        }
        if msg.role != MessageRole::ToolResult {
            continue;
        }
        let signature = tool_result_signature(msg)?;
        match &target {
            Some(target) if target == &signature => consecutive += 1,
            Some(_) => break,
            None => {
                target = Some(signature);
                consecutive = 1;
            }
        }
    }

    (consecutive >= threshold).then_some(consecutive)
}

fn tool_result_signature(msg: &Message) -> Option<String> {
    let metadata = msg.metadata.as_ref()?;
    let call = metadata.get("tool_call_fingerprint")?.as_str()?;
    let result = metadata.get("tool_result_fingerprint")?.as_str()?;
    Some(format!("{call}:{result}"))
}

#[derive(Debug)]
struct RepeatedReadRange {
    tool_name: String,
    path: String,
    repeated_range_count: usize,
    total_recent_reads: usize,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReadResourceKey {
    tool_name: String,
    path: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReadRangeKey {
    offset: Option<String>,
}

fn repeated_read_range_count(messages: &[Message], threshold: usize) -> Option<RepeatedReadRange> {
    let mut target_resource: Option<ReadResourceKey> = None;
    let mut range_counts: HashMap<ReadRangeKey, usize> = HashMap::new();
    let mut total_recent_reads = 0;
    let mut max_repeated_range_count = 0;

    for msg in messages.iter().rev() {
        match msg.role {
            MessageRole::User | MessageRole::System => break,
            MessageRole::Agent => {
                let tool_calls = msg.tool_calls();
                if tool_calls.is_empty() {
                    break;
                }

                let mut read_calls = Vec::new();
                for tool_call in tool_calls {
                    let read_call = read_call_key(tool_call)?;
                    read_calls.push(read_call);
                }

                for read_call in read_calls {
                    match &target_resource {
                        Some(target) if target == &read_call.resource => {}
                        Some(_) => return None,
                        None => target_resource = Some(read_call.resource.clone()),
                    }

                    total_recent_reads += 1;
                    let repeated_range_count = range_counts.entry(read_call.range).or_insert(0);
                    *repeated_range_count += 1;
                    max_repeated_range_count = max_repeated_range_count.max(*repeated_range_count);
                }
            }
            _ => continue,
        }
    }

    let target_resource = target_resource?;
    (max_repeated_range_count >= threshold && total_recent_reads > max_repeated_range_count)
        .then_some(RepeatedReadRange {
            tool_name: target_resource.tool_name,
            path: target_resource.path,
            repeated_range_count: max_repeated_range_count,
            total_recent_reads,
        })
}

#[derive(Clone, Debug)]
struct ReadCallKey {
    resource: ReadResourceKey,
    range: ReadRangeKey,
}

fn read_call_key(tool_call: &ToolCallContentPart) -> Option<ReadCallKey> {
    if !is_read_file_tool_name(&tool_call.name) {
        return None;
    }

    let path = tool_call.arguments.get("path")?.as_str()?.to_string();
    let offset = match tool_call.arguments.get("offset") {
        Some(serde_json::Value::Number(number)) => Some(number.to_string()),
        Some(serde_json::Value::String(value)) => Some(value.clone()),
        Some(value) => Some(value.to_string()),
        None => None,
    };

    Some(ReadCallKey {
        resource: ReadResourceKey {
            tool_name: tool_call.name.clone(),
            path,
        },
        range: ReadRangeKey { offset },
    })
}

fn is_read_file_tool_name(name: &str) -> bool {
    name == "read_file" || name.ends_with("__read_file")
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

    fn tool_result_msg(call_fingerprint: &str, result_fingerprint: &str) -> Message {
        let mut msg = Message::tool_result("call_1", Some(serde_json::json!({ "ok": true })), None);
        msg.metadata = Some(std::collections::HashMap::from([
            (
                "tool_call_fingerprint".to_string(),
                serde_json::json!(call_fingerprint),
            ),
            (
                "tool_result_fingerprint".to_string(),
                serde_json::json!(result_fingerprint),
            ),
        ]));
        msg
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
    fn test_loop_detected_three_identical_tool_results() {
        let filter = LoopDetectionFilter;
        let mut messages = vec![
            Message::user("do something"),
            agent_msg_with_calls(vec![("tool_a", serde_json::json!({"x": 1}))]),
            tool_result_msg("call:a", "result:a"),
            agent_msg_with_calls(vec![("tool_a", serde_json::json!({"x": 1}))]),
            tool_result_msg("call:a", "result:a"),
            agent_msg_with_calls(vec![("tool_a", serde_json::json!({"x": 1}))]),
            tool_result_msg("call:a", "result:a"),
        ];
        let original_len = messages.len();

        filter.post_load(&mut messages, &default_config());

        assert_eq!(messages.len(), original_len + 1);
        let last = messages.last().unwrap();
        assert_eq!(last.role, MessageRole::System);
        assert!(last.text().unwrap().contains("same tool call produced"));
    }

    #[test]
    fn test_loop_detected_repeated_read_range_with_alternating_offsets() {
        let filter = LoopDetectionFilter;
        let mut messages = vec![
            Message::user("inspect saved output"),
            agent_msg_with_calls(vec![(
                "read_file",
                serde_json::json!({"path": "/workspace/outputs/call_123.stdout", "offset": 0, "limit": 100}),
            )]),
            agent_msg_with_calls(vec![(
                "read_file",
                serde_json::json!({"path": "/workspace/outputs/call_123.stdout", "offset": 100, "limit": 100}),
            )]),
            agent_msg_with_calls(vec![(
                "read_file",
                serde_json::json!({"path": "/workspace/outputs/call_123.stdout", "offset": 0, "limit": 105}),
            )]),
            agent_msg_with_calls(vec![(
                "read_file",
                serde_json::json!({"path": "/workspace/outputs/call_123.stdout", "offset": 100, "limit": 105}),
            )]),
            agent_msg_with_calls(vec![(
                "read_file",
                serde_json::json!({"path": "/workspace/outputs/call_123.stdout", "offset": 0, "limit": 110}),
            )]),
        ];
        let original_len = messages.len();

        filter.post_load(&mut messages, &default_config());

        assert_eq!(messages.len(), original_len + 1);
        let last = messages.last().unwrap();
        assert_eq!(last.role, MessageRole::System);
        assert!(last.text().unwrap().contains("same file or output range"));
    }

    #[test]
    fn test_sequential_read_ranges_are_not_a_loop() {
        let filter = LoopDetectionFilter;
        let mut messages = vec![
            Message::user("inspect saved output"),
            agent_msg_with_calls(vec![(
                "read_file",
                serde_json::json!({"path": "/workspace/outputs/call_123.stdout", "offset": 0, "limit": 100}),
            )]),
            agent_msg_with_calls(vec![(
                "read_file",
                serde_json::json!({"path": "/workspace/outputs/call_123.stdout", "offset": 100, "limit": 100}),
            )]),
            agent_msg_with_calls(vec![(
                "read_file",
                serde_json::json!({"path": "/workspace/outputs/call_123.stdout", "offset": 200, "limit": 100}),
            )]),
            agent_msg_with_calls(vec![(
                "read_file",
                serde_json::json!({"path": "/workspace/outputs/call_123.stdout", "offset": 300, "limit": 100}),
            )]),
        ];
        let original_len = messages.len();

        filter.post_load(&mut messages, &default_config());

        assert_eq!(messages.len(), original_len);
    }

    #[test]
    fn test_tool_result_loop_breaks_on_different_result() {
        let filter = LoopDetectionFilter;
        let mut messages = vec![
            tool_result_msg("call:a", "result:a"),
            agent_msg_with_calls(vec![("tool_a", serde_json::json!({"x": 1}))]),
            tool_result_msg("call:a", "result:a"),
            agent_msg_with_calls(vec![("tool_a", serde_json::json!({"x": 1}))]),
            tool_result_msg("call:a", "result:b"),
        ];
        let original_len = messages.len();

        filter.post_load(&mut messages, &default_config());

        assert_eq!(messages.len(), original_len);
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

    #[test]
    fn test_config_schema_and_validate_config() {
        let cap = LoopDetectionCapability;

        let schema = cap.config_schema().expect("config schema");
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["threshold"].is_object());

        // Null, empty, and valid configs are accepted.
        assert!(cap.validate_config(&serde_json::Value::Null).is_ok());
        assert!(cap.validate_config(&serde_json::json!({})).is_ok());
        assert!(
            cap.validate_config(&serde_json::json!({"threshold": 2}))
                .is_ok()
        );

        // Zero, negative, and non-integer thresholds are rejected.
        assert!(
            cap.validate_config(&serde_json::json!({"threshold": 0}))
                .is_err()
        );
        assert!(
            cap.validate_config(&serde_json::json!({"threshold": -3}))
                .is_err()
        );
        assert!(
            cap.validate_config(&serde_json::json!({"threshold": "three"}))
                .is_err()
        );
    }

    #[test]
    fn test_localizations_resolve_uk() {
        let cap = LoopDetectionCapability;
        assert_eq!(
            cap.localized_name(Some("uk-UA")),
            "Виявлення циклів інструментів"
        );
        assert!(cap.describe_schema(None).is_some());
    }
}
