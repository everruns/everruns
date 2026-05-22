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
    fn tool_call_fingerprint_ignores_ui_only_fields_and_key_order() {
        let a = ToolCall {
            id: "call_a".to_string(),
            name: "bash".to_string(),
            arguments: json!({
                "cmd": "cargo test",
                "human_intent": "Testing",
                "output": "verbose"
            }),
        };
        let b = ToolCall {
            id: "call_b".to_string(),
            name: "bash".to_string(),
            arguments: json!({
                "output": "concise",
                "cmd": "cargo test"
            }),
        };

        assert_eq!(tool_call_fingerprint(&a), tool_call_fingerprint(&b));
    }

    #[test]
    fn tool_result_fingerprint_ignores_volatile_result_fields() {
        let a = ToolResult {
            tool_call_id: "call_a".to_string(),
            result: Some(json!({"value": 1, "duration_ms": 10})),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };
        let b = ToolResult {
            tool_call_id: "call_b".to_string(),
            result: Some(json!({"duration_ms": 99, "value": 1})),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };

        assert_eq!(
            tool_result_fingerprint("demo", &a),
            tool_result_fingerprint("demo", &b)
        );
    }
}
