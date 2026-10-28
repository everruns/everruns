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
/// closed without risking a semantic change, or exceeds OpenAI's object-depth
/// or total-property limits. Callers must then use the
/// sanitized non-strict schema instead.
pub fn strict_openai_tool_schema(schema: &Value) -> Option<Value> {
    if !within_openai_strict_complexity(schema, 0, &mut 5000) {
        return None;
    }
    let mut normalized = sanitize_openai_tool_schema(schema);
    if normalized.get("type").and_then(Value::as_str) != Some("object") {
        return None;
    }
    normalize_strict_node(&mut normalized)?;
    Some(normalized)
}

// Count schema nodes only: enum values and descriptions are data. References
// and definitions already decline strict conversion, so they are not expanded.
// https://developers.openai.com/api/docs/guides/structured-outputs
fn within_openai_strict_complexity(
    schema: &Value,
    parent_object_depth: usize,
    remaining_properties: &mut usize,
) -> bool {
    let is_object = schema.get("type").is_some_and(|kind| {
        kind == "object"
            || kind
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "object"))
    });
    let object_depth = parent_object_depth + usize::from(is_object);
    if object_depth > 10 {
        return false;
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        let Some(remaining) = remaining_properties.checked_sub(properties.len()) else {
            return false;
        };
        *remaining_properties = remaining;
        if !properties
            .values()
            .all(|child| within_openai_strict_complexity(child, object_depth, remaining_properties))
        {
            return false;
        }
    }
    if let Some(items) = schema.get("items")
        && !within_openai_strict_complexity(items, object_depth, remaining_properties)
    {
        return false;
    }
    schema
        .get("anyOf")
        .and_then(Value::as_array)
        .is_none_or(|variants| {
            variants.iter().all(|variant| {
                within_openai_strict_complexity(variant, parent_object_depth, remaining_properties)
            })
        })
}

/// Reverse only the null placeholders introduced by strict schema conversion.
/// Required nulls, explicitly nullable values and unknown fields are preserved
/// for ordinary tool validation. Run at the provider boundary before calls are
/// recorded, fingerprinted or approved; execution hooks must not rewrite them.
pub fn restore_strict_openai_tool_arguments(schema: &Value, arguments: &mut Value) {
    if strict_openai_tool_schema(schema).is_some() {
        restore_argument_node(schema, arguments);
    }
}

#[cfg(feature = "http")]
pub(crate) fn restore_strict_openai_tool_calls(
    tools: &[crate::tool_types::ToolDefinition],
    calls: &mut [crate::tool_types::ToolCall],
) {
    for call in calls {
        if let Some(tool) = tools.iter().find(|tool| tool.name() == call.name) {
            restore_strict_openai_tool_arguments(tool.parameters(), &mut call.arguments);
        }
    }
}

fn accepts_null(schema: &Value) -> bool {
    let type_allows = schema.get("type").is_none_or(|kind| match kind {
        Value::String(kind) => kind == "null",
        Value::Array(kinds) => kinds.iter().any(|kind| kind.as_str() == Some("null")),
        _ => false,
    });
    type_allows
        && schema.get("enum").is_none_or(|values| {
            values
                .as_array()
                .is_some_and(|values| values.contains(&Value::Null))
        })
        && schema.get("anyOf").is_none_or(|variants| {
            variants
                .as_array()
                .is_some_and(|variants| variants.iter().any(accepts_null))
        })
}

fn restore_argument_node(schema: &Value, arguments: &mut Value) {
    if let (Some(properties), Some(object)) = (
        schema.get("properties").and_then(Value::as_object),
        arguments.as_object_mut(),
    ) {
        let required = schema.get("required").and_then(Value::as_array);
        object.retain(|name, value| {
            let Some(property) = properties.get(name) else {
                return true;
            };
            let required = required
                .is_some_and(|fields| fields.iter().any(|field| field.as_str() == Some(name)));
            if value.is_null() && !required && !accepts_null(property) {
                return false;
            }
            restore_argument_node(property, value);
            true
        });
    }
    if let (Some(items), Some(array)) = (schema.get("items"), arguments.as_array_mut()) {
        for value in array {
            restore_argument_node(items, value);
        }
    }
    if let Some(variants) = schema.get("anyOf").and_then(Value::as_array) {
        // Do not guess which union branch the model intended. Only a unanimous
        // inverse is safe; ambiguous arguments remain for authoritative validation.
        let mut proposals = variants.iter().map(|variant| {
            let mut restored = arguments.clone();
            restore_argument_node(variant, &mut restored);
            restored
        });
        if let Some(first) = proposals.next()
            && proposals.all(|other| other == first)
        {
            *arguments = first;
        }
    }
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

    fn nested_objects(levels: usize) -> Value {
        let mut schema = json!({"type": "string"});
        for _ in 0..levels {
            schema = json!({"type": "object", "properties": {"child": schema}});
        }
        schema
    }

    #[test]
    fn strict_schema_complexity_counts_object_levels_through_arrays_and_unions() {
        assert!(strict_openai_tool_schema(&nested_objects(10)).is_some());
        assert!(strict_openai_tool_schema(&nested_objects(11)).is_none());
        for levels in [9, 10] {
            let schema = json!({"type": "object", "properties": {
                "children": {"type": "array", "items": {"anyOf": [
                    nested_objects(levels), {"type": "null"}
                ]}}
            }});
            assert_eq!(strict_openai_tool_schema(&schema).is_some(), levels == 9);
        }
        // Array and union wrappers do not consume object nesting levels.
        let mut schema = json!({"type": "string"});
        for _ in 0..11 {
            schema = json!({"type": "array", "items": {"anyOf": [schema]}});
        }
        assert!(
            strict_openai_tool_schema(&json!({
                "type": "object", "properties": {"items": schema}
            }))
            .is_some()
        );
    }

    #[test]
    fn strict_schema_complexity_counts_properties_across_branches() {
        let properties: serde_json::Map<String, Value> = (0..2499)
            .map(|n| (format!("field_{n}"), json!({"type": "string"})))
            .collect();
        let mut schema = json!({"type": "object", "properties": {
            "choice": {"anyOf": [
                {"type": "object", "properties": properties},
                {"type": "object", "properties": properties}
            ]},
            "optional": {"type": "string"}
        }});
        assert!(
            strict_openai_tool_schema(&schema).is_some(),
            "5000 properties"
        );
        schema["properties"]["extra"] = json!({"type": "string"});
        assert!(
            strict_openai_tool_schema(&schema).is_none(),
            "5001 properties"
        );
        let original = schema.clone();
        let mut args = json!({"optional": null, "unknown": 1});
        restore_strict_openai_tool_arguments(&schema, &mut args);
        assert_eq!(args, json!({"optional": null, "unknown": 1}));
        assert_eq!(schema, original);
    }

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

#[cfg(test)]
mod strict_argument_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn restores_optional_placeholders_without_weakening_validation() {
        let schema = json!({"type":"object", "properties":{
            "optional":{"type":"string"},
            "required":{"type":"string"},
            "nullable":{"type":["string","null"]},
            "enum":{"type":"string","enum":["a"]},
            "nested":{"type":"object","properties":{"flag":{"type":"boolean"}}},
            "array":{"type":"array","items":{"type":"object","properties":{"flag":{"type":"boolean"}}}}
        },"required":["required"]});
        let mut args = json!({"optional":null,"required":null,"nullable":null,"enum":null,"unknown":null,"nested":{"flag":null},"array":[{"flag":null}]});
        restore_strict_openai_tool_arguments(&schema, &mut args);
        assert_eq!(
            args,
            json!({"required":null,"nullable":null,"unknown":null,"nested":{},"array":[{}]})
        );
        let mut explicit = json!({"optional":"value","enum":"a","required":"ok"});
        let before = explicit.clone();
        restore_strict_openai_tool_arguments(&schema, &mut explicit);
        assert_eq!(explicit, before);
    }

    #[test]
    fn nonstrict_schemas_and_ambiguous_unions_keep_the_arguments() {
        let open = json!({"type":"object","properties":{"value":{"type":"string"}},"additionalProperties":true});
        let mut args = json!({"value":null});
        restore_strict_openai_tool_arguments(&open, &mut args);
        assert_eq!(args, json!({"value":null}));
        let union = json!({"type":"object","properties":{"value":{"anyOf":[
            {"type":"object","properties":{"a":{"type":"string"}}},
            {"type":"object","properties":{"a":{"type":"string"}},"required":["a"]}
        ]}}});
        let mut args = json!({"value":{"a":null}});
        restore_strict_openai_tool_arguments(&union, &mut args);
        assert_eq!(args, json!({"value":{"a":null}}));
    }
}
