//! Native invocation envelope shared by production and the Rust/Mira fixture.

use serde_json::{Value, json};

pub fn native_schema(payload: &Value) -> Value {
    fn implies_object(schema: &Value, root: &Value, depth: usize) -> bool {
        if depth > 32 {
            return false;
        }
        schema["type"] == "object"
            || schema
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix('#'))
                .and_then(|pointer| root.pointer(pointer))
                .is_some_and(|target| implies_object(target, root, depth + 1))
            || schema
                .get("allOf")
                .and_then(Value::as_array)
                .is_some_and(|parts| {
                    parts
                        .iter()
                        .any(|part| implies_object(part, root, depth + 1))
                })
    }
    fn rebase(value: &mut Value) {
        match value {
            Value::Object(object) => {
                if let Some(reference) = object.get_mut("$ref")
                    && let Some(fragment) =
                        reference.as_str().and_then(|text| text.strip_prefix('#'))
                    && (fragment.is_empty() || fragment.starts_with('/'))
                {
                    *reference = json!(format!("#/properties/params{fragment}"));
                }
                for child in object.values_mut() {
                    rebase(child);
                }
            }
            Value::Array(array) => {
                for child in array {
                    rebase(child);
                }
            }
            _ => {}
        }
    }
    let object_payload = implies_object(payload, payload, 0);
    let mut payload = payload.clone();
    // Some flattened commands express object-ness only through allOf
    // or a root $ref. Provider schema adapters can otherwise expose params as a
    // string. Make the already-required type explicit; never coerce arguments
    // or restrict an unconstrained/nullable payload.
    if object_payload && payload.get("type").is_none() {
        payload
            .as_object_mut()
            .expect("schema object")
            .insert("type".into(), json!("object"));
    }
    rebase(&mut payload);
    json!({
        "type":"object", "additionalProperties":false, "required":["params"],
        "properties":{
            "params":payload,
            "reason":{"type":"string","description":"Required for agent entity changes. Describe what the user asked to change; never include credentials."},
            "context_revision":{"type":"integer","description":"Manager context revision read before changing an existing entity. Read context first and pass its revision."}
        }
    })
}
