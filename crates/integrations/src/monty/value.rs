//! Bounded JSON bridge. Reject cycles, opaque objects and lossy integer conversion.
use monty_types::{MontyObject, ObjectRef};
use serde_json::Value;

pub(super) const MAX_VALUE_BYTES: usize = 256 * 1024;

pub(super) fn from_python(value: ObjectRef<'_>) -> Result<Value, String> {
    let mut remaining = MAX_VALUE_BYTES;
    let value = convert(value, 0, &mut remaining)?;
    encoded_size(&value, MAX_VALUE_BYTES)?;
    Ok(value)
}

fn convert(value: ObjectRef<'_>, depth: usize, remaining: &mut usize) -> Result<Value, String> {
    if depth > 64 || *remaining < 16 {
        return Err("Python value exceeds the JSON size/depth limit (or contains a cycle)".into());
    }
    *remaining -= 16;
    Ok(match value.type_name() {
        "NoneType" => Value::Null,
        "bool" => Value::Bool(value.as_bool().ok_or("invalid bool")?),
        "int" => Value::from(
            value
                .as_int()
                .ok_or("Python integer must fit signed 64 bits")?,
        ),
        "float" => Value::Number(
            serde_json::Number::from_f64(value.as_float().ok_or("invalid float")?)
                .ok_or("non-finite floats are not JSON")?,
        ),
        "str" => {
            let s = value.as_str().ok_or("invalid string")?;
            *remaining = remaining
                .checked_sub(s.len())
                .ok_or("Python value too large")?;
            Value::String(s.to_string())
        }
        "list" | "tuple" => Value::Array(
            value
                .items()
                .ok_or("invalid sequence")?
                .into_iter()
                .map(|v| convert(v, depth + 1, remaining))
                .collect::<Result<_, _>>()?,
        ),
        "dict" => {
            let mut map = serde_json::Map::new();
            for (key, value) in value.pairs().ok_or("invalid dict")? {
                let key = key.as_str().ok_or("JSON object keys must be strings")?;
                *remaining = remaining
                    .checked_sub(key.len())
                    .ok_or("Python value too large")?;
                map.insert(key.to_string(), convert(value, depth + 1, remaining)?);
            }
            Value::Object(map)
        }
        other => return Err(format!("Python {other} is not a JSON value")),
    })
}

pub(super) fn to_python(value: &Value) -> Result<MontyObject, String> {
    // The JSON parser and from_python already bound nesting. Check again for
    // trusted tool implementations that construct a Value without parsing.
    fn convert(value: &Value, depth: usize) -> Result<MontyObject, String> {
        if depth > 64 {
            return Err("tool result exceeds JSON depth limit".into());
        }
        Ok(match value {
            Value::Null => MontyObject::none(),
            Value::Bool(v) => MontyObject::bool(*v),
            Value::Number(v) => {
                if let Some(i) = v.as_i64() {
                    MontyObject::int(i)
                } else if v.is_f64() {
                    MontyObject::float(v.as_f64().ok_or("invalid number")?)
                } else {
                    return Err("JSON integer must fit signed 64 bits".into());
                }
            }
            Value::String(v) => MontyObject::string(v),
            Value::Array(v) => MontyObject::list(
                v.iter()
                    .map(|v| convert(v, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            Value::Object(v) => MontyObject::dict(
                v.iter()
                    .map(|(k, v)| Ok((MontyObject::string(k), convert(v, depth + 1)?)))
                    .collect::<Result<Vec<_>, String>>()?,
            ),
        })
    }
    encoded_size(value, MAX_VALUE_BYTES)?;
    convert(value, 0)
}

/// Count encoded bytes without allocating a second copy of a tool's output.
pub(super) fn encoded_size(value: &impl serde::Serialize, limit: usize) -> Result<usize, String> {
    struct Counter {
        used: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.used) {
                return Err(std::io::Error::other("JSON size limit"));
            }
            self.used += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { used: 0, limit };
    serde_json::to_writer(&mut counter, value).map_err(|_| "JSON value exceeds size limit")?;
    Ok(counter.used)
}
