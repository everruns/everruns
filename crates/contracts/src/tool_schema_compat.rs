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

/// Normalize a tool schema for OpenAI strict structured outputs.
///
/// Returns `None` when the schema uses constructs whose object shape cannot be
/// closed without risking a semantic change. Callers must then use the
/// sanitized non-strict schema instead.
pub fn strict_openai_tool_schema(schema: &Value) -> Option<Value> {
    let mut normalized = sanitize_openai_tool_schema(schema);
    if normalized.get("type").and_then(Value::as_str) != Some("object") {
        return None;
    }
    normalize_strict_node(&mut normalized)?;
    Some(normalized)
}

fn normalize_strict_node(node: &mut Value) -> Option<()> {
    let object = node.as_object_mut()?;

    // Strict structured outputs support only a JSON Schema subset. Decline
    // unknown keywords rather than sending a request the provider may reject.
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "type"
                | "properties"
                | "required"
                | "additionalProperties"
                | "items"
                | "enum"
                | "anyOf"
                | "description"
                | "title"
        )
    }) {
        return None;
    }

    if let Some(any_of) = object.get_mut("anyOf") {
        let variants = any_of.as_array_mut()?;
        if variants.is_empty() {
            return None;
        }
        for variant in variants {
            normalize_strict_node(variant)?;
        }
    }

    let schema_type = object.get("type").cloned();
    let types = match schema_type.as_ref() {
        Some(Value::String(value)) => vec![value.as_str()],
        Some(Value::Array(values)) if !values.is_empty() => values
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()?,
        None if object.contains_key("anyOf") || object.contains_key("enum") => Vec::new(),
        _ => return None,
    };

    let is_object = types.contains(&"object");
    let is_array = types.contains(&"array");
    if (object.contains_key("properties") || object.contains_key("required")) && !is_object {
        return None;
    }
    if object.contains_key("items") && !is_array {
        return None;
    }

    if is_object {
        // Explicitly open/dictionary shapes cannot be closed without removing
        // valid entries. Preserve them through the caller's non-strict fallback.
        if object
            .get("additionalProperties")
            .is_some_and(|value| value != &Value::Bool(false))
        {
            return None;
        }
        // A union containing object and another non-null type has no unambiguous
        // place for object-only strict constraints.
        if types
            .iter()
            .any(|value| *value != "object" && *value != "null")
        {
            return None;
        }
        let originally_required: std::collections::HashSet<String> = object
            .get("required")
            .map(|required| required.as_array().map(Vec::as_slice))
            .unwrap_or(Some(&[]))?
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect::<Option<_>>()?;
        let property_names = {
            let properties = object
                .entry("properties")
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                .as_object_mut()?;
            if originally_required
                .iter()
                .any(|name| !properties.contains_key(name))
            {
                return None;
            }

            for (name, property) in properties.iter_mut() {
                normalize_strict_node(property)?;
                if !originally_required.contains(name) {
                    make_nullable(property)?;
                }
            }
            properties.keys().cloned().map(Value::String).collect()
        };
        object.insert("required".to_string(), Value::Array(property_names));
        object.insert("additionalProperties".to_string(), Value::Bool(false));
    }

    if is_array {
        if types
            .iter()
            .any(|value| *value != "array" && *value != "null")
        {
            return None;
        }
        normalize_strict_node(object.get_mut("items")?)?;
    }

    Some(())
}

fn make_nullable(schema: &mut Value) -> Option<()> {
    let object = schema.as_object_mut()?;
    // Type and enum constraints both apply: adding null only to `type` still
    // leaves an optional enum impossible to omit in strict mode.
    if let Some(values) = object.get_mut("enum") {
        let values = values.as_array_mut()?;
        if !values.contains(&Value::Null) {
            values.push(Value::Null);
        }
    }
    if let Some(schema_type) = object.get_mut("type") {
        match schema_type {
            Value::String(value) if value != "null" => {
                *schema_type = Value::Array(vec![
                    Value::String(value.clone()),
                    Value::String("null".into()),
                ]);
            }
            Value::String(_) => {}
            Value::Array(types) => {
                if !types.iter().all(Value::is_string) {
                    return None;
                }
                if !types.iter().any(|value| value.as_str() == Some("null")) {
                    types.push(Value::String("null".into()));
                }
            }
            _ => return None,
        }
        return Some(());
    }

    let variants = object.get_mut("anyOf")?.as_array_mut()?;
    if !variants
        .iter()
        .any(|variant| variant.get("type").and_then(Value::as_str) == Some("null"))
    {
        variants.push(serde_json::json!({"type": "null"}));
    }
    Some(())
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
            // Recurse only into schema-valued keywords. Objects in enum,
            // default, const, examples or extension data are literal payloads.
            for (keyword, value) in object.iter_mut() {
                match keyword.as_str() {
                    "properties" | "patternProperties" | "$defs" | "definitions"
                    | "dependentSchemas" | "dependencies" => {
                        if let Some(schemas) = value.as_object_mut() {
                            for schema in schemas.values_mut() {
                                sanitize_node(schema);
                            }
                        }
                    }
                    "items"
                    | "additionalItems"
                    | "additionalProperties"
                    | "contains"
                    | "propertyNames"
                    | "not"
                    | "if"
                    | "then"
                    | "else"
                    | "unevaluatedProperties"
                    | "unevaluatedItems"
                    | "contentSchema"
                    | "allOf"
                    | "anyOf"
                    | "oneOf"
                    | "prefixItems" => sanitize_node(value),
                    _ => {}
                }
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
        assert_eq!(
            pattern,
            r"^[A-Za-z0-9_'+\-](?:[A-Za-z0-9_'+\-]|\.[A-Za-z0-9_'+\-])*@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$"
        );

        let email = Regex::new(pattern).unwrap();
        for valid in [
            "a@example.com",
            "first.last+tag@example.co.uk",
            "o'neill@example.com",
            "_@example.com",
        ] {
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
    fn sanitizer_preserves_literal_data_while_visiting_schema_branches() {
        let schema = json!({
            "type":"object",
            "properties":{"payload":{
                "type":"object",
                "default":{"pattern":"(?=literal)"},
                "enum":[{"pattern":"(?=literal)"}],
                "examples":[{"pattern":"(?=literal)"}]
            }},
            "$defs":{"code":{"type":"string","pattern":"(?=real)"}},
            "anyOf":[{"properties":{"code":{"type":"string","pattern":"(?<=real)"}}}]
        });
        let mut expected = schema.clone();
        expected["$defs"]["code"] = json!({"type":"string","description":"Validation for this value is enforced by the tool."});
        expected["anyOf"][0]["properties"]["code"] = json!({"type":"string","description":"Validation for this value is enforced by the tool."});
        assert_eq!(sanitize_openai_tool_schema(&schema), expected);
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
        for pattern in ["^(?=.{3,20}$)[a-z]+$", "a(?!b)", "(?<=a)b", "(?<!a)b"] {
            for description in [None, Some("Tenant-scoped identifier.")] {
                let mut item = json!({"type":"string","pattern":pattern});
                if let Some(description) = description {
                    item["description"] = json!(description);
                }
                let schema = json!({"type":"array","items":item});
                let notice = match description {
                    Some(_) => {
                        "Tenant-scoped identifier. Validation for this value is enforced by the tool."
                    }
                    None => "Validation for this value is enforced by the tool.",
                };
                assert_eq!(
                    sanitize_openai_tool_schema(&schema),
                    json!({"type":"array","items":{"type":"string","description":notice}})
                );
            }
        }
    }
}

#[cfg(test)]
mod strict_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strict_schema_closes_nested_objects_and_makes_optional_fields_nullable() {
        let normalized = strict_openai_tool_schema(&json!({
            "type": "object",
            "properties": {
                "fixed": {"type": "string"},
                "nested": {
                    "type": "object",
                    "properties": {"value": {"type": "integer"}},
                    "required": ["value"]
                },
                "list": {"type": "array", "items": {
                    "type": "object", "properties": {"flag": {"type": "boolean"}}
                }}
            },
            "required": ["fixed"]
        }))
        .unwrap();

        assert_eq!(
            normalized,
            json!({
                "type":"object", "additionalProperties":false, "required":["fixed","list","nested"],
                "properties": {
                    "fixed":{"type":"string"},
                    "nested":{"type":["object","null"],"properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false},
                    "list":{"type":["array","null"],"items":{
                        "type":"object","properties":{"flag":{"type":["boolean","null"]}},"required":["flag"],"additionalProperties":false
                    }}
                }
            })
        );
    }

    #[test]
    fn strict_schema_declines_ambiguous_composition() {
        for input in [
            json!({"type":"array","items":{"type":"string"}}),
            json!({"type":"object","required":["missing"]}),
            json!({"type":"object","properties":{"value":{"type":"array"}}}),
            json!({"type":"object","properties":{"value":{"type":["object","string"]}}}),
            json!({"type":"object","properties":{"value":{"anyOf":[]}}}),
        ] {
            assert_eq!(strict_openai_tool_schema(&input), None, "{input}");
        }
        assert!(
            strict_openai_tool_schema(&json!({
                "type": "object",
                "allOf": [{"type": "object", "properties": {"value": {"type": "string"}}}]
            }))
            .is_none()
        );
        assert!(
            strict_openai_tool_schema(&json!({
                "type": "object",
                "properties": {"value": {"type": "string", "pattern": "^[a-z]+$"}}
            }))
            .is_none()
        );
    }
    #[test]
    fn optional_enum_allows_null_without_relaxing_required_enum() {
        let input = json!({"type":"object", "properties": {
            "choice":{"type":"string","enum":["a","b"]},
            "fixed":{"type":"string","enum":["x"]},
            "nullable":{"type":["string","null"],"enum":["a",null]}
        }, "required":["fixed"]});
        assert_eq!(
            strict_openai_tool_schema(&input),
            Some(json!({
                "type":"object", "properties": {
                    "choice":{"type":["string","null"],"enum":["a","b",null]},
                    "fixed":{"type":"string","enum":["x"]},
                    "nullable":{"type":["string","null"],"enum":["a",null]}
                }, "required":["choice","fixed","nullable"],"additionalProperties":false
            }))
        );
    }

    #[test]
    fn strict_schema_declines_dictionary_shapes_instead_of_discarding_entries() {
        for additional in [json!(true), json!({"type":"string"})] {
            let input = json!({"type":"object","properties":{"labels":{
                "type":"object","additionalProperties":additional
            }},"required":["labels"]});
            assert_eq!(strict_openai_tool_schema(&input), None, "{input}");
        }
    }
}
