use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

/// Flatten only the command envelope. Nested request schemas retain their
/// original constraints and local references. Invocation metadata belongs to
/// the envelope and is removed by the shared command dispatcher before serde.
pub(super) fn parameters(schema: &Value) -> Value {
    let mut properties = Map::new();
    let mut required = BTreeSet::new();
    collect(schema, schema, &mut properties, &mut required, 0);
    assert!(!properties.contains_key("organization_id"));
    for (name, field) in [
        (
            "reason",
            json!({"type":"string", "description":"Why the user requested this change. Recorded in history."}),
        ),
        (
            "context_revision",
            json!({"type":"integer", "description":"Manager context revision read before changing an existing entity."}),
        ),
    ] {
        assert!(properties.insert(name.into(), field).is_none());
    }
    let mut result = json!({
        "type": "object", "properties": properties,
        "required": required, "additionalProperties": false,
    });
    if let Some(definitions) = schema.get("$defs").and_then(Value::as_object) {
        let mut retained = Map::new();
        let mut pending = vec![&result];
        let mut referenced = BTreeSet::new();
        while let Some(value) = pending.pop() {
            match value {
                Value::Object(object) => {
                    if let Some(name) = object
                        .get("$ref")
                        .and_then(Value::as_str)
                        .and_then(|reference| reference.strip_prefix("#/$defs/"))
                        && referenced.insert(name.to_owned())
                        && let Some(definition) = definitions.get(name)
                    {
                        retained.insert(name.to_owned(), definition.clone());
                        pending.push(definition);
                    }
                    pending.extend(object.values());
                }
                Value::Array(values) => pending.extend(values),
                _ => {}
            }
        }
        if !retained.is_empty() {
            result["$defs"] = Value::Object(retained);
        }
    }
    result
}

fn collect(
    schema: &Value,
    root: &Value,
    properties: &mut Map<String, Value>,
    required: &mut BTreeSet<String>,
    depth: usize,
) {
    assert!(depth < 32, "cyclic command envelope");
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let target = root
            .pointer(
                reference
                    .strip_prefix('#')
                    .expect("local command reference"),
            )
            .expect("resolved command reference");
        collect(target, root, properties, required, depth + 1);
    }
    if let Some(fields) = schema.get("properties").and_then(Value::as_object) {
        properties.extend(fields.clone());
    }
    if let Some(fields) = schema.get("required").and_then(Value::as_array) {
        required.extend(fields.iter().filter_map(Value::as_str).map(str::to_owned));
    }
    if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
        for part in parts {
            collect(part, root, properties, required, depth + 1);
        }
    }
}
