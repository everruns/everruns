//! Provider-specific JSON Schema compatibility at the LLM request boundary.

use serde_json::Value;

const ZOD_EMAIL_PATTERN: &str = r"^(?!\.)(?!.*\.\.)([A-Za-z0-9_'+\-\.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$";
const OPENAI_SAFE_ZOD_EMAIL_PATTERN: &str = r"^[A-Za-z0-9_'+\-](?:[A-Za-z0-9_'+\-]|\.[A-Za-z0-9_'+\-])*@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$";
const SERVER_VALIDATION_NOTE: &str = "Validation for this value is enforced by the tool.";

/// Make a tool schema acceptable to OpenAI without changing compatible constraints.
///
/// OpenAI rejects ECMAScript lookaround in `pattern`. The common Zod email
/// pattern has a lookaround-free equivalent. Other unsupported patterns cannot
/// be translated soundly in general, so they are omitted from the model-facing
/// copy only; the MCP server remains the validation boundary.
pub fn sanitize_openai_tool_schema(schema: &Value) -> Value {
    let mut sanitized = schema.clone();
    sanitize_node(&mut sanitized);
    sanitized
}

fn sanitize_node(node: &mut Value) {
    match node {
        Value::Object(object) => {
            let pattern = object
                .get("pattern")
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(pattern) = pattern {
                if pattern == ZOD_EMAIL_PATTERN {
                    object.insert(
                        "pattern".to_string(),
                        Value::String(OPENAI_SAFE_ZOD_EMAIL_PATTERN.to_string()),
                    );
                } else if contains_regex_lookaround(&pattern) {
                    object.remove("pattern");
                    let description = object
                        .get("description")
                        .and_then(Value::as_str)
                        .map(|description| format!("{description} {SERVER_VALIDATION_NOTE}"))
                        .unwrap_or_else(|| SERVER_VALIDATION_NOTE.to_string());
                    object.insert("description".to_string(), Value::String(description));
                }
            }
            for value in object.values_mut() {
                sanitize_node(value);
            }
        }
        Value::Array(items) => {
            for item in items {
                sanitize_node(item);
            }
        }
        _ => {}
    }
}

fn contains_regex_lookaround(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let mut escaped = false;
    let mut in_character_class = false;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'[' if !in_character_class => in_character_class = true,
            b']' if in_character_class => in_character_class = false,
            b'(' if !in_character_class
                && bytes.get(index + 1) == Some(&b'?')
                && (matches!(bytes.get(index + 2), Some(b'=') | Some(b'!'))
                    || matches!(
                        (bytes.get(index + 2), bytes.get(index + 3)),
                        (Some(b'<'), Some(b'=' | b'!'))
                    )) =>
            {
                return true;
            }
            _ => {}
        }
        index += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;
    use serde_json::json;

    #[test]
    fn rewrites_exact_resend_zod_email_pattern_without_weakening_validation() {
        let schema = json!({
            "type": "object",
            "properties": {
                "email": {"type": "string", "pattern": "^(?!\\.)(?!.*\\.\\.)([A-Za-z0-9_'+\\-\\.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\\-]*\\.)+[A-Za-z]{2,}$"}
            }
        });

        let sanitized = sanitize_openai_tool_schema(&schema);
        let pattern = sanitized["properties"]["email"]["pattern"]
            .as_str()
            .unwrap();
        assert_eq!(pattern, OPENAI_SAFE_ZOD_EMAIL_PATTERN);

        let email = Regex::new(pattern).unwrap();
        for valid in ["a@example.com", "first.last+tag@example.co.uk"] {
            assert!(email.is_match(valid), "expected valid: {valid}");
        }
        for invalid in [
            ".first@example.com",
            "first..last@example.com",
            "first.@example.com",
            "missing-at.example.com",
            "first@example",
        ] {
            assert!(!email.is_match(invalid), "expected invalid: {invalid}");
        }
    }

    #[test]
    fn leaves_supported_patterns_and_unrelated_schema_unchanged() {
        let schema = json!({
            "type": "object",
            "properties": {
                "slug": {"type": "string", "pattern": "^[a-z0-9-]+$"},
                "literal": {"type": "string", "pattern": "^\\\\(\\\\?=value$"},
                "class": {"type": "string", "pattern": "[(?=]+"}
            }
        });

        assert_eq!(sanitize_openai_tool_schema(&schema), schema);
    }

    #[test]
    fn removes_other_unsupported_lookaround_but_preserves_server_validation_notice() {
        let schema = json!({
            "type": "array",
            "items": {
                "type": "string",
                "description": "Tenant-scoped identifier.",
                "pattern": "^(?=.{3,20}$)[a-z]+$"
            }
        });

        let sanitized = sanitize_openai_tool_schema(&schema);

        assert!(sanitized["items"].get("pattern").is_none());
        assert_eq!(
            sanitized["items"]["description"],
            "Tenant-scoped identifier. Validation for this value is enforced by the tool."
        );
    }
}
