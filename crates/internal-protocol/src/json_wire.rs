//! Conversion between `prost_types` JSON values and `serde_json`.
//!
//! Split out of `lib.rs` (EVE-1024): a self-contained pair of mappings with no
//! dependency on the rest of the crate, and `lib.rs` is on the file-size
//! ratchet.

use prost_types::{ListValue, Struct, Value, value::Kind};

/// Convert prost_types::Value to serde_json::Value
///
/// Note: Proto Struct's NumberValue is always f64, but we preserve integer
/// types when possible to ensure correct deserialization into u32/u64 fields.
pub fn proto_value_to_json(value: &Value) -> serde_json::Value {
    match &value.kind {
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::NumberValue(n)) => {
            // Check if the number is a whole number that can be represented as an integer.
            // This is important because proto Struct's NumberValue is always f64,
            // but many Rust structs have u32/u64 fields that can't deserialize floats.
            if n.fract() == 0.0 {
                // Try to convert to i64 first (handles negative integers and most cases)
                if *n >= i64::MIN as f64 && *n <= i64::MAX as f64 {
                    return serde_json::Value::Number(serde_json::Number::from(*n as i64));
                }
                // For very large positive integers, try u64
                if *n >= 0.0 && *n <= u64::MAX as f64 {
                    return serde_json::Value::Number(serde_json::Number::from(*n as u64));
                }
            }
            // Fall back to f64 for actual floating point numbers
            serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                .unwrap_or_else(|| {
                    // EVE-652: NaN/Infinity has no JSON number representation.
                    // It becomes null (unchanged), but the loss is now visible.
                    tracing::warn!(
                        value = *n,
                        "internal-protocol: non-finite number has no JSON representation; emitting null"
                    );
                    serde_json::Value::Null
                })
        }
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::ListValue(list)) => proto_list_to_json(list),
        Some(Kind::StructValue(s)) => proto_struct_to_json(s),
        None => serde_json::Value::Null,
    }
}

/// Convert prost_types::ListValue to serde_json::Value (array)
pub fn proto_list_to_json(list: &ListValue) -> serde_json::Value {
    serde_json::Value::Array(list.values.iter().map(proto_value_to_json).collect())
}

/// Convert prost_types::Struct to serde_json::Value (object)
pub fn proto_struct_to_json(s: &Struct) -> serde_json::Value {
    serde_json::Value::Object(
        s.fields
            .iter()
            .map(|(k, v)| (k.clone(), proto_value_to_json(v)))
            .collect(),
    )
}

/// Convert serde_json::Value to prost_types::Value
pub fn json_to_proto_value(value: &serde_json::Value) -> Value {
    Value {
        kind: Some(match value {
            serde_json::Value::Null => Kind::NullValue(0),
            serde_json::Value::Bool(b) => Kind::BoolValue(*b),
            serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or_else(|| {
                // EVE-652: a JSON number outside f64 range previously became 0.0
                // silently. Keep the fallback but surface the loss.
                tracing::warn!(
                    number = %n,
                    "internal-protocol: JSON number not representable as f64; using 0.0"
                );
                0.0
            })),
            serde_json::Value::String(s) => Kind::StringValue(s.clone()),
            serde_json::Value::Array(arr) => Kind::ListValue(json_array_to_proto_list(arr)),
            serde_json::Value::Object(obj) => Kind::StructValue(json_object_to_proto_struct(obj)),
        }),
    }
}

/// Convert JSON array to prost_types::ListValue
pub fn json_array_to_proto_list(arr: &[serde_json::Value]) -> ListValue {
    ListValue {
        values: arr.iter().map(json_to_proto_value).collect(),
    }
}

/// Convert JSON object to prost_types::Struct
pub fn json_object_to_proto_struct(obj: &serde_json::Map<String, serde_json::Value>) -> Struct {
    Struct {
        fields: obj
            .iter()
            .map(|(k, v)| (k.clone(), json_to_proto_value(v)))
            .collect(),
    }
}

/// Convert serde_json::Value to prost_types::ListValue (assumes array)
pub fn json_to_proto_list(value: &serde_json::Value) -> ListValue {
    match value {
        serde_json::Value::Array(arr) => json_array_to_proto_list(arr),
        _ => ListValue { values: vec![] },
    }
}

/// Convert serde_json::Value to prost_types::Struct (assumes object)
pub fn json_to_proto_struct(value: &serde_json::Value) -> Struct {
    match value {
        serde_json::Value::Object(obj) => json_object_to_proto_struct(obj),
        _ => Struct {
            fields: std::collections::BTreeMap::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proto_value_to_json_preserves_integers() {
        // Integer zero
        let value = Value {
            kind: Some(Kind::NumberValue(0.0)),
        };
        let json = proto_value_to_json(&value);
        assert!(json.is_number());
        assert_eq!(json.as_i64(), Some(0));
        // Verify it's an integer, not a float (important for serde deserialization)
        let json_str = serde_json::to_string(&json).unwrap();
        assert_eq!(json_str, "0", "Should serialize as integer, not 0.0");

        // Positive integer
        let value = Value {
            kind: Some(Kind::NumberValue(42.0)),
        };
        let json = proto_value_to_json(&value);
        assert_eq!(json.as_i64(), Some(42));
        let json_str = serde_json::to_string(&json).unwrap();
        assert_eq!(json_str, "42");

        // Negative integer
        let value = Value {
            kind: Some(Kind::NumberValue(-100.0)),
        };
        let json = proto_value_to_json(&value);
        assert_eq!(json.as_i64(), Some(-100));

        // Large u64 value (like duration_ms)
        let value = Value {
            kind: Some(Kind::NumberValue(5314.0)),
        };
        let json = proto_value_to_json(&value);
        assert_eq!(json.as_u64(), Some(5314));
    }

    #[test]
    fn test_proto_value_to_json_preserves_floats() {
        // Actual float with fractional part
        let value = Value {
            kind: Some(Kind::NumberValue(1.5)),
        };
        let json = proto_value_to_json(&value);
        assert!(json.is_f64());
        assert!((json.as_f64().unwrap() - 1.5).abs() < f64::EPSILON);

        // Negative float
        let value = Value {
            kind: Some(Kind::NumberValue(-2.5)),
        };
        let json = proto_value_to_json(&value);
        assert!(json.is_f64());
        assert!((json.as_f64().unwrap() - (-2.5)).abs() < f64::EPSILON);
    }

    #[test]
    fn test_proto_struct_roundtrip_with_integers() {
        // Test that integers survive a JSON -> Proto -> JSON roundtrip
        let original = serde_json::json!({
            "tool_call_count": 2,
            "success_count": 5,
            "duration_ms": 5314
        });

        // Convert to proto struct
        let proto_struct = json_to_proto_struct(&original);

        // Convert back to JSON
        let result = proto_struct_to_json(&proto_struct);

        // Verify integers are preserved
        assert_eq!(result["tool_call_count"].as_u64(), Some(2));
        assert_eq!(result["success_count"].as_u64(), Some(5));
        assert_eq!(result["duration_ms"].as_u64(), Some(5314));

        // Most importantly: verify they can deserialize into u32/u64
        #[derive(serde::Deserialize)]
        struct TestStruct {
            tool_call_count: u32,
            success_count: u32,
            duration_ms: u64,
        }

        let deserialized: TestStruct = serde_json::from_value(result).unwrap();
        assert_eq!(deserialized.tool_call_count, 2);
        assert_eq!(deserialized.success_count, 5);
        assert_eq!(deserialized.duration_ms, 5314);
    }
}
