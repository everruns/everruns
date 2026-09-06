//! Stable tool call/result fingerprints for recent-window loop detection.

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::tool_types::{ToolCall, ToolResult};

const IGNORED_CALL_KEYS: &[&str] = &["human_intent", "output"];
const IGNORED_RESULT_KEYS: &[&str] = &[
    "created_at",
    "duration_ms",
    "elapsed_ms",
    "request_id",
    "time_ms",
    "timestamp",
    "updated_at",
];

pub fn tool_call_fingerprint(tool_call: &ToolCall) -> String {
    fingerprint_json(&json!({
        "tool_name": tool_call.name,
        "arguments": normalize_value_with_ignored_keys(&tool_call.arguments, IGNORED_CALL_KEYS),
    }))
}

pub fn tool_call_parts_fingerprint(tool_name: &str, arguments: &Value) -> String {
    fingerprint_json(&json!({
        "tool_name": tool_name,
        "arguments": normalize_value_with_ignored_keys(arguments, IGNORED_CALL_KEYS),
    }))
}

pub fn tool_result_fingerprint(tool_name: &str, tool_result: &ToolResult) -> String {
    fingerprint_json(&json!({
        "tool_name": tool_name,
        "success": tool_result.error.is_none(),
        "result": normalize_value_with_ignored_keys(
            tool_result.result.as_ref().unwrap_or(&Value::Null),
            IGNORED_RESULT_KEYS
        ),
        "error": normalize_error(tool_result.error.as_deref()),
        "connection_required": tool_result.connection_required,
    }))
}

pub fn tool_error_fingerprint(tool_name: &str, status: &str, error: &str) -> String {
    fingerprint_json(&json!({
        "tool_name": tool_name,
        "success": false,
        "status": status,
        "error": normalize_error(Some(error)),
    }))
}

fn fingerprint_json(value: &Value) -> String {
    let normalized = normalize_value(value);
    let encoded = serde_json::to_vec(&normalized).unwrap_or_default();
    let digest = Sha256::digest(encoded);
    format!("sha256:{}", to_lower_hex(&digest))
}

fn normalize_value_with_ignored_keys(value: &Value, ignored_keys: &[&str]) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<&String> = object
                .keys()
                .filter(|key| !ignored_keys.contains(&key.as_str()))
                .collect();
            keys.sort();
            let mut normalized = Map::new();
            for key in keys {
                normalized.insert(
                    key.clone(),
                    normalize_value_with_ignored_keys(&object[key], ignored_keys),
                );
            }
            Value::Object(normalized)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| normalize_value_with_ignored_keys(item, ignored_keys))
                .collect(),
        ),
        Value::String(text) => Value::String(normalize_text(text)),
        other => other.clone(),
    }
}

fn normalize_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            let mut normalized = Map::new();
            for key in keys {
                normalized.insert(key.clone(), normalize_value(&object[key]));
            }
            Value::Object(normalized)
        }
        Value::Array(items) => Value::Array(items.iter().map(normalize_value).collect()),
        Value::String(text) => Value::String(normalize_text(text)),
        other => other.clone(),
    }
}

fn normalize_error(error: Option<&str>) -> Option<String> {
    error.map(normalize_text)
}

fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_pin_canonical_hash_and_ignore_only_declared_metadata() {
        let expected = "sha256:40d39e472be6e5bd359976a905c732cbce4beeaade5d408b7c63fc27a79fb640";
        for arguments in [
            json!({"cmd":"cargo test"}),
            json!({"output":"verbose","human_intent":"Testing","cmd":"cargo test"}),
        ] {
            let call = ToolCall {
                id: "call_a".into(),
                name: "bash".into(),
                arguments: arguments.clone(),
            };
            assert_eq!(tool_call_fingerprint(&call), expected);
            assert_eq!(tool_call_parts_fingerprint("bash", &arguments), expected);
        }
        for (name, args) in [
            ("other", json!({"cmd":"cargo test"})),
            ("bash", json!({"cmd":"cargo build"})),
            ("bash", json!({"cmd":"cargo test","cwd":"/work"})),
        ] {
            assert_ne!(tool_call_parts_fingerprint(name, &args), expected);
        }
    }

    #[test]
    fn nested_normalization_preserves_array_order_and_meaningful_text() {
        let plain = json!({"steps":[{"text":"first\nsecond"},{"value":2}]});
        let formatted = json!({"output":"quiet","steps":[{"human_intent":"hint","text":" first \r\nsecond \r\n"},{"value":2,"output":"verbose"}]});
        let fingerprint = tool_call_parts_fingerprint("run", &plain);
        assert_eq!(tool_call_parts_fingerprint("run", &formatted), fingerprint);
        assert_ne!(
            tool_call_parts_fingerprint(
                "run",
                &json!({"steps":[{"value":2},{"text":"first\nsecond"}]})
            ),
            fingerprint
        );
        assert_ne!(
            tool_call_parts_fingerprint(
                "run",
                &json!({"steps":[{"text":"first second"},{"value":2}]})
            ),
            fingerprint
        );
    }

    #[test]
    fn results_ignore_volatile_fields_but_preserve_result_error_and_connection_identity() {
        let mut result = ToolResult {
            tool_call_id: "call_a".into(),
            result: Some(json!({"value":1})),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };
        let expected = "sha256:b2ef7dd0adef1ed7dbad196e8f582e211a4c13ab82c79a8afeb5517c11d46000";
        assert_eq!(tool_result_fingerprint("demo", &result), expected);
        for key in [
            "created_at",
            "duration_ms",
            "elapsed_ms",
            "request_id",
            "time_ms",
            "timestamp",
            "updated_at",
        ] {
            let mut with_metadata = result.clone();
            with_metadata
                .result
                .as_mut()
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(key.into(), json!("volatile"));
            assert_eq!(
                tool_result_fingerprint("demo", &with_metadata),
                expected,
                "{key}"
            );
        }
        result.tool_call_id = "call_b".into();
        result.raw_output = Some("private full output".into());
        assert_eq!(tool_result_fingerprint("demo", &result), expected);
        assert_ne!(tool_result_fingerprint("other", &result), expected);
        for changed in [
            ToolResult {
                result: Some(json!({"value":2})),
                ..result.clone()
            },
            ToolResult {
                error: Some("failed".into()),
                ..result.clone()
            },
            ToolResult {
                connection_required: Some("github".into()),
                ..result.clone()
            },
        ] {
            assert_ne!(tool_result_fingerprint("demo", &changed), expected);
        }
        let mut nested = result.clone();
        nested.result = Some(json!({"items":[{"value":1,"duration_ms":12}]}));
        let mut plain = nested.clone();
        plain.result = Some(json!({"items":[{"value":1}]}));
        assert_eq!(
            tool_result_fingerprint("demo", &nested),
            tool_result_fingerprint("demo", &plain)
        );
    }

    #[test]
    fn error_fingerprints_normalize_text_without_merging_status_or_tool() {
        let expected = tool_error_fingerprint("bash", "failed", "first\nsecond");
        assert_eq!(
            tool_error_fingerprint("bash", "failed", " first \r\nsecond \r\n"),
            expected
        );
        for (tool, status, error) in [
            ("other", "failed", "first\nsecond"),
            ("bash", "blocked", "first\nsecond"),
            ("bash", "failed", "different"),
        ] {
            assert_ne!(tool_error_fingerprint(tool, status, error), expected);
        }
    }
}
