// Agent-script validation. Pure functions so the limits are testable without
// storage. The limits are part of the resource contract.

use crate::domains::common::CommandError;
use serde_json::Value;

pub const MAX_NAME_LEN: usize = 64;
pub const MAX_DESCRIPTION_CHARS: usize = 300;
pub const MAX_BODY_BYTES: usize = 65_536;
/// Active scripts one agent may own.
pub const MAX_ACTIVE_SCRIPTS_PER_AGENT: i64 = 100;

/// `^[a-z][a-z0-9_-]{0,63}$`
pub fn validate_name(name: &str) -> Result<(), CommandError> {
    let mut chars = name.chars();
    let valid = name.len() <= MAX_NAME_LEN
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(CommandError::bad_request(format!(
            "Script name must match ^[a-z][a-z0-9_-]{{0,{}}}$",
            MAX_NAME_LEN - 1
        )))
    }
}

pub fn validate_description(description: &str) -> Result<(), CommandError> {
    let chars = description.chars().count();
    if chars == 0 || chars > MAX_DESCRIPTION_CHARS {
        return Err(CommandError::bad_request(format!(
            "Script description must be 1 to {MAX_DESCRIPTION_CHARS} characters"
        )));
    }
    Ok(())
}

pub fn validate_body(body: &str) -> Result<(), CommandError> {
    if body.is_empty() || body.len() > MAX_BODY_BYTES {
        return Err(CommandError::bad_request(format!(
            "Script body must be 1 to {MAX_BODY_BYTES} bytes"
        )));
    }
    Ok(())
}

/// An input schema is a JSON object with `"type": "object"`.
pub fn validate_input_schema(schema: &Value) -> Result<(), CommandError> {
    let is_object_schema = schema
        .as_object()
        .and_then(|object| object.get("type"))
        .and_then(Value::as_str)
        == Some("object");
    if is_object_schema {
        Ok(())
    } else {
        Err(CommandError::bad_request(
            "Script input_schema must be a JSON object with \"type\": \"object\"",
        ))
    }
}
